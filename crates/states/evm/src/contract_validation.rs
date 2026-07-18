//! Exact-anchor EVM contract validation state and evidence reducer.

use alloy_eips::eip2930::AccessList;
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_effects::ReadExternal;
use mfm_evm_capabilities::{
    EvmBlock, EvmBlockSelector, EvmCall, EvmCode, EvmNetworkBinding, EvmReadCapability,
    EvmSessionEvidence, EVM_CALL_MAX_RESPONSE_BYTES,
};
use mfm_ids::LocalPublicId;
use mfm_portfolio_model::evm::EvmBlockAnchor;
use mfm_program::{
    AdapterBindingSpec, ExternalReadEvidenceSet, NoContext, ReadState, StateError, StateResult,
    StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue};
use serde::{Deserialize, Serialize};

use crate::canonical::{
    block_anchor_hash, block_anchor_number, canonical_address, canonical_bytes,
    canonical_bytes_len, canonical_hash, invalid, parse_address, parse_bytes, parse_hash,
    parse_quantity, validate_block_anchor, validate_session,
};
use crate::identity::{adapter_binding, state_kind, state_version};
use crate::{EvmAccessListEntry, EvmStateError, EVM_TRANSACTION_DATA_MAX_BYTES};

/// Maximum checked calls admitted by one validation node.
pub const EVM_CONTRACT_VALIDATION_MAX_CALLS: usize = 64;
/// Maximum runtime-code bytes retained by one validation node.
pub const EVM_CONTRACT_CODE_MAX_BYTES: usize = 128 * 1024;
/// Maximum aggregate code and call-return bytes retained by one validation node.
pub const EVM_CONTRACT_VALIDATION_MAX_EVIDENCE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum aggregate call-return bytes admitted while reserving the full code allowance.
pub const EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES: usize =
    EVM_CONTRACT_VALIDATION_MAX_EVIDENCE_BYTES - EVM_CONTRACT_CODE_MAX_BYTES;
/// Maximum aggregate authored calldata bytes retained by one validation policy.
pub const EVM_CONTRACT_VALIDATION_MAX_TOTAL_CALLDATA_BYTES: usize = 2 * 1024 * 1024;
/// Maximum aggregate authored access-list addresses retained by one validation policy.
pub const EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_ENTRIES: usize = 256;
/// Maximum aggregate authored access-list storage keys retained by one validation policy.
pub const EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_STORAGE_KEYS: usize = 4_096;
const MAX_ACCESS_LIST_ENTRIES: usize = 256;
const MAX_ACCESS_LIST_STORAGE_KEYS: usize = 4_096;
#[cfg(test)]
const MAX_CANONICAL_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

/// Fully specified read-only call context independent of its expected return.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract_call_context",
    version = "1",
    schema = "mfm.evm.contract_validation.call_context"
)]
pub struct EvmContractCallContext {
    caller: String,
    target: String,
    value: String,
    calldata: String,
    gas_limit: String,
    access_list: Vec<EvmAccessListEntry>,
}

impl EvmContractCallContext {
    /// Creates one fully specified call context.
    pub fn new(
        caller: Address,
        target: Address,
        value: U256,
        calldata: impl AsRef<[u8]>,
        gas_limit: U256,
        access_list: Vec<EvmAccessListEntry>,
    ) -> Result<Self, EvmStateError> {
        let context = Self {
            caller: canonical_address(caller),
            target: canonical_address(target),
            value: value.to_string(),
            calldata: canonical_bytes(calldata.as_ref()),
            gas_limit: gas_limit.to_string(),
            access_list,
        };
        context.validate()?;
        Ok(context)
    }

    /// Returns the canonical caller.
    pub fn caller(&self) -> &str {
        &self.caller
    }

    /// Returns the canonical target.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns the canonical decimal value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns canonical calldata.
    pub fn calldata(&self) -> &str {
        &self.calldata
    }

    /// Returns the canonical decimal gas bound.
    pub fn gas_limit(&self) -> &str {
        &self.gas_limit
    }

    /// Returns the checked access list.
    pub fn access_list(&self) -> &[EvmAccessListEntry] {
        &self.access_list
    }

