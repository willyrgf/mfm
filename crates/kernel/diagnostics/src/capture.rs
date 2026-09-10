//! Bounded source traversal; extraction remains with the concrete source owner.

use std::error::Error;
use std::io::{self, Write};

use super::*;

/// Missing field awaiting attribution to its response or concrete captured source.
pub type CaptureOmission = (OmittedField, OmissionReason, Option<u64>);

impl DiagnosticEvidence {
    /// Captures a response and an exposed source prefix with owner-reviewed extraction.
    ///
    /// `capture` is called once for each candidate source, at most 32 times, and must not
    /// format arbitrary source text or walk deeper sources. Traversal follows `source()`
    /// links and stops at the layer or byte bound even for a cyclic chain. Locations are
    /// assigned here only after retaining the corresponding source. `end` accounts for
    /// evidence hidden by the upstream API; truncation overrides it with `BoundReached`.
    pub fn capture<'a>(
        response: Option<(ResponseContext, Vec<CaptureOmission>)>,
        mut source: Option<&'a (dyn Error + 'static)>,
        end: ChainEnd,
        mut capture: impl FnMut(&'a (dyn Error + 'static)) -> (SourceLayer, Vec<CaptureOmission>),
    ) -> Self {
        let (response, response_omissions) = match response {
            Some((response, omissions)) => (Some(response), omissions),
            None => (None, Vec::new()),
        };
        // BoundReached is the longest chain-end encoding; false is the longest boolean.
        // Reserve these markers before adding variable data.
        let mut evidence = Self {
            response,
            sources: SourceChain {
                layers: Vec::new(),
                end: ChainEnd::BoundReached,
            },
            omissions: Vec::new(),
            omissions_truncated: false,
        };
        let mut omissions = Vec::new();
        let mut omissions_truncated = false;
        retain_omissions(
            &mut omissions,
            &mut omissions_truncated,
            EvidenceLocation::Response,
            response_omissions,
        );
        while let Some(current) = source {
            if evidence.sources.layers.len() == 32 {
                break;
            }
            let index = evidence.sources.layers.len() as u8;
            let (layer, fields) = capture(current);
            evidence.sources.layers.push(layer);
            if !evidence.fits() {
                evidence.sources.layers.pop();
                break;
            }
            retain_omissions(
                &mut omissions,
                &mut omissions_truncated,
                EvidenceLocation::SourceLayer { index },
                fields,
            );
            source = current.source();
        }
        // Preserve response and source prefix before filling the remaining omission budget.
        for omission in omissions {
            evidence.omissions.push(omission);
            if !evidence.fits() {
                evidence.omissions.pop();
                omissions_truncated = true;
                break;
            }
        }
        if source.is_none() {
            evidence.sources.end = end;
        }
        evidence.omissions_truncated = omissions_truncated;
        evidence
    }

    fn fits(&self) -> bool {
        // This closed representation has no floats, non-string map keys or custom serializers.
        // Its only serialization failure is the counting writer's explicit byte-bound refusal.
        // All strings are checked ASCII: sorting object keys changes no encoded byte count.
        serde_json::to_writer(ByteBudget(MAX_DIAGNOSTIC_BYTES), self).is_ok()
    }
}

fn retain_omissions(
    omissions: &mut Vec<Omission>,
    truncated: &mut bool,
    at: EvidenceLocation,
    fields: Vec<CaptureOmission>,
) {
    for (field, reason, observed_bytes) in fields.into_iter().take(33) {
        if omissions.len() == 32 {
            *truncated = true;
            break;
        }
        omissions.push(Omission::new(at.clone(), field, reason, observed_bytes));
    }
}

struct ByteBudget(usize);
impl Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SourceLayer {
    /// One opaque source, with no invented facts.
    pub fn opaque() -> Self {
        Self {
            kind: SourceKind::Opaque,
            facts: Vec::new(),
            facts_truncated: false,
        }
    }
    /// One concrete transport error.
    pub fn transport(kind: TransportFailureKind) -> Self {
        Self {
            kind: SourceKind::Transport,
            facts: vec![SourceFact::Transport { kind }],
            facts_truncated: false,
        }
    }
    /// One OS error retaining kind and any exposed numeric code in the same layer.
    pub fn os(kind: OsFailureKind, code: Option<i32>) -> Self {
        let mut facts = vec![SourceFact::Os { kind }];
        if let Some(code) = code {
            facts.push(SourceFact::OsCode { code });
        }
        Self {
            kind: SourceKind::Os,
            facts,
            facts_truncated: false,
        }
    }
    /// One parser error with its exposed category and location.
    pub fn parse(category: ParseCategory, location: ParseLocation) -> Self {
        Self {
            kind: SourceKind::Parse,
            facts: vec![SourceFact::Parse { category, location }],
            facts_truncated: false,
        }
    }
}
