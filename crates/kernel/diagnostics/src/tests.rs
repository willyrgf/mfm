use super::*;
use mfm_values::{canonicalize_mfm_value, MfmValue as _, PersistedSchema as _};

#[test]
fn response_and_ancestry_survive_canonical_round_trip() {
    let layer = SourceLayer::new(
        SourceKind::Os,
        vec![
            SourceFact::OsCode { code: 104 },
            SourceFact::Os {
                kind: OsFailureKind::ConnectionReset,
            },
        ],
        false,
    )
    .unwrap();
    assert!(matches!(layer.facts()[0], SourceFact::Os { .. }));
    let evidence = DiagnosticEvidence::new(
        Some(ResponseContext::new(
            HttpStatusCode::new(200).unwrap(),
            Some(-32000),
        )),
        SourceChain::new(
            vec![
                SourceLayer::new(SourceKind::Opaque, vec![], false).unwrap(),
                layer,
            ],
            ChainEnd::Complete,
        )
        .unwrap(),
        vec![Omission::new(
            EvidenceLocation::Response,
            OmittedField::Message,
            OmissionReason::Withheld,
            Some(17),
        )],
        false,
    )
    .unwrap();
    let (bytes, _) = canonicalize_mfm_value(&evidence).unwrap();
    let decoded: DiagnosticEvidence = serde_json::from_slice(bytes.as_bytes()).unwrap();
    assert_eq!(decoded, evidence);
    assert_eq!(
        serde_json::to_vec(&evidence).unwrap().len(),
        bytes.as_bytes().len()
    );
    assert_eq!(
        DiagnosticEvidence::schema_descriptor()
            .unwrap()
            .identity()
            .canonical_json_shape()
            .unwrap(),
        &DiagnosticEvidence::schema_shape().unwrap()
    );
}

