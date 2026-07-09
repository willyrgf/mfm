//! Bitcoin address balance snapshot fact (`bitcoin.address_balance_snapshot`).

use mfm_facts::{CoverageStatus, FactAudience, FactVisibility, HoldingSourceStatus};
use mfm_program_derive::{MfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

use crate::BtcStateError;

/// Returns Platform visibility for Bitcoin address balance snapshot facts.
pub fn address_balance_fact_visibility() -> FactVisibility {
    FactVisibility::indexed_default(FactAudience::Platform)
}

/// Subject identity for a Bitcoin address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_subject",
    version = "1",
    schema = "mfm.bitcoin.fact.address_balance.subject"
)]
pub struct BtcAddressBalanceSubject {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    address: String,
}

impl BtcAddressBalanceSubject {
    /// Creates subject material for an address balance snapshot.
    pub fn new(
        network: impl Into<String>,
        bitcoin_network: impl Into<String>,
        semantic_source_identity: impl Into<String>,
        address: impl Into<String>,
    ) -> Result<Self, BtcStateError> {
        let network = network.into();
        let bitcoin_network = bitcoin_network.into();
        let semantic_source_identity = semantic_source_identity.into();
        let address = address.into();
        if network.trim().is_empty()
            || bitcoin_network.trim().is_empty()
            || semantic_source_identity.trim().is_empty()
            || address.trim().is_empty()
        {
            return Err(BtcStateError::InvalidInput {
                reason: "address balance subject fields must be non-empty".to_owned(),
            });
        }
        if semantic_source_identity.contains("://")
            || semantic_source_identity.contains('@')
            || semantic_source_identity.contains('/')
        {
            return Err(BtcStateError::InvalidInput {
                reason: "semantic_source_identity must not encode runtime routes".to_owned(),
            });
        }
        Ok(Self {
            network,
            bitcoin_network,
            semantic_source_identity,
            address,
        })
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the Bitcoin Core network tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }

    /// Returns the non-secret semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the observed Bitcoin address.
    pub fn address(&self) -> &str {
        &self.address
    }
}

/// Observed result for a Bitcoin address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_response",
    version = "1",
    schema = "mfm.bitcoin.fact.address_balance.response"
)]
pub struct BtcAddressBalanceResponse {
    anchor_height: u64,
    anchor_hash: String,
    balance_sats: u64,
    coverage: String,
    source_status: String,
}

impl BtcAddressBalanceResponse {
    /// Creates response material, failing closed on missing hash or inadmissible coverage/status.
    pub fn new(
        anchor_height: u64,
        anchor_hash: impl Into<String>,
        balance_sats: u64,
        coverage: CoverageStatus,
        source_status: HoldingSourceStatus,
    ) -> Result<Self, BtcStateError> {
        let anchor_hash = anchor_hash.into();
        if anchor_hash.trim().is_empty() {
            return Err(BtcStateError::InvalidInput {
                reason: "address balance anchor_hash is required".to_owned(),
            });
        }
        if !coverage.is_admissible_for_write() {
            return Err(BtcStateError::InvalidInput {
                reason: format!(
                    "coverage {} is not admissible for Platform write",
                    coverage.as_str()
                ),
            });
        }
        if !source_status.is_admissible_for_write() {
            return Err(BtcStateError::InvalidInput {
                reason: format!(
                    "source_status {} is not admissible for Platform write",
                    source_status.as_str()
                ),
            });
        }
        Ok(Self {
            anchor_height,
            anchor_hash,
            balance_sats,
            coverage: coverage.as_str().to_owned(),
            source_status: source_status.as_str().to_owned(),
        })
    }

    /// Returns the mandatory anchor height.
    pub const fn anchor_height(&self) -> u64 {
        self.anchor_height
    }

    /// Returns the mandatory anchor block hash.
    pub fn anchor_hash(&self) -> &str {
        &self.anchor_hash
    }

    /// Returns the total balance in satoshis.
    pub const fn balance_sats(&self) -> u64 {
        self.balance_sats
    }

    /// Returns the coverage tag.
    pub fn coverage(&self) -> &str {
        &self.coverage
    }

    /// Returns the holding source status tag.
    pub fn source_status(&self) -> &str {
        &self.source_status
    }

