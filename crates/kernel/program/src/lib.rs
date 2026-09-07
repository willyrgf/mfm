#![warn(missing_docs)]
//! Typed deterministic authoring and checked immutable Program v3 contracts.
//!
//! A Program is the sole persisted control document. Runtime associates its
//! immutable declarations with typed implementations. Authoring callbacks and
//! capability injection are erased before construction; this crate performs no IO.
//! Operation implementations compose children only through `OperationExpansion`, and capability
//! policies use typed `OperationExpansion` scopes for their before and after graphs. Direct trait callback calls bypass
//! kernel callback accounting and are forbidden in reviewed production code; checked Program
//! construction, not this trusted-code rule, remains the persisted graph boundary.

#[cfg(test)]
extern crate self as mfm_program;

use std::collections::BTreeSet;

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

#[cfg(test)]
mod tests;

pub use authoring::{
    expand_program, CapabilityInjection, MatchJoin, Operation, OperationExpansion,
};

const MAX_DECLARATIONS: usize = 65_536;
const MAX_STATES: usize = 65_535;
const MAX_MATCH_ARMS: usize = 256;
const MAX_PROGRAM_FRAME_WEIGHT: u64 = 65_536;

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
    /// Failure value.
    type Failure: MfmValue;

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

/// Derives the retained v1 contract reference for a Read capability.
pub fn capability_contract_ref<C: ReadCapabilityContract>() -> Result<ContentRef> {
    implementation_ref(
        "mfm.capability-contract",
        C::contract_id().map_err(|_| ProgramError::InvalidContract)?,
    )
}

/// Derives the retained v1 contract reference for an Effect capability.
pub fn effect_capability_contract_ref<C: EffectCapabilityContract>() -> Result<ContentRef> {
    implementation_ref(
        "mfm.capability-contract",
        C::contract_id().map_err(|_| ProgramError::InvalidContract)?,
    )
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

/// One closed Pure, Read, or Effect execution declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Execution {
    /// Deterministic execution without a capability.
    Pure,
    /// Observational execution against one exact capability and binding.
    Read {
        /// Capability contract.
        capability_contract_ref: ContentRef,
        /// Prepared intent contract.
        intent_contract_ref: ContentRef,
        /// Returned evidence contract.
        evidence_contract_ref: ContentRef,
        /// Immutable adapter binding.
        binding_ref: ContentRef,
    },
    /// Mutating execution against one exact capability and binding.
    Effect {
        /// Capability contract.
        capability_contract_ref: ContentRef,
        /// Prepared command contract.
        command_contract_ref: ContentRef,
        /// Returned evidence contract.
        evidence_contract_ref: ContentRef,
        /// Immutable adapter binding.
        binding_ref: ContentRef,
    },
}

impl Execution {
    /// Constructs Pure execution.
    pub(crate) fn pure() -> Self {
        Self::Pure
    }

    /// Constructs Read execution with its complete static ABI and binding.
    pub(crate) fn read(
        capability_contract_ref: ContentRef,
        intent_contract_ref: ContentRef,
        evidence_contract_ref: ContentRef,
        binding_ref: ContentRef,
    ) -> Self {
        Self::Read {
            capability_contract_ref,
            intent_contract_ref,
            evidence_contract_ref,
            binding_ref,
        }
    }

    /// Constructs Effect execution with its complete static ABI and binding.
    pub(crate) fn effect(
        capability_contract_ref: ContentRef,
        command_contract_ref: ContentRef,
        evidence_contract_ref: ContentRef,
        binding_ref: ContentRef,
    ) -> Self {
        Self::Effect {
            capability_contract_ref,
            command_contract_ref,
            evidence_contract_ref,
            binding_ref,
        }
    }
}

/// One checked State occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDeclaration {
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    execution: Execution,
    next_index: Option<u16>,
    failure_next_index: Option<u16>,
}

impl StateDeclaration {
    /// Constructs one State declaration.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        state_implementation_ref: ContentRef,
        input_contract_ref: ContentRef,
        output_contract_ref: ContentRef,
        failure_contract_ref: ContentRef,
        execution: Execution,
        next_index: Option<u16>,
        failure_next_index: Option<u16>,
    ) -> Self {
        Self {
            state_implementation_ref,
            input_contract_ref,
            output_contract_ref,
            failure_contract_ref,
            execution,
            next_index,
            failure_next_index,
        }
    }

    /// Returns the State implementation reference.
    pub const fn state_implementation_ref(&self) -> &ContentRef {
        &self.state_implementation_ref
    }

    /// Returns the input contract.
    pub const fn input_contract_ref(&self) -> &ContentRef {
        &self.input_contract_ref
    }

    /// Returns the output contract.
    pub const fn output_contract_ref(&self) -> &ContentRef {
        &self.output_contract_ref
    }

    /// Returns the failure contract.
    pub const fn failure_contract_ref(&self) -> &ContentRef {
        &self.failure_contract_ref
    }

    /// Returns the execution declaration.
    pub const fn execution(&self) -> &Execution {
        &self.execution
    }

    /// Returns the success successor.
    pub const fn next_index(&self) -> Option<u16> {
        self.next_index
    }

    /// Returns the failure successor.
    pub const fn failure_next_index(&self) -> Option<u16> {
        self.failure_next_index
    }
}