    fn capability_request(
        &self,
        anchor: &EvmBlockAnchor,
        max_response_bytes: usize,
    ) -> Result<EvmCall, EvmStateError> {
        self.validate()?;
        validate_block_anchor(anchor)?;
        EvmCall::new(
            parse_address(&self.caller)?,
            parse_address(&self.target)?,
            parse_quantity(&self.value)?,
            Bytes::from(parse_bytes(&self.calldata, EVM_TRANSACTION_DATA_MAX_BYTES)?),
            parse_quantity(&self.gas_limit)?,
            access_list_to_alloy(&self.access_list)?,
            EvmBlockSelector::ExactHash(block_anchor_hash(anchor)?),
            max_response_bytes,
        )
        .map_err(|_| invalid("contract validation call request was invalid"))
    }

    fn validate(&self) -> Result<(), EvmStateError> {
        parse_address(&self.caller)?;
        parse_address(&self.target)?;
        parse_quantity(&self.value)?;
        canonical_bytes_len(&self.calldata, EVM_TRANSACTION_DATA_MAX_BYTES)?;
        if parse_quantity(&self.gas_limit)?.is_zero() {
            return Err(invalid("contract validation call gas limit was zero"));
        }
        validate_access_list(&self.access_list)?;
        Ok(())
    }
}

/// One fully specified exact-anchor call check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract_call_check",
    version = "1",
    schema = "mfm.evm.contract_validation.call_check"
)]
pub struct EvmContractCallCheck {
    context: EvmContractCallContext,
    expected_return: String,
}

impl EvmContractCallCheck {
    /// Creates one checked call and exact expected return.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        caller: Address,
        target: Address,
        value: U256,
        calldata: impl AsRef<[u8]>,
        gas_limit: U256,
        access_list: Vec<EvmAccessListEntry>,
        expected_return: impl AsRef<[u8]>,
    ) -> Result<Self, EvmStateError> {
        let check = Self {
            context: EvmContractCallContext::new(
                caller,
                target,
                value,
                calldata,
                gas_limit,
                access_list,
            )?,
            expected_return: canonical_bytes(expected_return.as_ref()),
        };
        check.validate()?;
        Ok(check)
    }

    /// Returns the complete call context.
    pub const fn context(&self) -> &EvmContractCallContext {
        &self.context
    }

    /// Returns the canonical caller.
    pub fn caller(&self) -> &str {
        self.context.caller()
    }

    /// Returns the canonical target.
    pub fn target(&self) -> &str {
        self.context.target()
    }

    /// Returns the canonical decimal value.
    pub fn value(&self) -> &str {
        self.context.value()
    }

    /// Returns canonical calldata.
    pub fn calldata(&self) -> &str {
        self.context.calldata()
    }

    /// Returns the canonical decimal gas bound.
    pub fn gas_limit(&self) -> &str {
        self.context.gas_limit()
    }

    /// Returns the checked access list.
    pub fn access_list(&self) -> &[EvmAccessListEntry] {
        self.context.access_list()
    }

    /// Returns canonical expected return bytes.
    pub fn expected_return(&self) -> &str {
        &self.expected_return
    }

    /// Creates the exact source-bound capability request at `anchor`.
    pub fn capability_request(&self, anchor: &EvmBlockAnchor) -> Result<EvmCall, EvmStateError> {
        self.validate()?;
        self.context
            .capability_request(anchor, self.expected_return_len()?)
    }

    fn validate(&self) -> Result<(), EvmStateError> {
        self.context.validate()?;
        self.expected_return_len()?;
        Ok(())
    }

    fn expected_return_len(&self) -> Result<usize, EvmStateError> {
        canonical_bytes_len(&self.expected_return, EVM_CALL_MAX_RESPONSE_BYTES)
    }
}

/// Certified validation policy independent of a particular contract anchor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.evm.contract_validation.config",
    validate = "validate_evm_contract_validation_config"
)]
pub struct EvmContractValidationConfig {
    network_id: String,
    chain_id: u64,
    expected_runtime_code_hash: String,
    calls: Vec<EvmContractCallCheck>,
}

