//! Durable EVM transaction intent records used by write-side runtime states.

use std::fmt;

use alloy_primitives::keccak256;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use mfm_machine::ids::{FactKey, StateId};
use mfm_state_common::errors as op_errors;

use crate::rpc::normalize_quantity_hex;
use mfm_evm_dcv_model as shared_dcv;

/// Schema version for [`TxIntentV1`].
pub const TX_INTENT_SCHEMA_VERSION_V1: u32 = 1;

/// Kind of EVM transaction represented by a durable intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxIntentKind {
    /// Contract-creation transaction.
    ContractCreate,
    /// Call transaction to an existing contract.
    ContractCall,
}

/// Durable, non-secret intent for one signed EVM transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxIntentV1 {
    /// Intent schema version.
    pub schema_version: u32,
    /// Stable managed network identifier.
    pub network_id: String,
    /// Stable managed RPC control scope.
    pub control_scope: String,
    /// State-local logical transaction identifier, independent of runtime attempt number.
    pub logical_tx_id: String,
    /// Transaction kind.
    pub kind: TxIntentKind,
    /// Normalized sender address.
    pub from: String,
    /// Normalized recipient address for call transactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Normalized transaction value quantity.
    pub value_hex: String,
    /// Keccak-256 hash of calldata or initcode.
    pub calldata_or_initcode_hash: String,
    /// Normalized gas limit quantity.
    pub gas_limit_hex: String,
    /// Normalized legacy gas price quantity.
    pub gas_price_hex: String,
    /// Normalized nonce quantity.
    pub nonce_hex: String,
    /// EVM chain id used for signing.
    pub chain_id: u64,
    /// Expected transaction hash derived from the protected raw transaction capability.
    pub raw_tx_hash: String,
}

/// Protected signed transaction capability for one EVM transaction.
///
/// This type intentionally does not implement `Serialize`; raw signed transaction bytes can spend
/// funds and must only be persisted through the protected artifact path.
#[derive(Clone)]
pub struct SignedTxCapabilityV1 {
    raw_tx_bytes: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for SignedTxCapabilityV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SignedTxCapabilityV1(REDACTED)")
    }
}

/// In-memory result of preparing a signed transaction.
#[derive(Clone, Debug)]
pub struct PreparedTxIntentV1 {
    /// Public deterministic transaction intent metadata.
    pub intent: TxIntentV1,
    /// Protected signed transaction capability.
    pub capability: SignedTxCapabilityV1,
}

fn normalize_nonempty_component(
    value: &str,
    code: &'static str,
    message: &'static str,
) -> Result<String, mfm_machine::errors::StateError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(op_errors::state_unknown(code, message));
    }
    Ok(trimmed.to_string())
}

fn payload_hash_hex(payload: &[u8]) -> String {
    let hash = keccak256(payload);
    shared_dcv::bytes_to_hex_prefixed(hash.as_slice())
}

/// Computes the expected EVM transaction hash for a signed raw transaction.
pub fn raw_transaction_hash(raw_tx_hex: &str) -> Result<String, mfm_machine::errors::StateError> {
    let normalized = shared_dcv::normalize_hex_str(raw_tx_hex).map_err(|_| {
        op_errors::state_unknown("invalid_raw_tx_hex", "raw transaction hex was invalid")
    })?;
    let bytes = shared_dcv::hex_to_bytes(&normalized).map_err(|_| {
        op_errors::state_unknown("invalid_raw_tx_hex", "raw transaction hex was invalid")
    })?;
    raw_transaction_hash_bytes(&bytes)
}

/// Computes the expected EVM transaction hash for signed raw transaction bytes.
pub fn raw_transaction_hash_bytes(bytes: &[u8]) -> Result<String, mfm_machine::errors::StateError> {
    if bytes.is_empty() {
        return Err(op_errors::state_unknown(
            "invalid_raw_tx_hex",
            "raw transaction hex was empty",
        ));
    }
    let hash = keccak256(bytes);
    Ok(shared_dcv::bytes_to_hex_prefixed(hash.as_slice()))
}

impl SignedTxCapabilityV1 {
    /// Builds a protected capability from normalized or normalizable raw transaction hex.
    pub fn from_raw_tx_hex(raw_tx_hex: &str) -> Result<Self, mfm_machine::errors::StateError> {
        let normalized = shared_dcv::normalize_hex_str(raw_tx_hex).map_err(|_| {
            op_errors::state_unknown("invalid_raw_tx_hex", "raw transaction hex was invalid")
        })?;
        let bytes = shared_dcv::hex_to_bytes(&normalized).map_err(|_| {
            op_errors::state_unknown("invalid_raw_tx_hex", "raw transaction hex was invalid")
        })?;
        raw_transaction_hash_bytes(&bytes)?;
        Ok(Self {
            raw_tx_bytes: Zeroizing::new(bytes),
        })
    }

