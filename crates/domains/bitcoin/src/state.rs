//! Deterministic aggregate Bitcoin balance collection.
//!
//! One external-read state validates semantic demand, reduces one source-bound scan into an
//! ordered non-empty fact batch, and returns the minimal receipt needed for later hydration.

use std::str::FromStr;

use crate::capability::{
    BitcoinBalanceCollectionReadCapability, BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
use crate::model::{
    BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionResponse, BitcoinNetworkId,
    BitcoinNetworkTag, BitcoinSourceBinding, BitcoinSourceIdentity,
};
use bitcoin::{Amount, BlockHash};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_facts::{FactContentIdentityEvidence, MfmFactType};
use mfm_ids::{AdapterKind, AdapterVersion, DigestAlgorithm, StateKind, StateVersion};
use mfm_program::{
    AdapterBindingSpec, ExternalReadEvidenceSet, NoContext, NonEmpty, ReadState, StateError,
    StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmFactType as DeriveMfmFactType, MfmValue};
use mfm_values::ConfigError;
use serde::{Deserialize, Serialize};

const ADAPTER_VERSION: &str = "mfm.bitcoin.jsonrpc.adapter.v1";
const STATE_VERSION: &str = "mfm.bitcoin.state.collect_balances.v1";

/// Redaction-safe aggregate Bitcoin state error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Bitcoin balance collection material was invalid: {reason}")]
pub struct BitcoinBalanceCollectionError {
    reason: String,
}

impl BitcoinBalanceCollectionError {
    fn invalid(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl From<BitcoinBalanceCollectionError> for StateError {
    fn from(error: BitcoinBalanceCollectionError) -> Self {
        Self::Message(error.to_string())
    }
}

/// Certified semantic demand for one Bitcoin source collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.bitcoin.balance_collection.config",
    validate = "validate_bitcoin_balance_collection_config"
)]
pub struct BitcoinBalanceCollectionConfig {
    network_id: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    addresses: Vec<String>,
}

impl BitcoinBalanceCollectionConfig {
    /// Creates a checked aggregate collection config.
    pub fn new(
        network_id: impl Into<String>,
        bitcoin_network: impl Into<String>,
        semantic_source_identity: impl Into<String>,
        addresses: Vec<String>,
    ) -> Result<Self, ConfigError> {
        let config = Self {
            network_id: network_id.into(),
            bitcoin_network: bitcoin_network.into(),
            semantic_source_identity: semantic_source_identity.into(),
            addresses,
        };
        validate_bitcoin_balance_collection_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the required Bitcoin Core chain tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }

    /// Returns the semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns canonical addresses in strict UTF-8 order.
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// Reconstructs the exact checked capability request.
    pub fn request(
        &self,
    ) -> Result<BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionError> {
        BitcoinBalanceCollectionRequest::new(
            BitcoinSourceBinding::new(
                BitcoinNetworkId::new(&self.network_id)
                    .map_err(|_| invalid("semantic network id was invalid"))?,
                BitcoinNetworkTag::new(&self.bitcoin_network)
                    .map_err(|_| invalid("Bitcoin network tag was invalid"))?,
                BitcoinSourceIdentity::new(&self.semantic_source_identity)
                    .map_err(|_| invalid("semantic source identity was invalid"))?,
            ),
            self.addresses.clone(),
        )
        .map_err(|_| invalid("Bitcoin address demand was invalid"))
    }
}

/// Validates bounded, canonical, sorted, duplicate-free Bitcoin source demand.
fn validate_bitcoin_balance_collection_config(
    config: &BitcoinBalanceCollectionConfig,
) -> Result<(), String> {
    config
        .request()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Deterministic plan for one aggregate Bitcoin read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "balance_collection_plan",
    schema = "mfm.bitcoin.balance_collection.plan"
)]
pub struct BitcoinBalanceCollectionPlan {
    network_id: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    addresses: Vec<String>,
}

impl BitcoinBalanceCollectionPlan {
    /// Reconstructs the exact checked capability request.
    pub fn request(
        &self,
    ) -> Result<BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionError> {
        self.config().request()
    }

    fn config(&self) -> BitcoinBalanceCollectionConfig {
        BitcoinBalanceCollectionConfig {
            network_id: self.network_id.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            addresses: self.addresses.clone(),
        }
    }
}