impl EvmContractValidationConfig {
    /// Creates one exact validation policy.
    pub fn new(
        network_id: impl Into<String>,
        chain_id: u64,
        expected_runtime_code_hash: B256,
        calls: Vec<EvmContractCallCheck>,
    ) -> Result<Self, EvmStateError> {
        let config = Self {
            network_id: network_id.into(),
            chain_id,
            expected_runtime_code_hash: canonical_hash(expected_runtime_code_hash),
            calls,
        };
        config.validate().map_err(EvmStateError::invalid)?;
        Ok(config)
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the non-zero chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the mandatory expected runtime-code hash.
    pub fn expected_runtime_code_hash(&self) -> &str {
        &self.expected_runtime_code_hash
    }

    /// Returns ordered checked calls.
    pub fn calls(&self) -> &[EvmContractCallCheck] {
        &self.calls
    }

    /// Returns the checked runtime binding.
    pub fn network_binding(&self) -> Result<EvmNetworkBinding, EvmStateError> {
        self.validate().map_err(EvmStateError::invalid)?;
        EvmNetworkBinding::new(
            LocalPublicId::new(&self.network_id)
                .map_err(|_| invalid("contract validation network id was invalid"))?,
            self.chain_id,
        )
        .map_err(|_| invalid("contract validation network binding was invalid"))
    }

    fn validate(&self) -> Result<(), String> {
        validate_validation_policy(
            &self.network_id,
            self.chain_id,
            &self.expected_runtime_code_hash,
            &self.calls,
        )
    }
}

/// Validates one contract-validation config during typed authoring.
pub fn validate_evm_contract_validation_config(
    config: &EvmContractValidationConfig,
) -> Result<(), String> {
    config.validate()
}

/// Concrete contract and block anchor supplied by an upstream node or root seed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract_validation_target",
    version = "1",
    schema = "mfm.evm.contract_validation.target"
)]
pub struct EvmContractValidationTarget {
    address: String,
    anchor: EvmBlockAnchor,
}

impl EvmContractValidationTarget {
    /// Creates one concrete target at one number/hash anchor.
    pub fn new(address: Address, anchor: EvmBlockAnchor) -> Result<Self, EvmStateError> {
        let target = Self {
            address: canonical_address(address),
            anchor,
        };
        target.validate()?;
        Ok(target)
    }

    /// Returns the canonical contract address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns the exact block anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }

    fn validate(&self) -> Result<(), EvmStateError> {
        parse_address(&self.address)?;
        validate_block_anchor(&self.anchor)
    }
}

/// Complete immutable validation plan recomputed by live execution and replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract_validation_plan",
    version = "1",
    schema = "mfm.evm.contract_validation.plan"
)]
pub struct EvmContractValidationPlan {
    network_id: String,
    chain_id: u64,
    address: String,
    anchor: EvmBlockAnchor,
    expected_runtime_code_hash: String,
    calls: Vec<EvmContractCallCheck>,
}

impl EvmContractValidationPlan {
    fn from_config_and_target(
        config: &EvmContractValidationConfig,
        target: &EvmContractValidationTarget,
    ) -> Result<Self, EvmStateError> {
        config.validate().map_err(EvmStateError::invalid)?;
        target.validate()?;
        Ok(Self {
            network_id: config.network_id.clone(),
            chain_id: config.chain_id,
            address: target.address.clone(),
            anchor: target.anchor.clone(),
            expected_runtime_code_hash: config.expected_runtime_code_hash.clone(),
            calls: config.calls.clone(),
        })
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the canonical contract address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns the exact anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }

    /// Returns ordered checked calls.
    pub fn calls(&self) -> &[EvmContractCallCheck] {
        &self.calls
    }

    /// Returns the exact code-read request.
    pub fn code_request(&self) -> Result<(Address, EvmBlockSelector), EvmStateError> {
        self.validate()?;
        Ok((
            parse_address(&self.address)?,
            EvmBlockSelector::ExactHash(block_anchor_hash(&self.anchor)?),
        ))
    }

    /// Returns the final block-number selector used for canonicality.
    pub fn canonicality_selector(&self) -> Result<EvmBlockSelector, EvmStateError> {
        self.validate()?;
        Ok(EvmBlockSelector::Number(block_anchor_number(&self.anchor)?))
    }

    fn validate(&self) -> Result<(), EvmStateError> {
        validate_validation_policy(
            &self.network_id,
            self.chain_id,
            &self.expected_runtime_code_hash,
            &self.calls,
        )
        .map_err(EvmStateError::invalid)?;
        parse_address(&self.address)?;
        validate_block_anchor(&self.anchor)
    }
}