    /// Parses coverage as a closed enum.
    pub fn coverage_status(&self) -> Result<CoverageStatus, BtcStateError> {
        self.coverage
            .parse()
            .map_err(|_| BtcStateError::InvalidInput {
                reason: format!("unknown coverage status {:?}", self.coverage),
            })
    }

    /// Parses holding source status as a closed enum.
    pub fn holding_source_status(&self) -> Result<HoldingSourceStatus, BtcStateError> {
        self.source_status
            .parse()
            .map_err(|_| BtcStateError::InvalidInput {
                reason: format!("unknown holding source status {:?}", self.source_status),
            })
    }
}

/// Platform fact for a Bitcoin address balance snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_snapshot_fact",
    version = "1",
    schema = "mfm.bitcoin.fact.address_balance_snapshot"
)]
#[mfm_fact(kind = "bitcoin.address_balance_snapshot")]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.bitcoin_network",
    source = "subject",
    path = "bitcoin_network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.semantic_source_identity",
    source = "subject",
    path = "semantic_source_identity",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.address",
    source = "subject",
    path = "address",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.anchor_height",
    source = "result",
    path = "anchor_height",
    value_type = "unsigned_integer",
    operators(
        equal,
        greater_than,
        greater_than_or_equal,
        less_than,
        less_than_or_equal
    ),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(field(
    id = "result.anchor_hash",
    source = "result",
    path = "anchor_hash",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.balance_sats",
    source = "result",
    path = "balance_sats",
    value_type = "unsigned_integer",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.coverage",
    source = "result",
    path = "coverage",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.source_status",
    source = "result",
    path = "source_status",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "metadata.store_commit_order",
    source = "metadata",
    metadata = "store_commit_order",
    value_type = "unsigned_integer",
    operators(
        equal,
        greater_than,
        greater_than_or_equal,
        less_than,
        less_than_or_equal
    ),
    exposure = "query_only",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.anchor_height.desc",
    term(
        field = "result.anchor_height",
        direction = "descending",
        nulls = "last"
    )
))]
#[mfm_fact(ordering(
    name = "metadata.store_commit_order.desc",
    term(
        field = "metadata.store_commit_order",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct BtcAddressBalanceSnapshotFact {
    subject: BtcAddressBalanceSubject,
    response: BtcAddressBalanceResponse,
}

impl BtcAddressBalanceSnapshotFact {
    /// Creates a balance snapshot fact from validated subject and response material.
    pub const fn new(
        subject: BtcAddressBalanceSubject,
        response: BtcAddressBalanceResponse,
    ) -> Self {
        Self { subject, response }
    }

    /// Creates a fact, re-validating response admission rules for fail-closed writers.
    pub fn try_new(
        subject: BtcAddressBalanceSubject,
        anchor_height: u64,
        anchor_hash: impl Into<String>,
        balance_sats: u64,
        coverage: CoverageStatus,
        source_status: HoldingSourceStatus,
    ) -> Result<Self, BtcStateError> {
        let response = BtcAddressBalanceResponse::new(
            anchor_height,
            anchor_hash,
            balance_sats,
            coverage,
            source_status,
        )?;
        Ok(Self::new(subject, response))
    }

    /// Returns the subject material.
    pub const fn subject(&self) -> &BtcAddressBalanceSubject {
        &self.subject
    }

    /// Returns the response material.
    pub const fn response(&self) -> &BtcAddressBalanceResponse {
        &self.response
    }
}

/// Source-near holding material normalized from a Bitcoin address balance fact.
///
/// Contains no portfolio wallet_id/symbol_id; report join happens later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedBtcAddressHolding {
    /// Semantic network id.
    pub network: String,
    /// Bitcoin Core network tag.
    pub bitcoin_network: String,
    /// Observed address.
    pub address: String,
    /// Total balance in satoshis.
    pub balance_sats: u64,
    /// Anchor height.
    pub anchor_height: u64,
    /// Anchor block hash.
    pub anchor_hash: String,
    /// Coverage claim.
    pub coverage: CoverageStatus,
    /// Holding source status.
    pub source_status: HoldingSourceStatus,
}

/// Pure normalize: fact → source-near holding fields. Fail closed on missing hash or
/// inadmissible coverage/status tags.
pub fn normalize_btc_address_balance_fact(
    fact: &BtcAddressBalanceSnapshotFact,
) -> Result<NormalizedBtcAddressHolding, BtcStateError> {
    normalize_btc_address_balance(fact.subject(), fact.response())
}