/// Canonical primary evidence for one complete three-call Bitcoin read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "balance_collection_evidence",
    schema = "mfm.bitcoin.balance_collection.evidence"
)]
pub struct BitcoinBalanceCollectionEvidence {
    network_id: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    implementation_id: String,
    anchor_height: u64,
    anchor_hash: String,
    balances: Vec<(String, u64)>,
    final_canonical_hash: String,
}

impl BitcoinBalanceCollectionEvidence {
    /// Captures canonical primary evidence from a checked session response.
    pub fn from_response(response: &BitcoinBalanceCollectionResponse) -> Self {
        Self {
            network_id: response.binding().network_id().as_str().to_owned(),
            bitcoin_network: response.binding().bitcoin_network().as_str().to_owned(),
            semantic_source_identity: response
                .binding()
                .semantic_source_identity()
                .as_str()
                .to_owned(),
            implementation_id: response.implementation_id().to_owned(),
            anchor_height: response.anchor_height(),
            anchor_hash: response.anchor_hash().to_string(),
            balances: response
                .balances()
                .iter()
                .map(|balance| (balance.address().to_owned(), balance.balance_sats()))
                .collect(),
            final_canonical_hash: response.final_canonical_hash().to_string(),
        }
    }
}

/// The one aggregate fact-producing Bitcoin read state.
pub struct CollectBitcoinBalancesState {
    config: BitcoinBalanceCollectionConfig,
}

impl CollectBitcoinBalancesState {
    /// Returns the validated semantic demand.
    pub const fn config(&self) -> &BitcoinBalanceCollectionConfig {
        &self.config
    }
}

