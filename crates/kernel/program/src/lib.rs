#![warn(missing_docs)]
//! Typed deterministic authoring and checked immutable linear Program contracts.
//!
//! A Program is the sole persisted control document. Runtime associates its
//! immutable declarations with typed implementations. Authoring callbacks and
//! capability injection are erased before construction; this crate performs no IO.
//! Operation implementations compose children only through `OperationExpansion`, and capability
//! policies use typed `OperationExpansion` scopes for their before and after sequences. Direct trait callback calls bypass
//! kernel callback accounting and are forbidden in reviewed production code; checked Program
//! construction, not this trusted-code rule, remains the persisted definition boundary.

#[cfg(test)]
extern crate self as mfm_program;

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_capabilities::{EffectCapabilityContract, ReadCapabilityContract};
use mfm_ids::{
    ContentRef, DigestAlgorithm, DigestBytes, EntryPointId, SchemaId, SchemaVersion,
    SemanticTypeId, StableId,
};
use mfm_values::{
    CanonicalJsonProfile, EnumTagging, MfmValue, SchemaDescriptor, SchemaIdentity, SchemaKind,
    SchemaShape, MAX_RUN_OBJECT_CANONICAL_BYTES,
};
use serde::{Deserialize, Serialize};

mod authoring;
mod recovery;
pub use recovery::{
    Checkpoint, Classification, ClassifyError, ConclusionBound, EffectBounds, ExecutionPhase,
    FromNever, Handler, HandlerAbi, HandlerBinding, HistoryBound, Identity, Incident,
    IncidentSource, IncidentSummary, MapAbi, MapBinding, NoContext, NoParams, Occurrence,
    PolicyParams, ProgramLimits, RecoveryAllowances, RecoveryContext, RecoveryDenial,
    RecoveryLimit, RecoveryRequest, RecoveryTarget, RecoveryUsage, StandardRecovery, Stop,
    StopReason, ValueMap,
};

#[cfg(test)]
mod tests;

pub use authoring::{expand_program, CapabilityInjection, Operation, OperationExpansion};

const MAX_STATES: usize = 65_535;

/// Result type for Program construction and decoding.
pub type Result<T> = std::result::Result<T, ProgramError>;

/// Redaction-safe Program failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProgramError {
    /// Canonical bytes were malformed, noncanonical, or used an unknown wire.
    #[error("program canonical bytes are invalid")]
    Canonical,
    /// A declaration, identity, or graph contract was invalid.
    #[error("program contract is invalid")]
    InvalidContract,
    /// A fixed Program size or count ceiling was exceeded.
    #[error("program capacity exceeded")]
    Capacity,
}

/// One typed State implementation contract.
pub trait State: Send + Sync + 'static {
    /// Complete input value.
    type Input: MfmValue;
    /// Success value.
    type Output: MfmValue;
    /// Original failure value with intrinsic deterministic recovery semantics.
    type Failure: ClassifyError;

    /// Returns the stable implementation identity.
    fn state_id() -> Result<StableId>;
}

/// The only proposed typed State outcomes.
#[derive(Debug, PartialEq, Eq)]
pub enum ProposedStateOutcome<O, F> {
    /// State evaluation succeeded.
    Success {
        /// Complete successor or terminal output.
        output: O,
    },
    /// State evaluation returned a domain failure.
    Failure {
        /// Complete typed domain failure.
        failure: F,
    },
}

/// Deterministic State behavior without IO.
pub trait PureState: State {
    /// Evaluates one complete input.
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, StateExecutionError>;
}

/// Deterministic State behavior driven by typed Read evidence.
pub trait ReadState<C>: State
where
    C: ReadCapabilityContract,
{
    /// Deterministic context for this State's operational adapter incidents.
    type AdapterContext: MfmValue;
    /// Explains a capability failure without replacing its original cause.
    fn adapter_context(
        input: &Self::Input,
        intent: &C::Intent,
        error: &C::OperationalError,
    ) -> std::result::Result<Self::AdapterContext, StateExecutionError>;
    /// Prepares the exact adapter intent.
    fn prepare(input: &Self::Input) -> std::result::Result<C::Intent, PreparationError>;

    /// Interprets accepted evidence.
    fn interpret(
        input: Self::Input,
        evidence: &C::Evidence,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, StateExecutionError>;
}

/// Deterministic State behavior driven by typed Effect evidence.
pub trait EffectState<C>: State
where
    C: EffectCapabilityContract,
{
    /// Deterministic context for unresolved Effect adapter incidents.
    type AdapterContext: MfmValue;
    /// Explains the retained command's adapter failure without settling the Effect.
    fn adapter_context(
        input: &Self::Input,
        command: &C::Command,
        error: &C::OperationalError,
    ) -> std::result::Result<Self::AdapterContext, StateExecutionError>;
    /// Prepares the exact adapter command.
    fn prepare(input: &Self::Input) -> std::result::Result<C::Command, PreparationError>;

    /// Interprets accepted evidence.
    fn interpret(
        input: Self::Input,
        evidence: &C::Evidence,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, StateExecutionError>;
}

/// Redaction-safe failure of trusted deterministic capability preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("capability preparation failed")]
pub struct PreparationError;

/// Redaction-safe failure of trusted deterministic State execution.
///
/// This is an internal implementation error, not a durable domain outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("state execution failed")]
pub struct StateExecutionError;

/// Uninhabited failure value owned by the framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Never {}

impl Serialize for Never {
    fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match *self {}
    }
}

impl<'de> Deserialize<'de> for Never {
    fn deserialize<D>(_deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom("Never has no values"))
    }
}

