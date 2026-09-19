#![warn(missing_docs)]
//! Typed deterministic authoring and checked immutable linear Program contracts.
//!
//! A Program owns its checked control document, exact value contracts and mandatory executable
//! occurrences. Typed source traversal performs deterministic construction without IO; cold loading
//! discovers installed source types without fresh planning. Runtime invokes the already-bound
//! callbacks and retains all continuation, acknowledgement and recovery authority.
//!
//! See the [capability authoring guide](https://github.com/willyrgf/mfm/blob/main/docs/capability-authoring.md)
//! for consumer, State and native implementation responsibilities.

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

/// Internal execution boundary for the kernel; not an authoring or registration API.
#[doc(hidden)]
pub mod callback;

mod construction;
mod native;
mod native_abi;
pub use native::*;
pub use native_abi::{effect_implementation_ref, read_implementation_ref, NativeAbi};
#[doc(hidden)]
pub mod executable;
mod recovery;
mod typed_source;
pub use construction::{compile, components, load, Component, ComponentKind, ProgramEnvironment};
pub use recovery::{
    Classification, ClassifyError, ExecutionPhase, Handler, HandlerAbi, HandlerBinding, NoParams,
    PolicyParams, ProgramLimits, RecoveryAllowances, RecoveryContext, RecoveryDenial,
    RecoveryLimit, RecoveryRequest, RecoveryTarget, RecoveryUsage, StandardRecovery, Stop,
    StopReason,
};
pub use typed_source::*;

#[cfg(test)]
mod tests;

const MAX_STATES: usize = 65_535;

/// Result type for Program construction and decoding.
pub type Result<T> = std::result::Result<T, ProgramError>;

/// Redaction-safe Program failure.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ProgramError {
    /// Reviewed immutable construction or binding cause at its originating operation.
    #[error("program construction failed")]
    Diagnostic(mfm_values::InvocationDiagnostic),
    /// A checked implementation identity failed, retaining its grammar and rejection reason.
    #[error("program implementation identity is invalid")]
    Identity(#[from] mfm_ids::CheckedStringError),
    /// Exact value construction or schema derivation cause.
    #[error("program value construction failed")]
    Value(#[from] mfm_values::ValueError),
    /// Exact content/schema identity construction cause.
    #[error("program schema identity failed")]
    SchemaIdentity(#[from] mfm_ids::IdentityError),
    /// Exact capability identity cause.
    #[error("program capability identity failed")]
    Capability(#[from] mfm_capabilities::CapabilityError),
    /// Canonical encoder or grammar cause.
    #[error("program canonical encoding failed")]
    Encoding(#[from] mfm_canonical::CanonicalError),
    /// Stored Program decoding category, location and rejection reason.
    #[error("program JSON decoding failed")]
    Json(#[from] mfm_canonical::JsonError),
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

    /// Non-semantic description for installed-code inspection.
    fn description() -> &'static str {
        ""
    }
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
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_values::InvocationDiagnostic,
    >;
}

/// Deterministic State behavior driven by typed Read evidence.
pub trait ReadState<C>: State
where
    C: ReadCapabilityContract,
{
    /// Prepares the exact adapter intent.
    fn prepare(
        input: &Self::Input,
    ) -> std::result::Result<C::Intent, mfm_values::InvocationDiagnostic>;

    /// Interprets accepted evidence.
    fn interpret(
        input: Self::Input,
        evidence: &C::Evidence,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_values::InvocationDiagnostic,
    >;
}

/// Deterministic State behavior driven by typed Effect evidence.
pub trait EffectState<C>: State
where
    C: EffectCapabilityContract,
{
    /// Prepares the exact adapter command.
    fn prepare(
        input: &Self::Input,
    ) -> std::result::Result<C::Command, mfm_values::InvocationDiagnostic>;

    /// Interprets accepted evidence.
    fn interpret(
        input: Self::Input,
        evidence: &C::Evidence,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_values::InvocationDiagnostic,
    >;
}

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
        .map_err(mfm_values::ValueError::Identity)
    }
}

/// Derives the retained v1 implementation reference for a State.
pub fn state_implementation_ref<S: State>() -> Result<ContentRef> {
    implementation_ref("mfm.state-implementation", S::state_id()?)
}

/// Derives the exact Read capability identity, including its semantic request and evidence contracts.
pub fn capability_contract_ref<C: ReadCapabilityContract>() -> Result<ContentRef> {
    capability_ref(
        "read",
        C::contract_id()?,
        nominal_contract_ref::<C::Intent>()?,
        nominal_contract_ref::<C::Evidence>()?,
    )
}

/// Derives the exact Effect capability identity, including its semantic request and evidence contracts.
pub fn effect_capability_contract_ref<C: EffectCapabilityContract>() -> Result<ContentRef> {
    capability_ref(
        "effect",
        C::contract_id()?,
        nominal_contract_ref::<C::Command>()?,
        nominal_contract_ref::<C::Evidence>()?,
    )
}

fn capability_ref(
    mode: &str,
    implementation: StableId,
    request: ContentRef,
    evidence: ContentRef,
) -> Result<ContentRef> {
    #[derive(Serialize)]
    struct Contract<'a> {
        domain: &'static str,
        mode: &'a str,
        implementation: StableId,
        request: ContentRef,
        evidence: ContentRef,
    }
    let json = serde_json::to_string(&Contract {
        domain: "mfm.capability-contract.v3",
        mode,
        implementation,
        request,
        evidence,
    })
    .map_err(mfm_canonical::JsonError::new)?;
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)?;
    let schema = SchemaId::new(
        "mfm.capability-contract",
        "3",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )?;
    Ok(ContentRef::new(
        schema,
        raw_content_digest(canonical.as_bytes()),
    )?)
}

fn implementation_ref(schema_name: &str, id: StableId) -> Result<ContentRef> {
    let schema = SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )?;
    Ok(ContentRef::new(
        schema,
        raw_content_digest(id.as_str().as_bytes()),
    )?)
}

/// Derives the nominal contract reference for a typed value.
pub fn nominal_contract_ref<T: MfmValue>() -> Result<ContentRef> {
    derive_nominal_contract::<T>().map(|(contract_ref, _)| contract_ref)
}

pub(crate) fn derive_nominal_contract<T: MfmValue>() -> Result<(ContentRef, SchemaDescriptor)> {
    let descriptor = T::schema_descriptor()?;
    let schema = descriptor.identity().schema_id()?;
    let semantic = T::semantic_id()?;
    if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic) {
        return Err(ProgramError::InvalidContract);
    }
    let contract_ref = ContentRef::new(schema, raw_content_digest(b"mfm.contract.v1"))?;
    Ok((contract_ref, descriptor))
}

mod program;
pub use program::{Execution, Program, StateDeclaration};
