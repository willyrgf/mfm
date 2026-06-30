use mfm_ids::{CheckedStringError, StableAuthorKey};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

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
            pub fn new(value: impl Into<String>) -> Result<Self, CheckedStringError> {
                let value = value.into();
                let value = StableAuthorKey::new(value)?.into_string();
                Ok(Self { $field: value })
            }

            /// Returns the validated key string.
            pub fn as_str(&self) -> &str {
                &self.$field
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

#[cfg(test)]
mod tests {
    use super::SubjectDomainKey;
    use mfm_program::StableDomainKey;
    use serde_json::json;

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
    }
}