/// One Match tag and target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchVariant {
    tag: StableId,
    entry_index: u16,
}

impl MatchVariant {
    /// Constructs one Match arm.
    pub(crate) fn new(tag: StableId, entry_index: u16) -> Self {
        Self { tag, entry_index }
    }

    /// Returns the exact raw tag.
    pub const fn tag(&self) -> &StableId {
        &self.tag
    }

    /// Returns the target State index.
    pub const fn entry_index(&self) -> u16 {
        self.entry_index
    }
}

/// One deterministic closed-sum selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchDeclaration {
    selector_contract_ref: ContentRef,
    variants: Vec<MatchVariant>,
}

impl MatchDeclaration {
    /// Constructs one selector, sorting arms by raw tag bytes.
    pub(crate) fn new(
        selector_contract_ref: ContentRef,
        mut variants: Vec<MatchVariant>,
    ) -> Result<Self> {
        if variants.is_empty() {
            return Err(ProgramError::InvalidContract);
        }
        if variants.len() > MAX_MATCH_ARMS {
            return Err(ProgramError::Capacity);
        }
        variants.sort_by(|left, right| {
            left.tag
                .as_str()
                .as_bytes()
                .cmp(right.tag.as_str().as_bytes())
        });
        if variants.windows(2).any(|pair| pair[0].tag == pair[1].tag) {
            return Err(ProgramError::InvalidContract);
        }
        Ok(Self {
            selector_contract_ref,
            variants,
        })
    }

    /// Returns the selector value contract.
    pub const fn selector_contract_ref(&self) -> &ContentRef {
        &self.selector_contract_ref
    }

    /// Returns the sorted selector arms.
    pub fn variants(&self) -> &[MatchVariant] {
        &self.variants
    }
}

/// The only visible Program declaration variants.
#[derive(Debug, Clone, PartialEq, Eq)]
// The frozen authoring API owns declarations by value; boxing a public variant would change it.
#[allow(clippy::large_enum_variant)]
pub enum Declaration {
    /// One executable State occurrence.
    State(StateDeclaration),
    /// One deterministic selector.
    Match(MatchDeclaration),
}

/// One strict immutable Program document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    entry_point_id: EntryPointId,
    admitted_context_contract_ref: ContentRef,
    root_success_contract_ref: ContentRef,
    root_failure_contract_ref: ContentRef,
    declarations: Vec<Declaration>,
    canonical_bytes: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
}

impl Program {
    /// Constructs and validates one Program.
    pub(crate) fn new(
        entry_point_id: EntryPointId,
        admitted_context_contract_ref: ContentRef,
        root_success_contract_ref: ContentRef,
        root_failure_contract_ref: ContentRef,
        declarations: Vec<Declaration>,
    ) -> Result<Self> {
        validate_program(
            &admitted_context_contract_ref,
            &root_success_contract_ref,
            &root_failure_contract_ref,
            &declarations,
        )?;
        let wire = ProgramWire::from_parts(
            &entry_point_id,
            &admitted_context_contract_ref,
            &root_success_contract_ref,
            &root_failure_contract_ref,
            &declarations,
        );
        let bytes = encode_wire(&wire)?;
        Self::from_validated(
            entry_point_id,
            admitted_context_contract_ref,
            root_success_contract_ref,
            root_failure_contract_ref,
            declarations,
            bytes,
        )
    }