#[test]
fn rejects_invalid_facts_locations_and_unreviewed_text() {
    assert_eq!(
        SourceLayer::new(
            SourceKind::Opaque,
            vec![SourceFact::OsCode { code: 1 }],
            false
        ),
        Err(EvidenceError::InvalidFacts)
    );
    assert_eq!(
        SourceLayer::new(
            SourceKind::Os,
            vec![
                SourceFact::OsCode { code: 1 },
                SourceFact::OsCode { code: 2 }
            ],
            false
        ),
        Err(EvidenceError::InvalidFacts)
    );
    for at in [
        EvidenceLocation::Response,
        EvidenceLocation::SourceLayer { index: 0 },
    ] {
        assert_eq!(
            DiagnosticEvidence::new(
                None,
                SourceChain::new(vec![], ChainEnd::Complete).unwrap(),
                vec![Omission::new(
                    at,
                    OmittedField::Message,
                    OmissionReason::Withheld,
                    None
                )],
                false
            ),
            Err(EvidenceError::InvalidLocation)
        );
    }
    assert!(serde_json::from_str::<HttpStatusCode>("99").is_err());
    assert!(serde_json::from_str::<SqlState>(r#""secret-canary""#).is_err());
    assert!(serde_json::from_str::<SourceLayer>(
        r#"{"kind":"opaque","facts":[],"facts_truncated":false,"message":"secret-canary"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<SourceFact>(r#"{"Message":"secret-canary"}"#).is_err());
}

#[test]
fn enforces_independent_count_and_total_byte_bounds() {
    let opaque = SourceLayer::new(SourceKind::Opaque, vec![], false).unwrap();
    assert_eq!(
        SourceChain::new(vec![opaque; 33], ChainEnd::BoundReached),
        Err(EvidenceError::BoundExceeded)
    );
    assert_eq!(
        SourceLayer::new(
            SourceKind::Os,
            vec![SourceFact::OsCode { code: 1 }; 9],
            true
        ),
        Err(EvidenceError::BoundExceeded)
    );
    let omission = Omission::new(
        EvidenceLocation::Response,
        OmittedField::Body,
        OmissionReason::Withheld,
        None,
    );
    assert_eq!(
        DiagnosticEvidence::new(
            None,
            SourceChain::new(vec![], ChainEnd::Complete).unwrap(),
            vec![omission; 33],
            true
        ),
        Err(EvidenceError::BoundExceeded)
    );
    let layer = SourceLayer::new(
        SourceKind::Parse,
        vec![
            SourceFact::Parse {
                category: ParseCategory::Syntax,
                location: ParseLocation::LineColumn {
                    line: u64::MAX,
                    column: u64::MAX,
                },
            },
            SourceFact::Size {
                limit: u64::MAX,
                observed: ObservedSize::AtLeast { value: u64::MAX },
            },
        ],
        true,
    )
    .unwrap();
    assert_eq!(
        DiagnosticEvidence::new(
            None,
            SourceChain::new(vec![layer; 32], ChainEnd::BoundReached).unwrap(),
            vec![],
            true
        ),
        Err(EvidenceError::BoundExceeded)
    );
}

#[test]
fn capture_follows_real_sources_and_bounds_cycles_and_omissions() {
    #[derive(Debug)]
    struct Outer(std::io::Error);
    impl std::fmt::Display for Outer {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("capture must not format sources")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    let outer = Outer(std::io::Error::from_raw_os_error(104));
    let evidence = DiagnosticEvidence::capture(None, Some(&outer), ChainEnd::Complete, |source| {
        let (kind, facts) = if let Some(error) = source.downcast_ref::<std::io::Error>() {
            (
                SourceKind::Os,
                vec![
                    SourceFact::Os {
                        kind: OsFailureKind::ConnectionReset,
                    },
                    SourceFact::OsCode {
                        code: error.raw_os_error().unwrap(),
                    },
                ],
            )
        } else {
            (SourceKind::Opaque, vec![])
        };
        (
            SourceLayer::new(kind, facts, false).unwrap(),
            vec![(OmittedField::SourceDetail, OmissionReason::Withheld, None)],
        )
    });
    assert_eq!(evidence.sources().layers().len(), 2);
    assert_eq!(evidence.sources().layers()[0].kind(), SourceKind::Opaque);
    assert_eq!(evidence.sources().layers()[1].facts().len(), 2);
    assert_eq!(evidence.sources().end(), ChainEnd::Complete);
    let (bytes, _) = canonicalize_mfm_value(&evidence).unwrap();
    assert_eq!(
        serde_json::from_slice::<DiagnosticEvidence>(bytes.as_bytes()).unwrap(),
        evidence
    );

    #[derive(Debug)]
    struct Cycle;
    impl std::fmt::Display for Cycle {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("no formatting")
        }
    }
    impl std::error::Error for Cycle {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(self)
        }
    }
    let mut calls = 0;
    let bounded = DiagnosticEvidence::capture(None, Some(&Cycle), ChainEnd::Complete, |_| {
        calls += 1;
        (
            SourceLayer::opaque(),
            vec![
                (OmittedField::Message, OmissionReason::Withheld, None),
                (
                    OmittedField::SourceDetail,
                    OmissionReason::Unavailable,
                    None,
                ),
            ],
        )
    });
    assert_eq!(calls, 32);
    assert_eq!(bounded.sources().layers().len(), 32);
    assert_eq!(bounded.sources().end(), ChainEnd::BoundReached);
    assert_eq!(bounded.omissions().len(), 32);
    assert!(bounded.omissions_truncated());
    canonicalize_mfm_value(&bounded).unwrap();
}

#[test]
fn deserialization_rechecks_counts_locations_and_fact_order() {
    let valid = serde_json::json!({
        "response": { "status": 200, "rpc_code": null },
        "sources": { "layers": [{ "kind": "os", "facts": [
            { "Os": { "kind": "connection_reset" } }, { "OsCode": { "code": 104 } }
        ], "facts_truncated": false }], "end": "complete" },
        "omissions": [{ "at": "Response", "field": "body", "reason": "withheld", "observed_bytes": null }],
        "omissions_truncated": false,
    });
    serde_json::from_value::<DiagnosticEvidence>(valid.clone()).unwrap();
    let mut cases = Vec::new();
    let mut invalid = valid.clone();
    invalid["sources"]["layers"] =
        serde_json::json!(vec![valid["sources"]["layers"][0].clone(); 33]);
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["sources"]["layers"][0]["facts"]
        .as_array_mut()
        .unwrap()
        .reverse();
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["sources"]["layers"][0]["kind"] = serde_json::json!("opaque");
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["omissions"] = serde_json::json!(vec![valid["omissions"][0].clone(); 33]);
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["omissions"][0]["at"] = serde_json::json!({ "SourceLayer": { "index": 1 } });
    cases.push(invalid);
    let mut invalid = valid;
    invalid["response"] = serde_json::Value::Null;
    cases.push(invalid);
    for invalid in cases {
        assert!(serde_json::from_value::<DiagnosticEvidence>(invalid).is_err());
    }
}