/// Ordered external-read observation retained for deterministic validation replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract_validation_observation",
    version = "1",
    schema = "mfm.evm.contract_validation.observation"
)]
pub enum EvmContractValidationObservation {
    /// Exact-hash runtime-code read.
    Code {
        /// Requested contract address.
        address: String,
        /// Exact canonical anchor hash selector.
        anchor_hash: String,
        /// Raw runtime code.
        code: String,
    },
    /// Exact-hash call and return.
    Call {
        /// Zero-based position in the immutable validation plan.
        call_index: u64,
        /// Canonical digest of the exact source-bound capability request.
        request_digest: String,
        /// Raw return bytes.
        return_data: String,
    },
    /// Final block-by-number canonicality observation.
    AnchorByNumber {
        /// Block returned for the authored anchor number.
        block: EvmBlockAnchor,
    },
}

/// Complete bounded external-read evidence for one validation attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract_validation_evidence",
    version = "1",
    schema = "mfm.evm.contract_validation.evidence"
)]
pub struct EvmContractValidationEvidence {
    observations: Vec<EvmContractValidationObservation>,
    session: EvmSessionEvidence,
}

impl EvmContractValidationEvidence {
    /// Returns the retained source binding.
    pub const fn session(&self) -> &EvmSessionEvidence {
        &self.session
    }
}

/// Non-serializable, single-pass constructor for bounded live validation evidence.
///
/// Code and call results are consumed one at a time. Each raw result is checked and converted into
/// its one compact observation before the caller can issue the next request.
pub struct EvmContractValidationEvidenceBuilder<'a> {
    plan: &'a EvmContractValidationPlan,
    observations: Vec<EvmContractValidationObservation>,
    session: EvmSessionEvidence,
    budget: EvmContractValidationEvidenceBudget,
    next_call_index: usize,
}

impl<'a> EvmContractValidationEvidenceBuilder<'a> {
    /// Admits and consumes the runtime-code response before any contract call is issued.
    pub fn new(
        plan: &'a EvmContractValidationPlan,
        code: EvmCode,
        session: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        plan.validate()?;
        validate_session(session, &plan.network_id, plan.chain_id)?;

        let EvmCode { bytes, hash } = code;
        if keccak256(&bytes) != hash {
            return Err(invalid("contract code capability hash was inconsistent"));
        }
        let budget = validate_runtime_code(plan, &bytes)?;
        let observations = vec![EvmContractValidationObservation::Code {
            address: plan.address.clone(),
            anchor_hash: plan.anchor.hash().to_owned(),
            code: canonical_bytes(&bytes),
        }];

        Ok(Self {
            plan,
            observations,
            session: session.clone(),
            budget,
            next_call_index: 0,
        })
    }

    /// Returns the next exact call request, or `None` after every planned response was admitted.
    pub fn next_call_request(&self) -> Result<Option<EvmCall>, EvmStateError> {
        self.plan
            .calls
            .get(self.next_call_index)
            .map(|call| call.capability_request(&self.plan.anchor))
            .transpose()
    }

    /// Admits and consumes the next call result in immutable plan order.
    pub fn push_call_response(&mut self, response: Bytes) -> Result<(), EvmStateError> {
        let call =
            self.plan.calls.get(self.next_call_index).ok_or_else(|| {
                invalid("contract validation received an unexpected call response")
            })?;
        let expected_len = call.expected_return_len()?;
        if response.len() > expected_len {
            return Err(invalid(
                "contract validation call response exceeded its request bound",
            ));
        }
        let expected = parse_bytes(&call.expected_return, EVM_CALL_MAX_RESPONSE_BYTES)?;
        if response.as_ref() != expected.as_slice() {
            return Err(invalid("contract call return bytes differed"));
        }
        self.budget.admit_return(response.len())?;

        let call_index = u64::try_from(self.next_call_index)
            .map_err(|_| invalid("contract validation call index overflowed"))?;
        self.observations
            .push(EvmContractValidationObservation::Call {
                call_index,
                request_digest: validation_call_request_digest(call, &self.plan.anchor)?,
                return_data: canonical_bytes(&response),
            });
        self.next_call_index += 1;
        Ok(())
    }