impl StateSpec for CollectBitcoinBalancesState {
    type Config = BitcoinBalanceCollectionConfig;
    type Context = NoContext;
    type Input = ();
    type Output = BitcoinBalanceCollectionReceipt;
    type Effect = mfm_capabilities::ReadExternal;
    type Caps = (BitcoinBalanceCollectionReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.bitcoin",
            "collect_balances",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.bitcoin.state:collect_balances"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new(STATE_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.bitcoin.collect_balances"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        Ok(vec![AdapterBindingSpec {
            adapter_kind: bitcoin_jsonrpc_adapter_kind()
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            adapter_version: bitcoin_jsonrpc_adapter_version()
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        }])
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for CollectBitcoinBalancesState {
    type Plan = BitcoinBalanceCollectionPlan;
    type Evidence = BitcoinBalanceCollectionEvidence;
    type Facts = NonEmpty<BitcoinBalanceSnapshotFact>;

    fn plan(
        &self,
        _input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        validate_bitcoin_balance_collection_config(&self.config).map_err(StateError::Message)?;
        Ok(BitcoinBalanceCollectionPlan {
            network_id: self.config.network_id.clone(),
            bitcoin_network: self.config.bitcoin_network.clone(),
            semantic_source_identity: self.config.semantic_source_identity.clone(),
            addresses: self.config.addresses.clone(),
        })
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<(Self::Output, Self::Facts)> {
        if !evidence.fact_query_evidence().is_empty() {
            return Err(StateError::Message(
                "Bitcoin collection carried unexpected fact-query evidence".to_owned(),
            ));
        }
        let plan = self.plan(input, context)?;
        reduce_bitcoin_balance_collection(&plan, evidence.primary_evidence())
            .map_err(StateError::from)
    }
}

/// Reduces exact primary evidence into an ordered fact batch and minimal receipt.
fn reduce_bitcoin_balance_collection(
    plan: &BitcoinBalanceCollectionPlan,
    evidence: &BitcoinBalanceCollectionEvidence,
) -> Result<
    (
        BitcoinBalanceCollectionReceipt,
        NonEmpty<BitcoinBalanceSnapshotFact>,
    ),
    BitcoinBalanceCollectionError,
> {
    let request = plan.request()?;
    if evidence.network_id != request.binding().network_id().as_str()
        || evidence.bitcoin_network != request.binding().bitcoin_network().as_str()
        || evidence.semantic_source_identity
            != request.binding().semantic_source_identity().as_str()
        || evidence.implementation_id != BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    {
        return Err(invalid(
            "Bitcoin evidence source binding did not match the plan",
        ));
    }
    let anchor_hash = BlockHash::from_str(&evidence.anchor_hash)
        .map_err(|_| invalid("Bitcoin scan anchor hash was invalid"))?;
    let final_hash = BlockHash::from_str(&evidence.final_canonical_hash)
        .map_err(|_| invalid("Bitcoin final canonical hash was invalid"))?;
    if anchor_hash != final_hash {
        return Err(invalid("Bitcoin scan anchor was no longer canonical"));
    }
    if evidence.balances.len() != request.addresses().len() {
        return Err(invalid("Bitcoin balance evidence coverage was not exact"));
    }

    let mut facts = Vec::with_capacity(request.addresses().len());
    for (expected, (observed_address, observed_balance)) in
        request.addresses().iter().zip(&evidence.balances)
    {
        if observed_address != expected.as_str() || *observed_balance > Amount::MAX_MONEY.to_sat() {
            return Err(invalid(
                "Bitcoin balance evidence was not exact, ordered, and bounded",
            ));
        }
        facts.push(BitcoinBalanceSnapshotFact::new(
            BitcoinBalanceSnapshotSubject {
                network_id: evidence.network_id.clone(),
                bitcoin_network: evidence.bitcoin_network.clone(),
                semantic_source_identity: evidence.semantic_source_identity.clone(),
                address: observed_address.clone(),
            },
            BitcoinBalanceSnapshotResponse {
                anchor_height: evidence.anchor_height,
                anchor_hash: evidence.anchor_hash.clone(),
                balance_sats: *observed_balance,
            },
        ));
    }
    let facts = NonEmpty::try_from_vec(facts)
        .map_err(|error| invalid(format!("Bitcoin fact batch was empty: {error}")))?;
    let receipt = BitcoinBalanceCollectionReceipt::from_verified_facts(plan, facts.values())?;
    Ok((receipt, facts))
}

/// Minimal authority returned by an atomic Bitcoin collection settlement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "balance_collection_receipt",
    schema = "mfm.bitcoin.balance_collection.receipt"
)]
pub struct BitcoinBalanceCollectionReceipt {
    network_id: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    anchor_height: u64,
    anchor_hash: String,
    addresses: Vec<String>,
    fact_content_identities: Vec<FactContentIdentityEvidence>,
}

impl BitcoinBalanceCollectionReceipt {
    fn from_verified_facts(
        plan: &BitcoinBalanceCollectionPlan,
        facts: &[BitcoinBalanceSnapshotFact],
    ) -> Result<Self, BitcoinBalanceCollectionError> {
        if facts.is_empty() || facts.len() != plan.addresses.len() {
            return Err(invalid("Bitcoin receipt fact coverage was not exact"));
        }
        let descriptor =
            BitcoinBalanceSnapshotFact::descriptor().map_err(|error| invalid(error.to_string()))?;
        let first = &facts[0];
        let mut addresses = Vec::with_capacity(facts.len());
        let mut identities = Vec::with_capacity(facts.len());
        for (expected, fact) in plan.addresses.iter().zip(facts) {
            if &fact.subject.address != expected
                || fact.subject.network_id != plan.network_id
                || fact.subject.bitcoin_network != plan.bitcoin_network
                || fact.subject.semantic_source_identity != plan.semantic_source_identity
                || fact.response.anchor_height != first.response.anchor_height
                || fact.response.anchor_hash != first.response.anchor_hash
            {
                return Err(invalid(
                    "Bitcoin facts did not share exact receipt authority",
                ));
            }
            let identity = mfm_facts::derive_fact_content_identity_from_typed_values(
                &descriptor,
                fact.subject(),
                fact.response(),
            )
            .map_err(|error| invalid(error.to_string()))?;
            addresses.push(expected.clone());
            identities.push(FactContentIdentityEvidence::from_verified(&identity));
        }
        Ok(Self {
            network_id: plan.network_id.clone(),
            bitcoin_network: plan.bitcoin_network.clone(),
            semantic_source_identity: plan.semantic_source_identity.clone(),
            anchor_height: first.response.anchor_height,
            anchor_hash: first.response.anchor_hash.clone(),
            addresses,
            fact_content_identities: identities,
        })
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the required Bitcoin chain tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }

    /// Returns the semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the shared scan anchor height.
    pub const fn anchor_height(&self) -> u64 {
        self.anchor_height
    }