    /// Strictly decodes and validates canonical Program bytes.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
            return Err(ProgramError::Capacity);
        }
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ProgramError::Canonical)?;
        let wire: ProgramWire =
            serde_json::from_slice(canonical.as_bytes()).map_err(|_| ProgramError::Canonical)?;
        let (entry_point_id, admitted, success, failure, declarations) = wire.try_checked()?;
        validate_program(&admitted, &success, &failure, &declarations)?;
        let wire = ProgramWire::from_parts(
            &entry_point_id,
            &admitted,
            &success,
            &failure,
            &declarations,
        );
        let reencoded = encode_wire(&wire)?;
        if reencoded.as_bytes() != bytes {
            return Err(ProgramError::Canonical);
        }
        Self::from_validated(
            entry_point_id,
            admitted,
            success,
            failure,
            declarations,
            reencoded,
        )
    }

    fn from_validated(
        entry_point_id: EntryPointId,
        admitted_context_contract_ref: ContentRef,
        root_success_contract_ref: ContentRef,
        root_failure_contract_ref: ContentRef,
        declarations: Vec<Declaration>,
        canonical_bytes: PlainCanonicalJsonBytes,
    ) -> Result<Self> {
        if canonical_bytes.as_bytes().len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
            return Err(ProgramError::Capacity);
        }
        let content_ref = ContentRef::new(
            program_schema_id()?,
            raw_content_digest(canonical_bytes.as_bytes()),
        )
        .map_err(|_| ProgramError::InvalidContract)?;
        Ok(Self {
            entry_point_id,
            admitted_context_contract_ref,
            root_success_contract_ref,
            root_failure_contract_ref,
            declarations,
            canonical_bytes,
            content_ref,
        })
    }

    /// Returns the public dispatch identity.
    pub const fn entry_point_id(&self) -> &EntryPointId {
        &self.entry_point_id
    }

    /// Returns the admitted C0 contract.
    pub const fn admitted_context_contract_ref(&self) -> &ContentRef {
        &self.admitted_context_contract_ref
    }

    /// Returns the terminal success contract.
    pub const fn root_success_contract_ref(&self) -> &ContentRef {
        &self.root_success_contract_ref
    }

    /// Returns the terminal failure contract.
    pub const fn root_failure_contract_ref(&self) -> &ContentRef {
        &self.root_failure_contract_ref
    }

    /// Returns declarations in identity-bearing execution order.
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    /// Returns exact canonical Program bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical_bytes.as_bytes()
    }

    /// Returns the content identity of the exact Program bytes.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
}

fn program_schema_id() -> Result<SchemaId> {
    SchemaIdentity::new(
        SchemaKind::PersistedContract,
        None,
        "mfm-program-document",
        SchemaVersion::new("3").map_err(|_| ProgramError::InvalidContract)?,
        SchemaShape::CanonicalJsonTerminal {
            profile: CanonicalJsonProfile::GeneralFloatFree,
        },
    )
    .and_then(|identity| identity.schema_id())
    .map_err(|_| ProgramError::InvalidContract)
}

fn never_ref() -> Result<ContentRef> {
    nominal_contract_ref::<Never>()
}

fn validate_program(
    admitted: &ContentRef,
    success: &ContentRef,
    failure: &ContentRef,
    declarations: &[Declaration],
) -> Result<()> {
    if declarations.len() > MAX_DECLARATIONS {
        return Err(ProgramError::Capacity);
    }
    let state_count = declarations
        .iter()
        .filter(|declaration| matches!(declaration, Declaration::State(_)))
        .count();
    if state_count > MAX_STATES {
        return Err(ProgramError::Capacity);
    }
    let frame_weight = declarations.iter().try_fold(1_u64, |weight, declaration| {
        let declaration_weight = match declaration {
            Declaration::State(state) => match state.execution() {
                Execution::Pure | Execution::Read { .. } => 1,
                Execution::Effect { .. } => 2,
            },
            Declaration::Match(_) => 0,
        };
        weight
            .checked_add(declaration_weight)
            .ok_or(ProgramError::Capacity)
    })?;
    if frame_weight > MAX_PROGRAM_FRAME_WEIGHT {
        return Err(ProgramError::Capacity);
    }
    let never = never_ref()?;
    if admitted == &never || success == &never {
        return Err(ProgramError::InvalidContract);
    }
    if declarations.is_empty() {
        return (admitted == success && failure == &never)
            .then_some(())
            .ok_or(ProgramError::InvalidContract);
    }

    let entry_contract = match &declarations[0] {
        Declaration::State(state) => state.input_contract_ref(),
        Declaration::Match(selector) => selector.selector_contract_ref(),
    };
    if entry_contract != admitted {
        return Err(ProgramError::InvalidContract);
    }

    let mut reachable = BTreeSet::from([0usize]);
    for (index, declaration) in declarations.iter().enumerate() {
        match declaration {
            Declaration::State(state) => {
                if state.input_contract_ref() == &never || state.output_contract_ref() == &never {
                    return Err(ProgramError::InvalidContract);
                }
                validate_success_edge(index, state, success, declarations, &mut reachable)?;
                validate_failure_edge(index, state, failure, &never, declarations, &mut reachable)?;
            }
            Declaration::Match(selector) => {
                if selector.selector_contract_ref() == &never
                    || selector.variants().is_empty()
                    || selector.variants().len() > MAX_MATCH_ARMS
                    || !selector.variants().windows(2).all(|pair| {
                        pair[0].tag().as_str().as_bytes() < pair[1].tag().as_str().as_bytes()
                    })
                {
                    return Err(ProgramError::InvalidContract);
                }
                for variant in selector.variants() {
                    let target = checked_target(index, variant.entry_index(), declarations)?;
                    if !matches!(target, Declaration::State(_)) {
                        return Err(ProgramError::InvalidContract);
                    }
                    reachable.insert(usize::from(variant.entry_index()));
                }
            }
        }
    }
    if reachable.len() != declarations.len() {
        return Err(ProgramError::InvalidContract);
    }
    Ok(())
}