    /// Finishes evidence with the number-to-hash canonicality response.
    pub fn finish(
        mut self,
        final_canonical_block: EvmBlock,
    ) -> Result<EvmContractValidationEvidence, EvmStateError> {
        if self.next_call_index != self.plan.calls.len() {
            return Err(invalid(
                "contract validation evidence was missing call responses",
            ));
        }
        let block = EvmBlockAnchor::new(final_canonical_block.number, final_canonical_block.hash);
        validate_block_anchor(&block)?;
        if block != self.plan.anchor {
            return Err(invalid(
                "final block-by-number result did not preserve the authored anchor",
            ));
        }
        self.observations
            .push(EvmContractValidationObservation::AnchorByNumber { block });
        Ok(EvmContractValidationEvidence {
            observations: self.observations,
            session: self.session,
        })
    }
}

/// Compact proof that the declared checks held at one exact anchor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "verified_contract",
    version = "1",
    schema = "mfm.evm.contract_validation.verified"
)]
pub struct VerifiedEvmContract {
    address: String,
    anchor: EvmBlockAnchor,
    observed_runtime_code_hash: String,
    validation_plan_digest: String,
}

impl VerifiedEvmContract {
    /// Returns the verified contract address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns the exact verified anchor.
    pub const fn anchor(&self) -> &EvmBlockAnchor {
        &self.anchor
    }

    /// Returns the observed non-empty runtime-code hash.
    pub fn observed_runtime_code_hash(&self) -> &str {
        &self.observed_runtime_code_hash
    }

    /// Returns the canonical validation-plan digest.
    pub fn validation_plan_digest(&self) -> &str {
        &self.validation_plan_digest
    }
}

/// Recomputes all validation relations from retained typed evidence.
pub fn validate_evm_contract(
    plan: &EvmContractValidationPlan,
    evidence: &EvmContractValidationEvidence,
) -> Result<VerifiedEvmContract, EvmStateError> {
    plan.validate()?;
    validate_session(&evidence.session, &plan.network_id, plan.chain_id)?;
    let mut observations = evidence.observations.iter();
    let Some(EvmContractValidationObservation::Code {
        address,
        anchor_hash,
        code,
    }) = observations.next()
    else {
        return Err(invalid(
            "contract validation evidence did not begin with the code read",
        ));
    };
    if address != &plan.address || anchor_hash != plan.anchor.hash() {
        return Err(invalid(
            "contract validation evidence did not match the authored read plan",
        ));
    }

    let code = parse_bytes(code, EVM_CONTRACT_CODE_MAX_BYTES)?;
    let mut budget = validate_runtime_code(plan, &code)?;
    let observed_code_hash = keccak256(&code);

    for (expected_index, expected) in plan.calls.iter().enumerate() {
        let Some(EvmContractValidationObservation::Call {
            call_index,
            request_digest,
            return_data,
        }) = observations.next()
        else {
            return Err(invalid(
                "contract call evidence was missing, reordered, or changed",
            ));
        };
        let expected_index = u64::try_from(expected_index)
            .map_err(|_| invalid("contract validation call index overflowed"))?;
        if *call_index != expected_index
            || request_digest != &validation_call_request_digest(expected, &plan.anchor)?
        {
            return Err(invalid(
                "contract call evidence was missing, reordered, or changed",
            ));
        }
        let returned = parse_bytes(return_data, EVM_CALL_MAX_RESPONSE_BYTES)?;
        let expected_return = parse_bytes(&expected.expected_return, EVM_CALL_MAX_RESPONSE_BYTES)?;
        if returned != expected_return {
            return Err(invalid("contract call return bytes differed"));
        }
        budget.admit_return(returned.len())?;
    }

    let Some(EvmContractValidationObservation::AnchorByNumber { block }) = observations.next()
    else {
        return Err(invalid(
            "contract validation evidence did not end with block-by-number canonicality",
        ));
    };
    if observations.next().is_some() {
        return Err(invalid(
            "contract validation evidence contained unexpected extra observations",
        ));
    }
    validate_block_anchor(block)?;
    if block != &plan.anchor {
        return Err(invalid(
            "final block-by-number result did not preserve the authored anchor",
        ));
    }

    Ok(VerifiedEvmContract {
        address: plan.address.clone(),
        anchor: plan.anchor.clone(),
        observed_runtime_code_hash: canonical_hash(observed_code_hash),
        validation_plan_digest: validation_plan_digest(plan)?,
    })
}

/// Reusable exact-anchor contract validation read state.
pub struct ValidateEvmContractState {
    config: EvmContractValidationConfig,
}

