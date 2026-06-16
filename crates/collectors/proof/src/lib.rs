#![warn(missing_docs)]
//! Typed proof domain contracts.
//!
//! This crate owns the proof value, capability, state, and replay-verifier contracts used by the
//! typed proof operation. It has no dependency on the legacy machine, SDK, context, or generic IO
//! surfaces.

use std::future;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ExternalMutationAuthorityRole, NoCaps, ReadExternalRole,
};
use mfm_effects::{ApplySideEffect, Pure, ReadExternal};
use mfm_ids::{
    AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, DigestAlgorithm, StateKind,
    StateVersion,
};
use mfm_program::{
    AdapterBindingSpec, IdempotencyKey, PureState, ReadState, SideEffectState, StateResult,
    StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs, StateInput};
use serde::{de, Deserialize, Serialize};

const NAMESPACE: &str = "mfm.proof";
const ADAPTER_NAME: &str = "deterministic-proof";
const ADAPTER_VERSION: &str = "mfm.proof.adapter.deterministic.v1";

/// Returns the deterministic proof adapter kind.
pub fn proof_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        NAMESPACE,
        ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.proof.adapter:deterministic-proof"),
    )
}

/// Returns the deterministic proof adapter version.
pub fn proof_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(ADAPTER_VERSION)
}

fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: proof_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("proof adapter kind invalid: {error}"))
        })?,
        adapter_version: proof_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!("proof adapter version invalid: {error}"))
        })?,
    }])
}

fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.proof.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.proof.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn capability_kind(name: &'static str) -> mfm_capabilities::Result<CapabilityKind> {
    CapabilityKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.proof.capability:{name}").as_bytes()),
    )
    .map_err(|error| CapabilityError::Identity(error.to_string()))
}

/// Read capability used by the proof fact state.
pub struct ProofReadCapability;

impl CapabilitySpec for ProofReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        capability_kind("read")
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.proof.capability.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.proof.read"
    }
}

/// External mutation capability used by the proof side-effect state.
pub struct ProofMutationCapability;

impl CapabilitySpec for ProofMutationCapability {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        capability_kind("mutation")
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.proof.capability.mutation.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.proof.mutation"
    }
}

/// Config for the proof fact read state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.proof",
    name = "read-config",
    schema = "mfm.proof.config.read_fact"
)]
pub struct ProofReadConfig {
    /// Deterministic fact value returned by the enabled proof implementation.
    pub fact_n: u64,
}

/// Config for the proof side-effect state.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.proof",
    name = "apply-config",
    schema = "mfm.proof.config.apply_side_effect"
)]
pub struct ProofApplyConfig {
    /// Stable action name included in the intent and idempotency input.
    action: String,
}

impl ProofApplyConfig {
    /// Creates a proof apply config with a non-empty action.
    pub fn new(action: impl Into<String>) -> Result<Self, String> {
        let action = action.into();
        if action.trim().is_empty() {
            return Err("proof action must be non-empty".to_owned());
        }
        Ok(Self { action })
    }

    /// Returns the stable proof action.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Consumes this config into the stable proof action.
    pub fn into_action(self) -> String {
        self.action
    }
}

impl<'de> Deserialize<'de> for ProofApplyConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawProofApplyConfig {
            action: String,
        }

        let raw = RawProofApplyConfig::deserialize(deserializer)?;
        Self::new(raw.action).map_err(de::Error::custom)
    }
}

/// Config for proof output assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.proof",
    name = "assemble-config",
    schema = "mfm.proof.config.assemble_output"
)]
pub struct ProofAssembleConfig {}

/// Root proof workflow config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.proof",
    name = "workflow-config",
    schema = "mfm.proof.config.workflow"
)]
pub struct ProofWorkflowConfig {
    /// Config for the deterministic proof fact read.
    pub read: ProofReadConfig,
    /// Config for the proof side-effect mutation.
    pub apply: ProofApplyConfig,
}

impl ProofWorkflowConfig {
    /// Creates a proof workflow config from validated child configs.
    pub const fn new(read: ProofReadConfig, apply: ProofApplyConfig) -> Self {
        Self { read, apply }
    }
}

impl Default for ProofWorkflowConfig {
    fn default() -> Self {
        Self::new(
            ProofReadConfig { fact_n: 1 },
            ProofApplyConfig::new("accept").expect("default proof action is non-empty"),
        )
    }
}