    /// Returns the shared scan anchor hash.
    pub fn anchor_hash(&self) -> &str {
        &self.anchor_hash
    }

    /// Returns address keys in canonical request order.
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// Returns aligned fact content identities.
    pub fn fact_content_identities(&self) -> &[FactContentIdentityEvidence] {
        &self.fact_content_identities
    }
}

/// Subject identity for one aggregate Bitcoin balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "balance_snapshot_subject",
    schema = "mfm.bitcoin.fact.balance_snapshot.subject"
)]
pub struct BitcoinBalanceSnapshotSubject {
    network_id: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    address: String,
}

impl BitcoinBalanceSnapshotSubject {
    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the Bitcoin chain tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }

    /// Returns the semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the canonical address.
    pub fn address(&self) -> &str {
        &self.address
    }
}

/// Response material for one aggregate Bitcoin balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "balance_snapshot_response",
    schema = "mfm.bitcoin.fact.balance_snapshot.response"
)]
pub struct BitcoinBalanceSnapshotResponse {
    anchor_height: u64,
    anchor_hash: String,
    balance_sats: u64,
}

impl BitcoinBalanceSnapshotResponse {
    /// Returns the shared scan anchor height.
    pub const fn anchor_height(&self) -> u64 {
        self.anchor_height
    }

    /// Returns the shared scan anchor hash.
    pub fn anchor_hash(&self) -> &str {
        &self.anchor_hash
    }

    /// Returns the exact balance in satoshis.
    pub const fn balance_sats(&self) -> u64 {
        self.balance_sats
    }

    fn validate(&self) -> Result<(), BitcoinBalanceCollectionError> {
        BlockHash::from_str(&self.anchor_hash)
            .map_err(|_| invalid("Bitcoin fact anchor hash was invalid"))?;
        if self.balance_sats > Amount::MAX_MONEY.to_sat() {
            return Err(invalid("Bitcoin fact balance exceeded MAX_MONEY"));
        }
        Ok(())
    }
}

/// Aggregate Bitcoin balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, DeriveMfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "balance_snapshot_fact",
    schema = "mfm.bitcoin.fact.balance_snapshot"
)]
#[mfm_fact(kind = "bitcoin.balance_snapshot")]
#[mfm_fact(field(
    id = "subject.network_id",
    source = "subject",
    path = "network_id",
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
    exposure = "returnable"
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
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "metadata.store_commit_order.desc",
    term(
        field = "metadata.store_commit_order",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct BitcoinBalanceSnapshotFact {
    subject: BitcoinBalanceSnapshotSubject,
    response: BitcoinBalanceSnapshotResponse,
}

impl BitcoinBalanceSnapshotFact {
    /// Creates a fact from checked reduced material.
    pub const fn new(
        subject: BitcoinBalanceSnapshotSubject,
        response: BitcoinBalanceSnapshotResponse,
    ) -> Self {
        Self { subject, response }
    }

    /// Returns the typed subject.
    pub const fn subject(&self) -> &BitcoinBalanceSnapshotSubject {
        &self.subject
    }

    /// Returns the typed response.
    pub const fn response(&self) -> &BitcoinBalanceSnapshotResponse {
        &self.response
    }
}

/// Decodes one canonical, closed Bitcoin fact response and revalidates its domain invariants.
pub fn decode_bitcoin_balance_snapshot_response(
    bytes: &[u8],
) -> Result<BitcoinBalanceSnapshotResponse, BitcoinBalanceCollectionError> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|_| invalid("Bitcoin fact response was not canonical JSON"))?;
    let response: BitcoinBalanceSnapshotResponse = serde_json::from_slice(canonical.as_bytes())
        .map_err(|_| invalid("Bitcoin fact response did not match the closed schema"))?;
    response.validate()?;
    Ok(response)
}

/// Returns the stable Bitcoin JSON-RPC adapter kind.
pub fn bitcoin_jsonrpc_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        "mfm.bitcoin",
        "jsonrpc",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.bitcoin.adapter:jsonrpc"),
    )
}

/// Returns the replacement Bitcoin JSON-RPC adapter behavior version.
pub fn bitcoin_jsonrpc_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(ADAPTER_VERSION)
}

fn invalid(reason: impl Into<String>) -> BitcoinBalanceCollectionError {
    BitcoinBalanceCollectionError::invalid(reason)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
