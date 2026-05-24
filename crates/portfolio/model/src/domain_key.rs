use mfm_canonical::{CanonicalBytes, CanonicalError, PlainCanonicalJsonBytes};
use mfm_program_derive::MfmValue;
use mfm_values::{MfmValue, SchemaDescriptor};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable domain key contract for portfolio fanout/fanin instances.
pub trait StableDomainKey: MfmValue {
    /// Returns the descriptor for this domain key type.
    fn domain_key_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        Self::schema_descriptor()
    }

    /// Returns canonical bytes for this domain key instance.
    fn canonical_domain_bytes(&self) -> Result<CanonicalBytes, CanonicalError>;
}

macro_rules! stable_domain_key_type {
    ($ty:ident, $field:ident, $semantic:literal, $schema:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
        #[mfm(namespace = "mfm.portfolio", name = $semantic, schema = $schema)]
        pub struct $ty {
            #[doc = "Stable author key segment for this domain instance."]
            $field: String,
        }

        impl $ty {
            /// Creates a stable domain key after validating author-key grammar.
            pub fn new(value: impl Into<String>) -> Result<Self, StableDomainKeyError> {
                let value = value.into();
                validate_author_key(&value)?;
                Ok(Self { $field: value })
            }

            /// Returns the validated key string.
            pub fn as_str(&self) -> &str {
                &self.$field
            }
        }

        impl StableDomainKey for $ty {
            fn canonical_domain_bytes(&self) -> Result<CanonicalBytes, CanonicalError> {
                let json = serde_json::to_string(self)
                    .expect("serializing stable domain keys cannot fail");
                let canonical = PlainCanonicalJsonBytes::from_json_str(&json)?;
                Ok(CanonicalBytes::new(canonical.to_vec()))
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(Deserialize)]
                struct Wire {
                    $field: String,
                }

                let wire = Wire::deserialize(deserializer)?;
                Self::new(wire.$field).map_err(serde::de::Error::custom)
            }
        }
    };
}

stable_domain_key_type!(
    SourceDomainKey,
    source_key,
    "source-domain-key",
    "mfm.portfolio.domain_key.source",
    "Stable domain key for a portfolio source preparation instance."
);
stable_domain_key_type!(
    SubjectDomainKey,
    subject_key,
    "subject-domain-key",
    "mfm.portfolio.domain_key.subject",
    "Stable domain key for a portfolio subject instance."
);
stable_domain_key_type!(
    ViewDomainKey,
    view_key,
    "view-domain-key",
    "mfm.portfolio.domain_key.view",
    "Stable domain key for a portfolio execution view instance."
);
stable_domain_key_type!(
    ValuationDomainKey,
    valuation_key,
    "valuation-domain-key",
    "mfm.portfolio.domain_key.valuation",
    "Stable domain key for a portfolio valuation instance."
);
stable_domain_key_type!(
    ObservationBatchDomainKey,
    observation_batch_key,
    "observation-batch-domain-key",
    "mfm.portfolio.domain_key.observation_batch",
    "Stable domain key for a portfolio observation batch instance."
);
stable_domain_key_type!(
    ReportDomainKey,
    report_key,
    "report-domain-key",
    "mfm.portfolio.domain_key.report",
    "Stable domain key for a portfolio report instance."
);

impl mfm_program::StableDomainKey for SourceDomainKey {}
impl mfm_program::StableDomainKey for SubjectDomainKey {}
impl mfm_program::StableDomainKey for ViewDomainKey {}
impl mfm_program::StableDomainKey for ValuationDomainKey {}
impl mfm_program::StableDomainKey for ObservationBatchDomainKey {}
impl mfm_program::StableDomainKey for ReportDomainKey {}