/// Recorded proof fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "fact",
    version = "1",
    schema = "mfm.proof.fact"
)]
pub struct ProofFact {
    /// Deterministic fact value.
    pub n: u64,
}

/// Canonical request used to record the proof fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "fact_request",
    version = "1",
    schema = "mfm.proof.fact_request"
)]
pub struct ProofFactRequest {
    /// Deterministic proof source name.
    pub source: String,
}

/// Canonical response artifact for the proof fact read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "fact_response",
    version = "1",
    schema = "mfm.proof.fact_response"
)]
pub struct ProofFactResponse {
    /// Recorded fact payload.
    pub fact: ProofFact,
}

/// Proof mutation intent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "intent",
    version = "1",
    schema = "mfm.proof.intent"
)]
pub struct ProofIntent {
    /// Fact value being acted on.
    pub fact_n: u64,
    /// Stable action name.
    pub action: String,
}

/// Stable proof idempotency input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "idempotency_input",
    version = "1",
    schema = "mfm.proof.idempotency_input"
)]
pub struct ProofIdempotencyInput {
    /// Fact value bound into the idempotency key.
    pub fact_n: u64,
    /// Stable action name bound into the idempotency key.
    pub action: String,
}

/// Proof submission evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "submission",
    version = "1",
    schema = "mfm.proof.submission"
)]
pub struct ProofSubmission {
    /// Stable submission id.
    pub submission_id: String,
    /// Idempotency digest used for submission.
    pub idempotency_digest: String,
}

/// Proof receipt evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "receipt",
    version = "1",
    schema = "mfm.proof.receipt"
)]
pub struct ProofReceipt {
    /// Deterministic transaction hash.
    pub tx_hash: String,
    /// Submission id accepted by the implementation.
    pub submission_id: String,
}

/// Proof confirmation evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "confirmation",
    version = "1",
    schema = "mfm.proof.confirmation"
)]
pub struct ProofConfirmation {
    /// Confirmed transaction hash.
    pub tx_hash: String,
    /// Number of deterministic confirmations.
    pub confirmations: u64,
}

/// Terminal side-effect result consumed by proof output assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "side_effect_result",
    version = "1",
    schema = "mfm.proof.side_effect_result"
)]
pub struct ProofSideEffectResult {
    /// Confirmed transaction hash.
    pub tx_hash: String,
    /// Number of deterministic confirmations.
    pub confirmations: u64,
    /// Terminal side-effect status.
    pub status: String,
}

/// Terminal proof output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.proof",
    name = "output",
    version = "1",
    schema = "mfm.proof.output"
)]
pub struct ProofOutput {
    /// Recorded fact used by the proof workflow.
    pub fact: ProofFact,
    /// Confirmed side-effect result.
    pub side_effect: ProofSideEffectResult,
}

/// Input consumed by proof output assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.proof.input.assemble_output")]
pub struct ProofAssembleInput {
    /// Recorded fact.
    pub fact: ProofFact,
    /// Confirmed side-effect result.
    pub side_effect: ProofSideEffectResult,
}

/// Public output contract for proof workflows.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.proof.public_outputs")]
pub struct ProofPublicOutputs<'program, 'scope> {
    /// Terminal proof output.
    pub output: mfm_program::Handle<'program, 'scope, ProofOutput>,
}

/// Operation output handles produced by [`ProofWorkflowOperation`].
#[derive(OperationOutput)]
#[mfm(schema = "mfm.proof.operation_outputs")]
pub struct ProofOperationOutputs<'program, 'scope> {
    /// Terminal proof output.
    pub output: mfm_program::Handle<'program, 'scope, ProofOutput>,
}

/// Read state that records a typed proof fact.
pub struct ProofReadFactState {
    config: ProofReadConfig,
}

impl StateSpec for ProofReadFactState {
    type Config = ProofReadConfig;
    type Input = ();
    type Output = ProofFact;
    type Effect = ReadExternal;
    type Caps = (ProofReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("read_fact")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("read_fact")
    }

    fn name() -> &'static str {
        "mfm.proof.read_fact"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for ProofReadFactState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(&'a self, _input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
        future::ready(Ok(ProofFact {
            n: self.config.fact_n,
        }))
    }
}