/// Pure normalize from subject + response material.
pub fn normalize_btc_address_balance(
    subject: &BtcAddressBalanceSubject,
    response: &BtcAddressBalanceResponse,
) -> Result<NormalizedBtcAddressHolding, BtcStateError> {
    if response.anchor_hash().trim().is_empty() {
        return Err(BtcStateError::InvalidInput {
            reason: "address balance anchor_hash is required".to_owned(),
        });
    }
    let coverage = response.coverage_status()?;
    let source_status = response.holding_source_status()?;
    if !coverage.is_acceptable_for_report() {
        return Err(BtcStateError::InvalidInput {
            reason: format!(
                "coverage {} is not acceptable for report selection",
                coverage.as_str()
            ),
        });
    }
    if !source_status.is_acceptable_for_report() {
        return Err(BtcStateError::InvalidInput {
            reason: format!(
                "source_status {} is not acceptable for report selection",
                source_status.as_str()
            ),
        });
    }
    Ok(NormalizedBtcAddressHolding {
        network: subject.network().to_owned(),
        bitcoin_network: subject.bitcoin_network().to_owned(),
        address: subject.address().to_owned(),
        balance_sats: response.balance_sats(),
        anchor_height: response.anchor_height(),
        anchor_hash: response.anchor_hash().to_owned(),
        coverage,
        source_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_facts::{FactFieldExposure, FactFieldExtraction, FactFieldValueType};
    use mfm_program::MfmFactType;

    fn valid_subject() -> BtcAddressBalanceSubject {
        BtcAddressBalanceSubject::new(
            "bitcoin-mainnet",
            "main",
            "public-bitcoin-core",
            "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
        )
        .expect("subject")
    }

    fn response_from_json(json: serde_json::Value) -> BtcAddressBalanceResponse {
        serde_json::from_value(json).expect("response json")
    }

    #[test]
    fn descriptor_declares_kind_anchors_coverage_and_store_commit_order() {
        let descriptor = BtcAddressBalanceSnapshotFact::descriptor().expect("descriptor");
        assert_eq!(
            descriptor.fact_kind().as_str(),
            "bitcoin.address_balance_snapshot"
        );
        for field_id in [
            "subject.network",
            "subject.bitcoin_network",
            "subject.semantic_source_identity",
            "subject.address",
            "result.anchor_height",
            "result.anchor_hash",
            "result.balance_sats",
            "result.coverage",
            "result.source_status",
        ] {
            assert!(
                descriptor.fields().iter().any(|field| {
                    field.field_id().as_str() == field_id
                        && field.exposure() == FactFieldExposure::Returnable
                }),
                "missing returnable field {field_id}"
            );
        }
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "result.anchor_height"
                && field.value_type() == FactFieldValueType::UnsignedInteger
                && field.sortable()
        }));
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "metadata.store_commit_order"
                && matches!(field.extraction(), FactFieldExtraction::Metadata(_))
                && field.sortable()
        }));
        let ordering_names: Vec<_> = descriptor
            .orderings()
            .iter()
            .map(|ordering| ordering.name().as_str())
            .collect();
        assert!(ordering_names.contains(&"result.anchor_height.desc"));
        assert!(ordering_names.contains(&"metadata.store_commit_order.desc"));
    }

    #[test]
    fn write_admission_requires_hash_and_admissible_coverage_status() {
        assert!(BtcAddressBalanceResponse::new(
            100,
            "",
            1,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .is_err());
        assert!(BtcAddressBalanceResponse::new(
            100,
            "00".repeat(32),
            1,
            CoverageStatus::Truncated,
            HoldingSourceStatus::Ok,
        )
        .is_err());
        assert!(BtcAddressBalanceResponse::new(
            100,
            "00".repeat(32),
            1,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Failed,
        )
        .is_err());
        let response = BtcAddressBalanceResponse::new(
            100,
            "00".repeat(32),
            42,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("admissible");
        let fact = BtcAddressBalanceSnapshotFact::new(valid_subject(), response);
        let normalized = normalize_btc_address_balance_fact(&fact).expect("normalize");
        assert_eq!(normalized.balance_sats, 42);
        assert_eq!(normalized.anchor_height, 100);
        assert_eq!(normalized.coverage, CoverageStatus::ConfiguredOnly);
        assert_eq!(normalized.source_status, HoldingSourceStatus::Ok);
    }

    #[test]
    fn normalize_rejects_inadmissible_tags_even_if_response_fields_present() {
        // Tampered payloads that bypassed constructor checks (e.g. legacy decode).
        let missing_hash = response_from_json(serde_json::json!({
            "anchor_height": 99,
            "anchor_hash": "",
            "balance_sats": 7,
            "coverage": "configured_only",
            "source_status": "ok",
        }));
        assert!(normalize_btc_address_balance(&valid_subject(), &missing_hash).is_err());

        let incomplete = response_from_json(serde_json::json!({
            "anchor_height": 99,
            "anchor_hash": "aa".repeat(32),
            "balance_sats": 7,
            "coverage": "incomplete",
            "source_status": "ok",
        }));
        assert!(normalize_btc_address_balance(&valid_subject(), &incomplete).is_err());
    }

    #[test]
    fn response_json_round_trip_has_no_floats_or_secrets() {
        let response = BtcAddressBalanceResponse::new(
            850_000,
            "0f".repeat(32),
            100_000,
            CoverageStatus::CompleteAtAnchor,
            HoldingSourceStatus::Ok,
        )
        .expect("response");
        let fact = BtcAddressBalanceSnapshotFact::new(valid_subject(), response);
        let value = serde_json::to_value(&fact).expect("json");
        let text = serde_json::to_string(&value).expect("text");
        for forbidden in ["rpc_url", "password", "http://", "wallet_id", "symbol_id"] {
            assert!(
                !text.contains(forbidden),
                "leaked forbidden token {forbidden}: {text}"
            );
        }
        // Hashed structures must not contain JSON floats.
        fn assert_no_floats(value: &serde_json::Value) {
            match value {
                serde_json::Value::Number(n) => assert!(n.is_u64() || n.is_i64(), "{n}"),
                serde_json::Value::Array(items) => items.iter().for_each(assert_no_floats),
                serde_json::Value::Object(map) => map.values().for_each(assert_no_floats),
                _ => {}
            }
        }
        assert_no_floats(&value);
        let decoded: BtcAddressBalanceSnapshotFact =
            serde_json::from_value(value).expect("round-trip");
        assert_eq!(decoded, fact);
    }

    #[test]
    fn platform_candidate_query_plan_is_exact_full_set_not_limit_one() {
        use mfm_facts::{
            compile_fact_query_plan, FactAudience, FactCanonicalScalar, FactFieldId,
            FactOrderingName, FactQueryInput, FactQueryOperator, FactQueryPredicate,
            FactQueryScope, FactVisibilityScope, ScopeDecisionEvidence, StoreScopeRef,
        };
        use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

        let descriptor = BtcAddressBalanceSnapshotFact::descriptor().expect("descriptor");
        let subject = valid_subject();
        let input = FactQueryInput::new(
            StoreScopeRef::new("mfm.store.default").expect("store"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x31; 32]),
            )),
            vec![
                FactQueryPredicate::new(
                    FactFieldId::new("subject.network").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::string(subject.network()),
                ),
                FactQueryPredicate::new(
                    FactFieldId::new("subject.bitcoin_network").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::string(subject.bitcoin_network()),
                ),
                FactQueryPredicate::new(
                    FactFieldId::new("subject.semantic_source_identity").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::string(subject.semantic_source_identity()),
                ),
                FactQueryPredicate::new(
                    FactFieldId::new("subject.address").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::string(subject.address()),
                ),
            ],
            vec![
                FactFieldId::new("result.anchor_height").expect("field"),
                FactFieldId::new("result.anchor_hash").expect("field"),
                FactFieldId::new("result.balance_sats").expect("field"),
                FactFieldId::new("result.coverage").expect("field"),
                FactFieldId::new("result.source_status").expect("field"),
            ],
            FactOrderingName::new("result.anchor_height.desc").expect("ordering"),
            None, // full candidate set — never limit=1 as selection
        )
        .expect("query input");
        let plan = compile_fact_query_plan(&descriptor, input).expect("plan");
        assert_eq!(plan.query_scope().audience(), FactAudience::Platform);
        assert_eq!(plan.limit(), None);
        assert_eq!(plan.ordering().name().as_str(), "result.anchor_height.desc");
    }
}