fn validate_success_edge(
    index: usize,
    state: &StateDeclaration,
    root_success: &ContentRef,
    declarations: &[Declaration],
    reachable: &mut BTreeSet<usize>,
) -> Result<()> {
    match state.next_index() {
        Some(next) => {
            let target = checked_target(index, next, declarations)?;
            let target_contract = match target {
                Declaration::State(target) => target.input_contract_ref(),
                Declaration::Match(target) => target.selector_contract_ref(),
            };
            if target_contract != state.output_contract_ref() {
                return Err(ProgramError::InvalidContract);
            }
            reachable.insert(usize::from(next));
        }
        None if state.output_contract_ref() == root_success => {}
        None => return Err(ProgramError::InvalidContract),
    }
    Ok(())
}

fn validate_failure_edge(
    index: usize,
    state: &StateDeclaration,
    root_failure: &ContentRef,
    never: &ContentRef,
    declarations: &[Declaration],
    reachable: &mut BTreeSet<usize>,
) -> Result<()> {
    match state.failure_next_index() {
        Some(_) if state.failure_contract_ref() == never => Err(ProgramError::InvalidContract),
        Some(next) => {
            let Declaration::State(target) = checked_target(index, next, declarations)? else {
                return Err(ProgramError::InvalidContract);
            };
            if target.input_contract_ref() != state.failure_contract_ref() {
                return Err(ProgramError::InvalidContract);
            }
            reachable.insert(usize::from(next));
            Ok(())
        }
        None if state.failure_contract_ref() == never
            || state.failure_contract_ref() == root_failure =>
        {
            Ok(())
        }
        None => Err(ProgramError::InvalidContract),
    }
}

fn checked_target(
    source_index: usize,
    target_index: u16,
    declarations: &[Declaration],
) -> Result<&Declaration> {
    let target_index = usize::from(target_index);
    if target_index <= source_index {
        return Err(ProgramError::InvalidContract);
    }
    declarations
        .get(target_index)
        .ok_or(ProgramError::InvalidContract)
}

fn encode_wire(wire: &ProgramWire) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(wire).map_err(|_| ProgramError::Canonical)?;
    let bytes =
        PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| ProgramError::Canonical)?;
    if bytes.as_bytes().len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
        return Err(ProgramError::Capacity);
    }
    Ok(bytes)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgramWire {
    domain: String,
    entry_point_id: EntryPointId,
    admitted_context_contract_ref: ContentRef,
    root_success_contract_ref: ContentRef,
    root_failure_contract_ref: ContentRef,
    declarations: Vec<DeclarationWire>,
}

impl ProgramWire {
    fn from_parts(
        entry_point_id: &EntryPointId,
        admitted: &ContentRef,
        success: &ContentRef,
        failure: &ContentRef,
        declarations: &[Declaration],
    ) -> Self {
        Self {
            domain: "mfm.program.v3".to_owned(),
            entry_point_id: entry_point_id.clone(),
            admitted_context_contract_ref: admitted.clone(),
            root_success_contract_ref: success.clone(),
            root_failure_contract_ref: failure.clone(),
            declarations: declarations.iter().map(DeclarationWire::from).collect(),
        }
    }