/// Side-effect state that applies a proof mutation through typed intent and receipt contracts.
pub struct ProofApplySideEffectState {
    config: ProofApplyConfig,
}

impl StateSpec for ProofApplySideEffectState {
    type Config = ProofApplyConfig;
    type Input = ProofFact;
    type Output = ProofSideEffectResult;
    type Effect = ApplySideEffect;
    type Caps = (ProofMutationCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("apply_side_effect")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("apply_side_effect")
    }

    fn name() -> &'static str {
        "mfm.proof.apply_side_effect"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for ProofApplySideEffectState {
    type Intent = ProofIntent;
    type IdempotencyInput = ProofIdempotencyInput;
    type Submission = ProofSubmission;
    type Receipt = ProofReceipt;
    type Confirmation = ProofConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent> {
        Ok(ProofIntent {
            fact_n: input.n,
            action: self.config.action.clone(),
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
    ) -> StateResult<Self::IdempotencyInput> {
        Ok(ProofIdempotencyInput {
            fact_n: intent.fact_n,
            action: intent.action.clone(),
        })
    }

    fn submit<'a>(
        &'a self,
        intent: &'a Self::Intent,
        key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Ok(ProofSubmission {
            submission_id: format!("proof-submission-{}-{}", intent.action, intent.fact_n),
            idempotency_digest: key.digest().as_str().to_owned(),
        }))
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
    ) -> StateResult<Self::Output> {
        Ok(ProofSideEffectResult {
            tx_hash: confirmation.tx_hash.clone(),
            confirmations: confirmation.confirmations,
            status: "confirmed".to_owned(),
        })
    }
}

/// Pure state that assembles the proof public output value.
pub struct ProofAssembleOutputState;

impl StateSpec for ProofAssembleOutputState {
    type Config = ProofAssembleConfig;
    type Input = ProofAssembleInput;
    type Output = ProofOutput;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("assemble_output")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("assemble_output")
    }

    fn name() -> &'static str {
        "mfm.proof.assemble_output"
    }

    fn new(_config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for ProofAssembleOutputState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(ProofOutput {
            fact: input.fact,
            side_effect: input.side_effect,
        })
    }
}

/// Facts recorded by a proof implementation and supplied to replay verifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedProofFacts {
    /// Read fact observed by the proof workflow.
    pub fact: ProofFact,
}

/// No-live-IO replay verifier contract for proof implementations.
pub trait ProofReplayVerifier: Send + Sync {
    /// Verifies recorded submission evidence.
    fn verify_submission(
        &self,
        intent: &ProofIntent,
        submission: &ProofSubmission,
        facts: &RecordedProofFacts,
    ) -> Result<(), ProofReplayError>;

    /// Verifies recorded receipt evidence.
    fn verify_receipt(
        &self,
        intent: &ProofIntent,
        receipt: &ProofReceipt,
        facts: &RecordedProofFacts,
    ) -> Result<(), ProofReplayError>;

    /// Verifies recorded confirmation evidence.
    fn verify_confirmation(
        &self,
        receipt: &ProofReceipt,
        confirmation: &ProofConfirmation,
        facts: &RecordedProofFacts,
    ) -> Result<(), ProofReplayError>;
}

/// Proof replay verification error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofReplayError {
    /// Redaction-safe message.
    pub message: String,
}

impl ProofReplayError {
    /// Creates a replay error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProofReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProofReplayError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_apply_config_rejects_empty_actions_at_construction() {
        assert!(ProofApplyConfig::new("accept").is_ok());
        assert!(ProofApplyConfig::new("   ").is_err());
    }

    #[test]
    fn proof_workflow_config_has_distinct_canonical_shape_from_runtime_intent() {
        let config = ProofWorkflowConfig::default();
        let intent = ProofIntent {
            fact_n: config.read.fact_n,
            action: config.apply.action().to_owned(),
        };

        let config_json = serde_json::to_string(&config).expect("config json");
        let intent_json = serde_json::to_string(&intent).expect("intent json");
        let config_bytes =
            mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&config_json).expect("config");
        let intent_bytes =
            mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&intent_json).expect("intent");

        assert_ne!(config_bytes.as_bytes(), intent_bytes.as_bytes());
        assert_ne!(config_bytes.content_digest(), intent_bytes.content_digest());
    }
}