/// Errors returned when validating stable domain keys.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StableDomainKeyError {
    /// The key was empty.
    #[error("domain key must be non-empty")]
    Empty,
    /// The full key exceeded the supported length.
    #[error("domain key `{value}` exceeded 256 bytes")]
    TooLong {
        /// Rejected key.
        value: String,
    },
    /// A segment was empty.
    #[error("domain key `{value}` contained an empty segment")]
    EmptySegment {
        /// Rejected key.
        value: String,
    },
    /// A segment exceeded the supported length.
    #[error("domain key segment `{segment}` in `{value}` exceeded 64 bytes")]
    SegmentTooLong {
        /// Rejected key.
        value: String,
        /// Rejected segment.
        segment: String,
    },
    /// The key used a reserved prefix.
    #[error("domain key `{value}` used reserved prefix `{prefix}`")]
    ReservedPrefix {
        /// Rejected key.
        value: String,
        /// Reserved prefix.
        prefix: &'static str,
    },
    /// The key contained a character outside the author-key grammar.
    #[error("domain key `{value}` contained invalid character `{ch}`")]
    InvalidCharacter {
        /// Rejected key.
        value: String,
        /// Invalid character.
        ch: char,
    },
    /// One segment did not start with `[a-z0-9]`.
    #[error("domain key segment `{segment}` in `{value}` must start with [a-z0-9]")]
    InvalidSegmentStart {
        /// Rejected key.
        value: String,
        /// Rejected segment.
        segment: String,
    },
}

/// Validates the RFC author-key grammar used by stable domain keys.
pub fn validate_author_key(value: &str) -> Result<(), StableDomainKeyError> {
    if value.is_empty() {
        return Err(StableDomainKeyError::Empty);
    }
    if value.len() > 256 {
        return Err(StableDomainKeyError::TooLong {
            value: value.to_string(),
        });
    }
    for prefix in ["mfm.", "sys.", "_"] {
        if value.starts_with(prefix) {
            return Err(StableDomainKeyError::ReservedPrefix {
                value: value.to_string(),
                prefix,
            });
        }
    }
    for segment in value.split('/') {
        if segment.is_empty() {
            return Err(StableDomainKeyError::EmptySegment {
                value: value.to_string(),
            });
        }
        if segment.len() > 64 {
            return Err(StableDomainKeyError::SegmentTooLong {
                value: value.to_string(),
                segment: segment.to_string(),
            });
        }
        let mut chars = segment.chars();
        let first = chars
            .next()
            .expect("segment is known non-empty after validation");
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return Err(StableDomainKeyError::InvalidSegmentStart {
                value: value.to_string(),
                segment: segment.to_string(),
            });
        }
        for ch in chars {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.') {
                continue;
            }
            return Err(StableDomainKeyError::InvalidCharacter {
                value: value.to_string(),
                ch,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{StableDomainKey, StableDomainKeyError, SubjectDomainKey};
    use serde_json::json;

    #[test]
    fn stable_domain_keys_validate_author_key_grammar() {
        assert_eq!(
            SubjectDomainKey::new("wallet_main/eth")
                .expect("valid")
                .as_str(),
            "wallet_main/eth"
        );
        assert!(matches!(
            SubjectDomainKey::new("mfm.internal"),
            Err(StableDomainKeyError::ReservedPrefix { .. })
        ));
        assert!(matches!(
            SubjectDomainKey::new("Wallet"),
            Err(StableDomainKeyError::InvalidSegmentStart { .. })
        ));
    }

    #[test]
    fn stable_domain_key_bytes_are_canonical_json() {
        let key = SubjectDomainKey::new("wallet_main").expect("valid key");
        let bytes = key
            .canonical_domain_bytes()
            .expect("canonical domain bytes");
        assert_eq!(bytes.as_bytes(), br#"{"subject_key":"wallet_main"}"#);
    }

    #[test]
    fn stable_domain_keys_validate_deserialized_values() {
        let key: SubjectDomainKey =
            serde_json::from_value(json!({"subject_key": "wallet_main"})).expect("valid key");
        assert_eq!(key.as_str(), "wallet_main");

        let err = serde_json::from_value::<SubjectDomainKey>(json!({"subject_key": "sys.hidden"}))
            .expect_err("reserved prefixes must reject during deserialize");
        assert!(
            err.to_string().contains("reserved prefix"),
            "unexpected error: {err}"
        );
    }
}