    /// Returns the protected raw transaction bytes.
    pub(crate) fn raw_tx_bytes(&self) -> &[u8] {
        &self.raw_tx_bytes
    }

    /// Computes the transaction hash for this protected capability.
    pub fn raw_tx_hash(&self) -> Result<String, mfm_machine::errors::StateError> {
        raw_transaction_hash_bytes(&self.raw_tx_bytes)
    }
}

impl PreparedTxIntentV1 {
    /// Builds a contract-creation intent and protected capability from a signed raw transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn signed_legacy_create(
        network_id: &str,
        control_scope: &str,
        logical_tx_id: &str,
        from: &str,
        value_hex: &str,
        initcode: &[u8],
        gas_limit_hex: &str,
        gas_price_hex: &str,
        nonce_hex: &str,
        chain_id: u64,
        raw_tx_hex: &str,
    ) -> Result<Self, mfm_machine::errors::StateError> {
        Self::signed_legacy(
            network_id,
            control_scope,
            logical_tx_id,
            TxIntentKind::ContractCreate,
            from,
            None,
            value_hex,
            initcode,
            gas_limit_hex,
            gas_price_hex,
            nonce_hex,
            chain_id,
            raw_tx_hex,
        )
    }

    /// Builds a contract-call intent and protected capability from a signed raw transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn signed_legacy_call(
        network_id: &str,
        control_scope: &str,
        logical_tx_id: &str,
        from: &str,
        to: &str,
        value_hex: &str,
        calldata: &[u8],
        gas_limit_hex: &str,
        gas_price_hex: &str,
        nonce_hex: &str,
        chain_id: u64,
        raw_tx_hex: &str,
    ) -> Result<Self, mfm_machine::errors::StateError> {
        Self::signed_legacy(
            network_id,
            control_scope,
            logical_tx_id,
            TxIntentKind::ContractCall,
            from,
            Some(to),
            value_hex,
            calldata,
            gas_limit_hex,
            gas_price_hex,
            nonce_hex,
            chain_id,
            raw_tx_hex,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn signed_legacy(
        network_id: &str,
        control_scope: &str,
        logical_tx_id: &str,
        kind: TxIntentKind,
        from: &str,
        to: Option<&str>,
        value_hex: &str,
        calldata_or_initcode: &[u8],
        gas_limit_hex: &str,
        gas_price_hex: &str,
        nonce_hex: &str,
        chain_id: u64,
        raw_tx_hex: &str,
    ) -> Result<Self, mfm_machine::errors::StateError> {
        let capability = SignedTxCapabilityV1::from_raw_tx_hex(raw_tx_hex)?;
        let network_id = normalize_nonempty_component(
            network_id,
            "invalid_network_id",
            "network_id must be non-empty",
        )?;
        let control_scope = normalize_nonempty_component(
            control_scope,
            "invalid_control_scope",
            "control_scope must be non-empty",
        )?;
        let logical_tx_id = normalize_nonempty_component(
            logical_tx_id,
            "invalid_logical_tx_id",
            "logical transaction id must be non-empty and contain no whitespace",
        )?;
        let from = shared_dcv::normalize_address(from).map_err(|_| {
            op_errors::state_unknown("invalid_from_address", "from address was invalid")
        })?;
        let to = to
            .map(|value| {
                shared_dcv::normalize_address(value).map_err(|_| {
                    op_errors::state_unknown("invalid_to_address", "to address was invalid")
                })
            })
            .transpose()?;
        let value_hex = normalize_quantity_hex(value_hex, "transaction value was invalid")?;
        let gas_limit_hex = normalize_quantity_hex(gas_limit_hex, "gas limit was invalid")?;
        let gas_price_hex = normalize_quantity_hex(gas_price_hex, "gas price was invalid")?;
        let nonce_hex = normalize_quantity_hex(nonce_hex, "nonce was invalid")?;
        let raw_tx_hash = capability.raw_tx_hash()?;

        let intent = TxIntentV1 {
            schema_version: TX_INTENT_SCHEMA_VERSION_V1,
            network_id,
            control_scope,
            logical_tx_id,
            kind,
            from,
            to,
            value_hex,
            calldata_or_initcode_hash: payload_hash_hex(calldata_or_initcode),
            gas_limit_hex,
            gas_price_hex,
            nonce_hex,
            chain_id,
            raw_tx_hash,
        };

        Ok(Self { intent, capability })
    }
}

impl TxIntentV1 {
    /// Derives the attempt-independent fact key used to record this intent.
    pub fn fact_key(&self, state_id: &StateId) -> FactKey {
        tx_intent_fact_key(
            state_id,
            &self.network_id,
            &self.control_scope,
            &self.logical_tx_id,
        )
    }

    /// Derives the attempt-independent fact key used to record the protected capability.
    pub fn capability_fact_key(&self, state_id: &StateId) -> FactKey {
        tx_capability_fact_key(
            state_id,
            &self.network_id,
            &self.control_scope,
            &self.logical_tx_id,
        )
    }
}

/// Derives the attempt-independent fact key for a logical transaction intent.
pub fn tx_intent_fact_key(
    state_id: &StateId,
    network_id: &str,
    control_scope: &str,
    logical_tx_id: &str,
) -> FactKey {
    FactKey(format!(
        "mfm:evm.tx_intent|state:{}|scope:{}|network:{}|logical_tx:{}",
        state_id.as_str(),
        control_scope,
        network_id,
        logical_tx_id
    ))
}

/// Derives the attempt-independent fact key for a protected signed transaction capability.
pub fn tx_capability_fact_key(
    state_id: &StateId,
    network_id: &str,
    control_scope: &str,
    logical_tx_id: &str,
) -> FactKey {
    FactKey(format!(
        "mfm:evm.tx_capability|state:{}|scope:{}|network:{}|logical_tx:{}",
        state_id.as_str(),
        control_scope,
        network_id,
        logical_tx_id
    ))
}

#[cfg(test)]
mod tests {
    use mfm_machine::hashing::artifact_id_for_json;

    use super::*;

    fn sample_intent() -> TxIntentV1 {
        PreparedTxIntentV1::signed_legacy_call(
            "ethereum-mainnet",
            "shared",
            "configure:0",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            "0x0",
            &[0xde, 0xad, 0xbe, 0xef],
            "0x5208",
            "0x1",
            "0x0",
            1,
            "0xf86c808252089422222222222222222222222222222222222222228084deadbeef25a0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .expect("intent")
        .intent
    }

    #[test]
    fn tx_intent_hashes_as_canonical_non_secret_json() {
        let left = serde_json::to_value(sample_intent()).expect("intent json");
        let right = serde_json::to_value(sample_intent()).expect("intent json");

        assert_eq!(
            artifact_id_for_json(&left).expect("left hash"),
            artifact_id_for_json(&right).expect("right hash")
        );
    }

    #[test]
    fn tx_intent_canonical_hash_rejects_float_mutation() {
        let mut value = serde_json::to_value(sample_intent()).expect("intent json");
        value["chain_id"] = serde_json::json!(1.5);

        assert!(artifact_id_for_json(&value).is_err());
    }

    #[test]
    fn tx_intent_rejects_secret_shaped_mutation() {
        let mut value = serde_json::to_value(sample_intent()).expect("intent json");
        value["private_key"] = serde_json::json!("do not persist");

        assert!(artifact_id_for_json(&value).is_err());
    }

    #[test]
    fn tx_intent_public_json_does_not_include_raw_transaction() {
        let value = serde_json::to_value(sample_intent()).expect("intent json");
        let rendered = serde_json::to_string(&value).expect("intent string");

        assert!(value.get("raw_tx_hash").is_some());
        assert!(value.get("raw_tx_hex").is_none());
        assert!(value.get("signature").is_none());
        assert!(!rendered.contains("raw_tx_hex"));
        assert!(!rendered.contains("signature"));
    }

    #[test]
    fn signed_tx_capability_debug_redacts_raw_transaction() {
        let raw_tx_hex = "0xdeadbeefcafebabe";
        let capability =
            SignedTxCapabilityV1::from_raw_tx_hex(raw_tx_hex).expect("capability from raw tx");
        let rendered = format!("{capability:?}");

        assert!(rendered.contains("REDACTED"));
        assert!(!rendered.contains(raw_tx_hex));
        assert!(!rendered.contains("deadbeefcafebabe"));
    }

    #[test]
    fn tx_intent_fact_key_is_attempt_independent_and_stable() {
        let state_id = StateId::must_new("evm.write.configure".to_string());
        let left = tx_intent_fact_key(&state_id, "ethereum-mainnet", "shared", "configure:0");
        let right = tx_intent_fact_key(&state_id, "ethereum-mainnet", "shared", "configure:0");
        let other = tx_intent_fact_key(&state_id, "ethereum-mainnet", "shared", "configure:1");

        assert_eq!(left, right);
        assert_ne!(left, other);
        assert!(!left.0.contains("attempt"));
    }

    #[test]
    fn tx_capability_fact_key_is_separate_from_public_intent_key() {
        let state_id = StateId::must_new("evm.write.configure".to_string());
        let intent = sample_intent();

        assert_ne!(
            intent.fact_key(&state_id),
            intent.capability_fact_key(&state_id)
        );
    }
}