impl ValidateEvmContractState {
    /// Returns the certified validation config.
    pub const fn config(&self) -> &EvmContractValidationConfig {
        &self.config
    }

    /// Builds the complete immutable read plan for a concrete target.
    pub fn plan(
        &self,
        target: &EvmContractValidationTarget,
    ) -> Result<EvmContractValidationPlan, EvmStateError> {
        EvmContractValidationPlan::from_config_and_target(&self.config, target)
    }

    /// Applies the state-owned validation reducer.
    pub fn reduce(
        &self,
        target: &EvmContractValidationTarget,
        evidence: &EvmContractValidationEvidence,
    ) -> Result<VerifiedEvmContract, EvmStateError> {
        validate_evm_contract(&self.plan(target)?, evidence)
    }
}

impl StateSpec for ValidateEvmContractState {
    type Config = EvmContractValidationConfig;
    type Context = NoContext;
    type Input = EvmContractValidationTarget;
    type Output = VerifiedEvmContract;
    type Effect = ReadExternal;
    type Caps = (EvmReadCapability,);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("contract.validate")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("contract.validate")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.validate"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for ValidateEvmContractState {
    type Plan = EvmContractValidationPlan;
    type Evidence = EvmContractValidationEvidence;

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        self.plan(input).map_err(StateError::from)
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        if !evidence.fact_query_evidence().is_empty() {
            return Err(StateError::Message(
                "EVM contract validation carried unexpected fact-query evidence".to_owned(),
            ));
        }
        self.reduce(input, evidence.primary_evidence())
            .map_err(StateError::from)
    }
}

fn validate_validation_policy(
    network_id: &str,
    chain_id: u64,
    expected_runtime_code_hash: &str,
    calls: &[EvmContractCallCheck],
) -> Result<(), String> {
    LocalPublicId::new(network_id)
        .map_err(|_| "contract validation network id was invalid".to_owned())?;
    if chain_id == 0 {
        return Err("contract validation chain id must be non-zero".to_owned());
    }
    let expected = parse_hash(expected_runtime_code_hash).map_err(|error| error.to_string())?;
    if expected == keccak256([]) {
        return Err("expected runtime-code hash must not be the empty-code hash".to_owned());
    }
    if calls.len() > EVM_CONTRACT_VALIDATION_MAX_CALLS {
        return Err("contract validation contained too many calls".to_owned());
    }

    let mut total_returns = 0usize;
    let mut total_calldata = 0usize;
    let mut total_access_list_entries = 0usize;
    let mut total_access_list_storage_keys = 0usize;
    for call in calls {
        call.validate().map_err(|error| error.to_string())?;
        total_returns = checked_policy_sum(
            total_returns,
            call.expected_return_len()
                .map_err(|error| error.to_string())?,
            "contract validation expected-return size overflowed",
        )?;
        total_calldata = checked_policy_sum(
            total_calldata,
            canonical_bytes_len(call.calldata(), EVM_TRANSACTION_DATA_MAX_BYTES)
                .map_err(|error| error.to_string())?,
            "contract validation calldata size overflowed",
        )?;
        total_access_list_entries = checked_policy_sum(
            total_access_list_entries,
            call.access_list().len(),
            "contract validation access-list size overflowed",
        )?;
        for entry in call.access_list() {
            total_access_list_storage_keys = checked_policy_sum(
                total_access_list_storage_keys,
                entry.storage_keys().len(),
                "contract validation access-list storage-key size overflowed",
            )?;
        }
    }

    if total_returns > EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES {
        return Err(
            "contract validation expected returns exceeded their aggregate bound".to_owned(),
        );
    }
    if total_calldata > EVM_CONTRACT_VALIDATION_MAX_TOTAL_CALLDATA_BYTES {
        return Err("contract validation calldata exceeded its aggregate bound".to_owned());
    }
    if total_access_list_entries > EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_ENTRIES {
        return Err(
            "contract validation access-list addresses exceeded their aggregate bound".to_owned(),
        );
    }
    if total_access_list_storage_keys > EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_STORAGE_KEYS {
        return Err(
            "contract validation access-list storage keys exceeded their aggregate bound"
                .to_owned(),
        );
    }
    Ok(())
}

fn checked_policy_sum(current: usize, added: usize, reason: &str) -> Result<usize, String> {
    current.checked_add(added).ok_or_else(|| reason.to_owned())
}