    fn try_checked(
        self,
    ) -> Result<(
        EntryPointId,
        ContentRef,
        ContentRef,
        ContentRef,
        Vec<Declaration>,
    )> {
        if self.domain != "mfm.program.v3" {
            return Err(ProgramError::InvalidContract);
        }
        let declarations = self
            .declarations
            .into_iter()
            .map(DeclarationWire::try_checked)
            .collect::<Result<Vec<_>>>()?;
        Ok((
            self.entry_point_id,
            self.admitted_context_contract_ref,
            self.root_success_contract_ref,
            self.root_failure_contract_ref,
            declarations,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
enum DeclarationWire {
    State(Box<StateWire>),
    Match(MatchWire),
}

impl From<&Declaration> for DeclarationWire {
    fn from(value: &Declaration) -> Self {
        match value {
            Declaration::State(state) => Self::State(Box::new(StateWire::from(state))),
            Declaration::Match(selector) => Self::Match(MatchWire {
                selector_contract_ref: selector.selector_contract_ref().clone(),
                variants: selector
                    .variants()
                    .iter()
                    .map(|variant| MatchVariantWire {
                        tag: variant.tag().clone(),
                        entry_index: variant.entry_index(),
                    })
                    .collect(),
            }),
        }
    }
}

impl DeclarationWire {
    fn try_checked(self) -> Result<Declaration> {
        match self {
            Self::State(state) => Ok(Declaration::State(state.into_checked())),
            Self::Match(selector) => selector.try_checked().map(Declaration::Match),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    execution: ExecutionWire,
    next_index: RequiredOption<u16>,
    failure_next_index: RequiredOption<u16>,
}

impl From<&StateDeclaration> for StateWire {
    fn from(state: &StateDeclaration) -> Self {
        Self {
            state_implementation_ref: state.state_implementation_ref().clone(),
            input_contract_ref: state.input_contract_ref().clone(),
            output_contract_ref: state.output_contract_ref().clone(),
            failure_contract_ref: state.failure_contract_ref().clone(),
            execution: match &state.execution {
                Execution::Pure => ExecutionWire::Pure,
                Execution::Read {
                    capability_contract_ref,
                    intent_contract_ref,
                    evidence_contract_ref,
                    binding_ref,
                } => ExecutionWire::Read {
                    capability_contract_ref: capability_contract_ref.clone(),
                    intent_contract_ref: intent_contract_ref.clone(),
                    evidence_contract_ref: evidence_contract_ref.clone(),
                    binding_ref: binding_ref.clone(),
                },
                Execution::Effect {
                    capability_contract_ref,
                    command_contract_ref,
                    evidence_contract_ref,
                    binding_ref,
                } => ExecutionWire::Effect {
                    capability_contract_ref: capability_contract_ref.clone(),
                    command_contract_ref: command_contract_ref.clone(),
                    evidence_contract_ref: evidence_contract_ref.clone(),
                    binding_ref: binding_ref.clone(),
                },
            },
            next_index: RequiredOption(state.next_index()),
            failure_next_index: RequiredOption(state.failure_next_index()),
        }
    }
}

impl StateWire {
    fn into_checked(self) -> StateDeclaration {
        StateDeclaration::new(
            self.state_implementation_ref,
            self.input_contract_ref,
            self.output_contract_ref,
            self.failure_contract_ref,
            self.execution.into_execution(),
            self.next_index.0,
            self.failure_next_index.0,
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
enum ExecutionWire {
    Pure,
    Read {
        capability_contract_ref: ContentRef,
        intent_contract_ref: ContentRef,
        evidence_contract_ref: ContentRef,
        binding_ref: ContentRef,
    },
    Effect {
        capability_contract_ref: ContentRef,
        command_contract_ref: ContentRef,
        evidence_contract_ref: ContentRef,
        binding_ref: ContentRef,
    },
}

impl ExecutionWire {
    fn into_execution(self) -> Execution {
        match self {
            Self::Pure => Execution::pure(),
            Self::Read {
                capability_contract_ref,
                intent_contract_ref,
                evidence_contract_ref,
                binding_ref,
            } => Execution::read(
                capability_contract_ref,
                intent_contract_ref,
                evidence_contract_ref,
                binding_ref,
            ),
            Self::Effect {
                capability_contract_ref,
                command_contract_ref,
                evidence_contract_ref,
                binding_ref,
            } => Execution::effect(
                capability_contract_ref,
                command_contract_ref,
                evidence_contract_ref,
                binding_ref,
            ),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchWire {
    selector_contract_ref: ContentRef,
    variants: Vec<MatchVariantWire>,
}

impl MatchWire {
    fn try_checked(self) -> Result<MatchDeclaration> {
        let variants = self
            .variants
            .into_iter()
            .map(|variant| MatchVariant::new(variant.tag, variant.entry_index))
            .collect::<Vec<_>>();
        let checked = MatchDeclaration::new(self.selector_contract_ref, variants.clone())?;
        if checked.variants != variants {
            return Err(ProgramError::InvalidContract);
        }
        Ok(checked)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchVariantWire {
    tag: StableId,
    entry_index: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct RequiredOption<T>(Option<T>);