impl MfmValue for Never {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        mfm_values::framework_value_descriptor(
            "mfm-program",
            Self::semantic_id()?,
            "mfm.kernel.never",
            SchemaShape::Enum {
                tagging: EnumTagging::External,
                variants: Vec::new(),
            },
            "mfm_program::Never",
        )
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.kernel",
            "never",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([
                0x52, 0x7c, 0x19, 0x8c, 0xa1, 0xe6, 0x17, 0x0d, 0xdf, 0x89, 0x66, 0xdd, 0x86, 0xeb,
                0xd4, 0x18, 0xb6, 0x22, 0x26, 0xf1, 0x4f, 0x6e, 0xf3, 0x4d, 0x7b, 0x7d, 0xdd, 0x7b,
                0x91, 0xa3, 0x68, 0x35,
            ]),
        )
        .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

/// Derives the retained v1 implementation reference for a State.
pub fn state_implementation_ref<S: State>() -> Result<ContentRef> {
    implementation_ref(
        "mfm.state-implementation",
        S::state_id().map_err(|_| ProgramError::InvalidContract)?,
    )
}

/// Derives the exact Read capability identity, including its operational-error contract.
pub fn capability_contract_ref<C: ReadCapabilityContract>() -> Result<ContentRef> {
    capability_ref(
        "read",
        C::contract_id().map_err(|_| ProgramError::InvalidContract)?,
        nominal_contract_ref::<C::Intent>()?,
        nominal_contract_ref::<C::Evidence>()?,
        nominal_contract_ref::<C::OperationalError>()?,
    )
}

/// Derives the exact Effect capability identity, including its operational-error contract.
pub fn effect_capability_contract_ref<C: EffectCapabilityContract>() -> Result<ContentRef> {
    capability_ref(
        "effect",
        C::contract_id().map_err(|_| ProgramError::InvalidContract)?,
        nominal_contract_ref::<C::Command>()?,
        nominal_contract_ref::<C::Evidence>()?,
        nominal_contract_ref::<C::OperationalError>()?,
    )
}

fn capability_ref(
    mode: &str,
    implementation: StableId,
    request: ContentRef,
    evidence: ContentRef,
    operational_error: ContentRef,
) -> Result<ContentRef> {
    #[derive(Serialize)]
    struct Contract<'a> {
        domain: &'static str,
        mode: &'a str,
        implementation: StableId,
        request: ContentRef,
        evidence: ContentRef,
        operational_error: ContentRef,
    }
    let json = serde_json::to_string(&Contract {
        domain: "mfm.capability-contract.v2",
        mode,
        implementation,
        request,
        evidence,
        operational_error,
    })
    .map_err(|_| ProgramError::Canonical)?;
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| ProgramError::Canonical)?;
    let schema = SchemaId::new(
        "mfm.capability-contract",
        "2",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| ProgramError::InvalidContract)?;
    ContentRef::new(schema, raw_content_digest(canonical.as_bytes()))
        .map_err(|_| ProgramError::InvalidContract)
}

fn implementation_ref(schema_name: &str, id: StableId) -> Result<ContentRef> {
    let schema = SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| ProgramError::InvalidContract)?;
    ContentRef::new(schema, raw_content_digest(id.as_str().as_bytes()))
        .map_err(|_| ProgramError::InvalidContract)
}

/// Derives the nominal contract reference for a typed value.
pub fn nominal_contract_ref<T: MfmValue>() -> Result<ContentRef> {
    derive_nominal_contract::<T>().map(|(contract_ref, _)| contract_ref)
}

pub(crate) fn derive_nominal_contract<T: MfmValue>() -> Result<(ContentRef, SchemaDescriptor)> {
    let descriptor = T::schema_descriptor().map_err(|_| ProgramError::InvalidContract)?;
    let schema = descriptor
        .identity()
        .schema_id()
        .map_err(|_| ProgramError::InvalidContract)?;
    let semantic = T::semantic_id().map_err(|_| ProgramError::InvalidContract)?;
    if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic) {
        return Err(ProgramError::InvalidContract);
    }
    let contract_ref = ContentRef::new(schema, raw_content_digest(b"mfm.contract.v1"))
        .map_err(|_| ProgramError::InvalidContract)?;
    Ok((contract_ref, descriptor))
}

mod program;
pub use program::{Execution, Program, StateDeclaration};