struct EvmContractValidationEvidenceBudget {
    retained_bytes: usize,
}

impl EvmContractValidationEvidenceBudget {
    fn from_code(code_len: usize) -> Result<Self, EvmStateError> {
        if code_len == 0 {
            return Err(invalid("observed contract runtime code was empty"));
        }
        if code_len > EVM_CONTRACT_CODE_MAX_BYTES {
            return Err(invalid("observed contract runtime code exceeded its bound"));
        }
        if code_len > EVM_CONTRACT_VALIDATION_MAX_EVIDENCE_BYTES {
            return Err(invalid("contract validation evidence exceeded its bound"));
        }
        Ok(Self {
            retained_bytes: code_len,
        })
    }

    fn admit_return(&mut self, return_len: usize) -> Result<(), EvmStateError> {
        if return_len > EVM_CALL_MAX_RESPONSE_BYTES {
            return Err(invalid(
                "contract validation call response exceeded its request bound",
            ));
        }
        let retained_bytes = self
            .retained_bytes
            .checked_add(return_len)
            .ok_or_else(|| invalid("contract validation evidence size overflowed"))?;
        if retained_bytes > EVM_CONTRACT_VALIDATION_MAX_EVIDENCE_BYTES {
            return Err(invalid("contract validation evidence exceeded its bound"));
        }
        self.retained_bytes = retained_bytes;
        Ok(())
    }
}

fn validate_runtime_code(
    plan: &EvmContractValidationPlan,
    code: &[u8],
) -> Result<EvmContractValidationEvidenceBudget, EvmStateError> {
    let budget = EvmContractValidationEvidenceBudget::from_code(code.len())?;
    if canonical_hash(keccak256(code)) != plan.expected_runtime_code_hash {
        return Err(invalid("observed contract runtime-code hash differed"));
    }
    Ok(budget)
}

fn validation_call_request_digest(
    call: &EvmContractCallCheck,
    anchor: &EvmBlockAnchor,
) -> Result<String, EvmStateError> {
    #[derive(Serialize)]
    struct DigestMaterial<'a> {
        context: &'a EvmContractCallContext,
        anchor_hash: &'a str,
        max_response_bytes: u64,
    }

    validate_block_anchor(anchor)?;
    let max_response_bytes = u64::try_from(call.expected_return_len()?)
        .map_err(|_| invalid("contract validation response bound overflowed"))?;
    let material = DigestMaterial {
        context: call.context(),
        anchor_hash: anchor.hash(),
        max_response_bytes,
    };
    let json = serde_json::to_string(&material)
        .map_err(|_| invalid("contract validation call request could not be serialized"))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|bytes| bytes.content_digest().as_str().to_owned())
        .map_err(|_| invalid("contract validation call request could not be canonicalized"))
}

fn access_list_to_alloy(entries: &[EvmAccessListEntry]) -> Result<AccessList, EvmStateError> {
    validate_access_list(entries)?;
    entries
        .iter()
        .map(EvmAccessListEntry::to_alloy)
        .collect::<Result<Vec<_>, _>>()
        .map(AccessList)
}

fn validate_access_list(entries: &[EvmAccessListEntry]) -> Result<(), EvmStateError> {
    if entries.len() > MAX_ACCESS_LIST_ENTRIES {
        return Err(invalid(
            "contract call access list contained too many addresses",
        ));
    }
    let mut storage_keys = 0usize;
    for entry in entries {
        entry.validate()?;
        storage_keys = storage_keys
            .checked_add(entry.storage_keys().len())
            .ok_or_else(|| invalid("contract call access list size overflowed"))?;
    }
    if storage_keys > MAX_ACCESS_LIST_STORAGE_KEYS {
        return Err(invalid(
            "contract call access list contained too many storage keys",
        ));
    }
    Ok(())
}

fn validation_plan_digest(plan: &EvmContractValidationPlan) -> Result<String, EvmStateError> {
    let json = serde_json::to_string(plan)
        .map_err(|_| invalid("contract validation plan could not be serialized"))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|bytes| bytes.content_digest().as_str().to_owned())
        .map_err(|_| invalid("contract validation plan could not be canonicalized"))
}

#[cfg(test)]
#[path = "contract_validation_tests.rs"]
mod tests;
