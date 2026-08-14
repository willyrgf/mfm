//! Final callback-free Program and typed-ingress contracts.
//!
//! The visible declaration algebra is deliberately small: a sequential Program contains only
//! State declarations and deterministic Match selectors. Runtime never receives a generic
//! context map or a live callback from this module.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_capabilities::{AccessCapabilityContract, EffectMode, FactSelectionMode, ReadMode};
use mfm_facts::FactSelectionRequest;
pub use mfm_ids::SequentialControlAddress;
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId, SchemaVersion, StableId,
};
use mfm_values::{CanonicalJsonProfile, MfmValue, SchemaIdentity, SchemaKind, SchemaShape};
use serde::de;
use serde::{Deserialize, Serialize};

/// Maximum canonical Program document size.
pub const MAX_PROGRAM_BYTES: usize = 32 * 1024 * 1024;
/// Maximum sequential declarations in one Program.
pub const MAX_DECLARATIONS: usize = 65_536;
/// Maximum Match arms in one selector.
pub const MAX_MATCH_ARMS: usize = 256;
/// Maximum conclusion reservation accepted by one State declaration.
pub const MAX_STATE_CONCLUSION_BYTES: u64 = 32 * 1024 * 1024;

/// Returns the dedicated persisted schema identity for canonical Program documents.
///
/// Program documents are retained implementation contracts, not root result values. Keeping this
/// identity distinct from every State result contract prevents a document object from being
/// mistaken for a domain value solely because their content digests happen to share a hash
/// algorithm.
pub fn program_document_schema_id() -> Result<SchemaId> {
    let version = SchemaVersion::new("1").map_err(|_| ProgramError::InvalidContract)?;
    let identity = SchemaIdentity::new(
        SchemaKind::PersistedContract,
        None,
        "mfm-program-document",
        version,
        SchemaShape::CanonicalJsonTerminal {
            profile: CanonicalJsonProfile::GeneralFloatFree,
        },
    )
    .map_err(|_| ProgramError::InvalidContract)?;
    identity
        .schema_id()
        .map_err(|_| ProgramError::InvalidContract)
}

/// Result of final Program construction and typed ingress.
pub type Result<T> = std::result::Result<T, ProgramError>;

/// Redaction-safe Program construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProgramError {
    /// Canonical bytes were malformed, non-canonical, or over the fixed bound.
    #[error("program canonical input is invalid")]
    Canonical,
    /// A State/Match declaration violates the sequential contract.
    #[error("program sequential contract is invalid")]
    InvalidContract,
    /// A typed value does not match its nominal contract or catalog brand.
    #[error("program typed value is not qualified by this catalog")]
    InvalidValue,
    /// The catalog registration or typed downcast is not exact.
    #[error("program catalog association is invalid")]
    InvalidCatalog,
    /// A fixed Program or context limit was exceeded.
    #[error("program capacity bound exceeded")]
    Capacity,
}

/// A typed failure contract with one static integrity-blocked value.
///
/// The value is selected by the domain's closed failure contract and receives no input,
/// evidence, or Runtime authority. It is therefore safe for the capability-certified integrity
/// path to use without invoking a State callback or accepting a caller-supplied failure.
pub trait FailureValue: MfmValue {
    /// Returns the contract-fixed failure for an accepted integrity block.
    fn integrity_blocked() -> Self;
}

/// A callback-free typed State contract owned by the domain-facing Program layer.
pub trait State: Send + Sync + 'static {
    /// Complete cumulative input consumed by this State.
    type Input: MfmValue;
    /// Complete successor context or terminal public result.
    type Output: MfmValue;
    /// Explicit fail-fast domain value.
    type Failure: FailureValue;

    /// Returns the stable semantic State implementation identity.
    fn state_id() -> Result<StableId>;

    /// Builds the declared failure for an accepted capability integrity block.
    ///
    /// Most State contracts use their failure type's fixed integrity value. Domains whose
    /// reviewed failure contract carries correlation already present in the cumulative input may
    /// override this static, callback-free projection. Runtime never invokes ordinary State
    /// interpretation on this path.
    fn integrity_failure(_input: &Self::Input) -> Self::Failure {
        Self::Failure::integrity_blocked()
    }
}

/// Derives the one canonical content identity for a semantic State implementation.
///
/// The identity is domain-separated from value and capability contracts. Runtime registration
/// derives it from the State type, so callers cannot bind an unrelated arbitrary reference.
pub fn state_implementation_ref<S: State>() -> Result<ContentRef> {
    let state_id = S::state_id()?;
    let schema = SchemaId::new(
        "mfm.state-implementation",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| ProgramError::InvalidContract)?;
    ContentRef::new(schema, raw_content_digest(state_id.as_str().as_bytes()))
        .map_err(|_| ProgramError::InvalidContract)
}

/// One secret-free immutable State/capability/adapter association.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingDescriptor {
    state_implementation_ref: ContentRef,
    capability_contract_ref: Option<ContentRef>,
    adapter_implementation_ref: Option<ContentRef>,
    physical_target_ref: ContentRef,
    effect_domain: Option<StableId>,
    public_signer_key_instance_ref: Option<ContentRef>,
}

impl BindingDescriptor {
    /// Constructs the sole immutable binding descriptor shape.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_implementation_ref: ContentRef,
        capability_contract_ref: Option<ContentRef>,
        adapter_implementation_ref: Option<ContentRef>,
        physical_target_ref: ContentRef,
        effect_domain: Option<StableId>,
        public_signer_key_instance_ref: Option<ContentRef>,
    ) -> Result<Self> {
        if capability_contract_ref.is_none() != adapter_implementation_ref.is_none() {
            return Err(ProgramError::InvalidContract);
        }
        Ok(Self {
            state_implementation_ref,
            capability_contract_ref,
            adapter_implementation_ref,
            physical_target_ref,
            effect_domain,
            public_signer_key_instance_ref,
        })
    }

    /// Returns the exact State implementation identity.
    pub const fn state_implementation_ref(&self) -> &ContentRef {
        &self.state_implementation_ref
    }

    /// Returns the capability contract identity for Access, if present.
    pub const fn capability_contract_ref(&self) -> Option<&ContentRef> {
        self.capability_contract_ref.as_ref()
    }

    /// Returns the qualified adapter identity for Access, if present.
    pub const fn adapter_implementation_ref(&self) -> Option<&ContentRef> {
        self.adapter_implementation_ref.as_ref()
    }

    /// Returns the immutable physical route/target identity.
    pub const fn physical_target_ref(&self) -> &ContentRef {
        &self.physical_target_ref
    }

    /// Returns the Effect domain, if the State is an Effect.
    pub const fn effect_domain(&self) -> Option<&StableId> {
        self.effect_domain.as_ref()
    }

    /// Returns the public signer key-instance identity, if applicable.
    pub const fn public_signer_key_instance_ref(&self) -> Option<&ContentRef> {
        self.public_signer_key_instance_ref.as_ref()
    }

    /// Returns the content identity of this exact canonical binding descriptor.
    pub fn content_ref(&self) -> Result<ContentRef> {
        let schema = SchemaId::new(
            "mfm.execution-binding",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .map_err(|_| ProgramError::Canonical)?;
        let canonical = canonical_json(self)?;
        ContentRef::new(schema, raw_content_digest(canonical.as_bytes()))
            .map_err(|_| ProgramError::Canonical)
    }
}

/// One callback-free execution mode declared by a State.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionMode {
    /// Deterministic computation with no preparation.
    Pure,
    /// Non-mutating provider access.
    Read {
        /// Exact capability contract.
        capability_contract_ref: ContentRef,
        /// Total attempts including the initial attempt.
        total_attempt_bound: u16,
        /// Whether preparation must carry an interpretation-only prior-fact request.
        fact_selection_required: bool,
    },
    /// Provider operation with exactly one possible entry.
    Effect {
        /// Exact capability contract.
        capability_contract_ref: ContentRef,
        /// Public Effect domain identity fixed by the binding.
        effect_domain: StableId,
        /// Whether preparation must carry an interpretation-only prior-fact request.
        fact_selection_required: bool,
    },
}

impl ExecutionMode {
    /// Validates the mode's attempt and entry contract.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Pure => Ok(()),
            Self::Read {
                total_attempt_bound,
                ..
            } => {
                if (1..=3).contains(total_attempt_bound) {
                    Ok(())
                } else {
                    Err(ProgramError::InvalidContract)
                }
            }
            Self::Effect { effect_domain, .. } => {
                if !effect_domain.as_str().is_empty() {
                    Ok(())
                } else {
                    Err(ProgramError::InvalidContract)
                }
            }
        }
    }

    /// Returns true for Pure, which creates no preparation.
    pub const fn is_pure(&self) -> bool {
        matches!(self, Self::Pure)
    }

    /// Returns the Access capability contract identity, if this is Access.
    pub const fn capability_contract_ref(&self) -> Option<&ContentRef> {
        match self {
            Self::Pure => None,
            Self::Read {
                capability_contract_ref,
                ..
            }
            | Self::Effect {
                capability_contract_ref,
                ..
            } => Some(capability_contract_ref),
        }
    }

    /// Returns the total attempt bound, if this is Access.
    pub const fn total_attempt_bound(&self) -> Option<u16> {
        match self {
            Self::Pure => None,
            Self::Read {
                total_attempt_bound,
                ..
            } => Some(*total_attempt_bound),
            Self::Effect { .. } => Some(1),
        }
    }
}

/// One durable State declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateDeclaration {
    address: SequentialControlAddress,
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: Option<ContentRef>,
    execution: ExecutionMode,
    terminal: bool,
    next_address: Option<SequentialControlAddress>,
    failure_next_address: Option<SequentialControlAddress>,
    execution_binding: Option<BindingDescriptor>,
    maximum_conclusion_bytes: u64,
}

impl<'de> Deserialize<'de> for StateDeclaration {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            address: SequentialControlAddress,
            state_implementation_ref: ContentRef,
            input_contract_ref: ContentRef,
            output_contract_ref: ContentRef,
            failure_contract_ref: Option<ContentRef>,
            execution: ExecutionMode,
            terminal: bool,
            next_address: Option<SequentialControlAddress>,
            failure_next_address: Option<SequentialControlAddress>,
            execution_binding: Option<BindingDescriptor>,
            maximum_conclusion_bytes: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        let mut state = match wire.next_address {
            Some(next) if !wire.terminal => Self::with_next(
                wire.address,
                wire.state_implementation_ref,
                wire.input_contract_ref,
                wire.output_contract_ref,
                wire.failure_contract_ref,
                wire.execution,
                next,
            ),
            None if wire.terminal => Self::new(
                wire.address,
                wire.state_implementation_ref,
                wire.input_contract_ref,
                wire.output_contract_ref,
                wire.failure_contract_ref,
                wire.execution,
                true,
            ),
            _ => Err(ProgramError::InvalidContract),
        }
        .map_err(de::Error::custom)?;
        if let Some(next) = wire.failure_next_address {
            state = state.with_failure_next(next).map_err(de::Error::custom)?;
        }
        if let Some(binding) = wire.execution_binding {
            state = state
                .with_execution_binding(binding)
                .map_err(de::Error::custom)?;
        }
        state
            .with_maximum_conclusion_bytes(wire.maximum_conclusion_bytes)
            .map_err(de::Error::custom)
    }
}

impl StateDeclaration {
    /// Creates and validates one State declaration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: SequentialControlAddress,
        state_implementation_ref: ContentRef,
        input_contract_ref: ContentRef,
        output_contract_ref: ContentRef,
        failure_contract_ref: Option<ContentRef>,
        execution: ExecutionMode,
        terminal: bool,
    ) -> Result<Self> {
        execution.validate()?;
        Ok(Self {
            address,
            state_implementation_ref,
            input_contract_ref,
            output_contract_ref,
            failure_contract_ref,
            execution,
            terminal,
            next_address: None,
            failure_next_address: None,
            execution_binding: None,
            maximum_conclusion_bytes: MAX_STATE_CONCLUSION_BYTES,
        })
    }

    /// Creates a nonterminal State with its explicit sequential successor.
    pub fn with_next(
        address: SequentialControlAddress,
        state_implementation_ref: ContentRef,
        input_contract_ref: ContentRef,
        output_contract_ref: ContentRef,
        failure_contract_ref: Option<ContentRef>,
        execution: ExecutionMode,
        next_address: SequentialControlAddress,
    ) -> Result<Self> {
        let mut state = Self::new(
            address,
            state_implementation_ref,
            input_contract_ref,
            output_contract_ref,
            failure_contract_ref,
            execution,
            false,
        )?;
        state.next_address = Some(next_address);
        Ok(state)
    }

    /// Associates an explicit typed-failure successor with this State.
    ///
    /// The failure value must use `failure_contract_ref` supplied to the constructor. A failure
    /// successor is distinct from the ordinary success successor and is traversed immediately by
    /// the callback-free reducer.
    pub fn with_failure_next(
        mut self,
        failure_next_address: SequentialControlAddress,
    ) -> Result<Self> {
        if self.failure_contract_ref.is_none() {
            return Err(ProgramError::InvalidContract);
        }
        self.failure_next_address = Some(failure_next_address);
        Ok(self)
    }

    /// Associates the State with its immutable process binding descriptor.
    pub fn with_execution_binding(mut self, binding: BindingDescriptor) -> Result<Self> {
        if self.execution.is_pure() {
            return Err(ProgramError::InvalidContract);
        }
        if binding.capability_contract_ref.is_none() != binding.adapter_implementation_ref.is_none()
        {
            return Err(ProgramError::InvalidContract);
        }
        self.execution_binding = Some(binding);
        Ok(self)
    }

    /// Replaces the fixed maximum conclusion reservation for this State.
    pub fn with_maximum_conclusion_bytes(mut self, maximum: u64) -> Result<Self> {
        if maximum == 0 || maximum > MAX_STATE_CONCLUSION_BYTES {
            return Err(ProgramError::Capacity);
        }
        self.maximum_conclusion_bytes = maximum;
        Ok(self)
    }

    /// Returns the exact sequential address.
    pub const fn address(&self) -> &SequentialControlAddress {
        &self.address
    }

    /// Returns the State implementation identity.
    pub const fn state_implementation_ref(&self) -> &ContentRef {
        &self.state_implementation_ref
    }

    /// Returns the State input contract.
    pub const fn input_contract_ref(&self) -> &ContentRef {
        &self.input_contract_ref
    }

    /// Returns the complete successor contract.
    pub const fn output_contract_ref(&self) -> &ContentRef {
        &self.output_contract_ref
    }

    /// Returns the typed failure contract, if the State can fail.
    pub const fn failure_contract_ref(&self) -> Option<&ContentRef> {
        self.failure_contract_ref.as_ref()
    }

    /// Returns the State's sealed execution mode.
    pub const fn execution(&self) -> &ExecutionMode {
        &self.execution
    }

    /// Returns whether this State is a terminal root State.
    pub const fn terminal(&self) -> bool {
        self.terminal
    }

    /// Returns the explicit successor for a nonterminal State.
    pub const fn next_address(&self) -> Option<&SequentialControlAddress> {
        self.next_address.as_ref()
    }

    /// Returns the explicit successor for this State's typed failure route.
    pub const fn failure_next_address(&self) -> Option<&SequentialControlAddress> {
        self.failure_next_address.as_ref()
    }

    /// Returns the immutable execution binding descriptor, if this is Access.
    pub const fn execution_binding(&self) -> Option<&BindingDescriptor> {
        self.execution_binding.as_ref()
    }

    /// Returns the complete conclusion reservation.
    pub const fn maximum_conclusion_bytes(&self) -> u64 {
        self.maximum_conclusion_bytes
    }

    /// Returns whether this Access State requires a preparation-time fact selection.
    pub const fn fact_selection_required(&self) -> bool {
        match &self.execution {
            ExecutionMode::Pure => false,
            ExecutionMode::Read {
                fact_selection_required,
                ..
            }
            | ExecutionMode::Effect {
                fact_selection_required,
                ..
            } => *fact_selection_required,
        }
    }

    /// Returns the Effect domain identity, if this is an Effect State.
    pub const fn effect_domain(&self) -> Option<&StableId> {
        match &self.execution {
            ExecutionMode::Effect { effect_domain, .. } => Some(effect_domain),
            ExecutionMode::Pure | ExecutionMode::Read { .. } => None,
        }
    }
}

/// One exhaustive Match variant with an exact complete payload contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchVariant {
    tag: StableId,
    payload_contract_ref: ContentRef,
    continuation_contract_ref: ContentRef,
    entry_address: SequentialControlAddress,
}

impl MatchVariant {
    /// Creates one Match arm.
    pub fn new(
        tag: StableId,
        payload_contract_ref: ContentRef,
        continuation_contract_ref: ContentRef,
        entry_address: SequentialControlAddress,
    ) -> Self {
        Self {
            tag,
            payload_contract_ref,
            continuation_contract_ref,
            entry_address,
        }
    }

    /// Returns the exact arm tag.
    pub const fn tag(&self) -> &StableId {
        &self.tag
    }

    /// Returns the complete selected payload contract.
    pub const fn payload_contract_ref(&self) -> &ContentRef {
        &self.payload_contract_ref
    }

    /// Returns the common continuation contract.
    pub const fn continuation_contract_ref(&self) -> &ContentRef {
        &self.continuation_contract_ref
    }

    /// Returns the first State reached by this selected arm.
    pub const fn entry_address(&self) -> &SequentialControlAddress {
        &self.entry_address
    }
}

/// One deterministic closed-sum Match selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MatchDeclaration {
    address: SequentialControlAddress,
    selector_contract_ref: ContentRef,
    variants: Vec<MatchVariant>,
}

impl<'de> Deserialize<'de> for MatchDeclaration {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            address: SequentialControlAddress,
            selector_contract_ref: ContentRef,
            variants: Vec<MatchVariant>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.address, wire.selector_contract_ref, wire.variants)
            .map_err(de::Error::custom)
    }
}

impl MatchDeclaration {
    /// Creates an exhaustive Match declaration.
    pub fn new(
        address: SequentialControlAddress,
        selector_contract_ref: ContentRef,
        mut variants: Vec<MatchVariant>,
    ) -> Result<Self> {
        if variants.is_empty() || variants.len() > MAX_MATCH_ARMS {
            return Err(ProgramError::InvalidContract);
        }
        variants.sort_by(|left, right| left.tag.cmp(&right.tag));
        if variants.windows(2).any(|pair| pair[0].tag == pair[1].tag)
            || variants
                .windows(2)
                .any(|pair| pair[0].continuation_contract_ref != pair[1].continuation_contract_ref)
        {
            return Err(ProgramError::InvalidContract);
        }
        Ok(Self {
            address,
            selector_contract_ref,
            variants,
        })
    }

    /// Returns the selector address.
    pub const fn address(&self) -> &SequentialControlAddress {
        &self.address
    }

    /// Returns the closed-sum selector contract.
    pub const fn selector_contract_ref(&self) -> &ContentRef {
        &self.selector_contract_ref
    }

    /// Returns all arms in canonical tag order.
    pub fn variants(&self) -> &[MatchVariant] {
        &self.variants
    }
}

/// The only visible Program declaration variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[allow(clippy::large_enum_variant)]
pub enum Declaration {
    /// One executable State occurrence.
    State(Box<StateDeclaration>),
    /// One deterministic closed-sum selector.
    Match(MatchDeclaration),
}

/// Bounded strict serializable Program data; it is not execution authority by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramDocument {
    entry_point_id: StableId,
    root_contract_ref: ContentRef,
    admitted_context_contract_ref: ContentRef,
    declarations: Vec<Declaration>,
}

impl<'de> Deserialize<'de> for ProgramDocument {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            entry_point_id: StableId,
            root_contract_ref: ContentRef,
            admitted_context_contract_ref: ContentRef,
            declarations: Vec<Declaration>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.entry_point_id,
            wire.root_contract_ref,
            wire.admitted_context_contract_ref,
            wire.declarations,
        )
        .map_err(de::Error::custom)
    }
}

impl ProgramDocument {
    /// Constructs one normalized State/Match-only document.
    pub fn new(
        entry_point_id: StableId,
        root_contract_ref: ContentRef,
        admitted_context_contract_ref: ContentRef,
        declarations: Vec<Declaration>,
    ) -> Result<Self> {
        if declarations.len() > MAX_DECLARATIONS {
            return Err(ProgramError::Capacity);
        }
        if declarations.is_empty() && root_contract_ref != admitted_context_contract_ref {
            return Err(ProgramError::InvalidContract);
        }
        let mut addresses = BTreeMap::new();
        for declaration in &declarations {
            let address = match declaration {
                Declaration::State(state) => state.address(),
                Declaration::Match(selector) => selector.address(),
            };
            if addresses.insert(address.clone(), declaration).is_some() {
                return Err(ProgramError::InvalidContract);
            }
        }
        for declaration in &declarations {
            if let Declaration::State(state) = declaration {
                state.execution.validate()?;
                if state.terminal() != state.next_address().is_none() {
                    return Err(ProgramError::InvalidContract);
                }
                if state.failure_next_address().is_some() && state.failure_contract_ref().is_none()
                {
                    return Err(ProgramError::InvalidContract);
                }
                if state.execution().is_pure() != state.execution_binding().is_none() {
                    return Err(ProgramError::InvalidContract);
                }
                if state.maximum_conclusion_bytes() == 0
                    || state.maximum_conclusion_bytes() > MAX_STATE_CONCLUSION_BYTES
                {
                    return Err(ProgramError::Capacity);
                }
                if state.terminal() && state.output_contract_ref() != &root_contract_ref {
                    return Err(ProgramError::InvalidContract);
                }
            } else if let Declaration::Match(selector) = declaration {
                let normalized = MatchDeclaration::new(
                    selector.address.clone(),
                    selector.selector_contract_ref.clone(),
                    selector.variants.clone(),
                )?;
                if normalized != *selector {
                    return Err(ProgramError::InvalidContract);
                }
                for variant in selector.variants() {
                    let Some(Declaration::State(state)) = addresses.get(variant.entry_address())
                    else {
                        return Err(ProgramError::InvalidContract);
                    };
                    if state.input_contract_ref() != variant.payload_contract_ref()
                        || state.output_contract_ref() != variant.continuation_contract_ref()
                    {
                        return Err(ProgramError::InvalidContract);
                    }
                }
            }
        }
        for declaration in &declarations {
            if let Declaration::State(state) = declaration {
                if let Some(next) = state.next_address() {
                    let target = addresses.get(next).ok_or(ProgramError::InvalidContract)?;
                    let target_input = match target {
                        Declaration::State(target) => target.input_contract_ref(),
                        Declaration::Match(target) => target.selector_contract_ref(),
                    };
                    if state.output_contract_ref() != target_input {
                        return Err(ProgramError::InvalidContract);
                    }
                }
                if let Some(failure_next) = state.failure_next_address() {
                    let target = addresses
                        .get(failure_next)
                        .ok_or(ProgramError::InvalidContract)?;
                    let target_input = match target {
                        Declaration::State(target) => target.input_contract_ref(),
                        Declaration::Match(target) => target.selector_contract_ref(),
                    };
                    if state.failure_contract_ref() != Some(target_input) {
                        return Err(ProgramError::InvalidContract);
                    }
                }
            } else if let Declaration::Match(selector) = declaration {
                for variant in selector.variants() {
                    if !addresses.contains_key(variant.entry_address()) {
                        return Err(ProgramError::InvalidContract);
                    }
                }
            }
        }
        if !declarations.is_empty() {
            let mut incoming = BTreeSet::new();
            for declaration in &declarations {
                match declaration {
                    Declaration::State(state) => {
                        if let Some(next) = state.next_address() {
                            incoming.insert(next.clone());
                        }
                        if let Some(next) = state.failure_next_address() {
                            incoming.insert(next.clone());
                        }
                    }
                    Declaration::Match(selector) => {
                        for variant in selector.variants() {
                            incoming.insert(variant.entry_address().clone());
                        }
                    }
                }
            }
            let entries: Vec<_> = declarations
                .iter()
                .filter(|declaration| {
                    let address = match declaration {
                        Declaration::State(state) => state.address(),
                        Declaration::Match(selector) => selector.address(),
                    };
                    !incoming.contains(address)
                })
                .collect();
            if entries.len() != 1 {
                return Err(ProgramError::InvalidContract);
            }
            let entry = entries[0];
            let entry_address = match entry {
                Declaration::State(state) => state.address().clone(),
                Declaration::Match(selector) => selector.address().clone(),
            };
            let entry_contract = match entry {
                Declaration::State(state) => state.input_contract_ref(),
                Declaration::Match(selector) => selector.selector_contract_ref(),
            };
            if entry_contract != &admitted_context_contract_ref {
                return Err(ProgramError::InvalidContract);
            }
            let mut visiting = BTreeSet::new();
            let mut visited = BTreeSet::new();
            validate_graph(&entry_address, &addresses, &mut visiting, &mut visited)?;
            if visited.len() != declarations.len() {
                return Err(ProgramError::InvalidContract);
            }
            let mut terminal_visiting = BTreeSet::new();
            let mut terminal_paths = BTreeSet::new();
            validate_terminal_paths(
                &entry_address,
                &addresses,
                &mut terminal_visiting,
                &mut terminal_paths,
            )?;
        }
        let document = Self {
            entry_point_id,
            root_contract_ref,
            admitted_context_contract_ref,
            declarations,
        };
        let bytes = canonical_json(&document)?;
        if bytes.as_bytes().len() > MAX_PROGRAM_BYTES {
            return Err(ProgramError::Capacity);
        }
        Ok(document)
    }

    /// Returns the public entry-point identity.
    pub const fn entry_point_id(&self) -> &StableId {
        &self.entry_point_id
    }

    /// Returns the declared root-result contract.
    pub const fn root_contract_ref(&self) -> &ContentRef {
        &self.root_contract_ref
    }

    /// Returns the singular admitted `C0` contract.
    pub const fn admitted_context_contract_ref(&self) -> &ContentRef {
        &self.admitted_context_contract_ref
    }

    /// Returns the normalized sequential declarations.
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    /// Finds one declaration by its exact callback-free control address.
    pub fn declaration(&self, address: &SequentialControlAddress) -> Option<&Declaration> {
        self.declarations
            .iter()
            .find(|declaration| match declaration {
                Declaration::State(state) => state.address() == address,
                Declaration::Match(selector) => selector.address() == address,
            })
    }

    /// Encodes this document through the only canonical Program path.
    pub fn canonical_bytes(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(self)
    }

    /// Strictly decodes one complete canonical Program document without granting catalog
    /// authority.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PROGRAM_BYTES {
            return Err(ProgramError::Capacity);
        }
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ProgramError::Canonical)?;
        let document: Self =
            serde_json::from_slice(canonical.as_bytes()).map_err(|_| ProgramError::Canonical)?;
        let checked = document.canonical_bytes()?;
        if checked.as_bytes() != canonical.as_bytes() {
            return Err(ProgramError::Canonical);
        }
        Ok(document)
    }

    /// Returns the content identity of this exact normalized Program document.
    pub fn program_ref(&self) -> Result<ContentRef> {
        let canonical = self.canonical_bytes()?;
        ContentRef::new(
            program_document_schema_id()?,
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, canonical.digest_bytes()),
        )
        .map_err(|_| ProgramError::InvalidContract)
    }

    /// Returns the unique root control address of this normalized graph.
    pub fn entry_address(&self) -> Result<&SequentialControlAddress> {
        let mut incoming = BTreeSet::new();
        for declaration in &self.declarations {
            match declaration {
                Declaration::State(state) => {
                    if let Some(next) = state.next_address() {
                        incoming.insert(next.clone());
                    }
                    if let Some(next) = state.failure_next_address() {
                        incoming.insert(next.clone());
                    }
                }
                Declaration::Match(selector) => {
                    for variant in selector.variants() {
                        incoming.insert(variant.entry_address().clone());
                    }
                }
            }
        }
        let mut roots = self.declarations.iter().filter_map(|declaration| {
            let address = match declaration {
                Declaration::State(state) => state.address(),
                Declaration::Match(selector) => selector.address(),
            };
            (!incoming.contains(address)).then_some(address)
        });
        let root = roots.next().ok_or(ProgramError::InvalidContract)?;
        if roots.next().is_some() {
            return Err(ProgramError::InvalidContract);
        }
        Ok(root)
    }
}

/// Opaque immutable callback-free Program authority.
pub struct Program {
    document: ProgramDocument,
    program_ref: ProgramRef,
    catalog: Arc<ProgramCatalogInner>,
}

impl fmt::Debug for Program {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Program")
            .field("document", &self.document)
            .field("program_ref", &self.program_ref)
            .finish_non_exhaustive()
    }
}

/// Opaque content address of one normalized Program document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProgramRef(ContentRef);

impl ProgramRef {
    /// Returns the underlying immutable content identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.0
    }
}

impl Program {
    fn new(document: ProgramDocument, catalog: Arc<ProgramCatalogInner>) -> Result<Self> {
        let program_ref = document.program_ref()?;
        Ok(Self {
            document,
            program_ref: ProgramRef(program_ref),
            catalog,
        })
    }

    /// Returns the immutable Program content identity.
    pub const fn program_ref(&self) -> &ProgramRef {
        &self.program_ref
    }

    /// Returns the exact normalized document for callback-free Store/Replay use.
    pub const fn document(&self) -> &ProgramDocument {
        &self.document
    }

    /// Returns whether this Program belongs to the exact in-process catalog instance.
    pub fn belongs_to_catalog(&self, catalog: &ProgramCatalog) -> bool {
        Arc::ptr_eq(&self.catalog, &catalog.inner)
    }
}

type ReifyValue = fn(&SchemaIdentity, &[u8]) -> Result<Box<dyn Any + Send + Sync>>;

struct ValueAssociation {
    type_id: TypeId,
    schema_id: mfm_ids::SchemaId,
    schema_identity: SchemaIdentity,
    descriptor_identity: Vec<u8>,
    reify: ReifyValue,
}

impl PartialEq for ValueAssociation {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
            && self.schema_id == other.schema_id
            && self.descriptor_identity == other.descriptor_identity
    }
}

impl Eq for ValueAssociation {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapabilityMode {
    Read,
    Effect,
}

struct CapabilityAssociation {
    type_id: TypeId,
    intent_contract_ref: ContentRef,
    evidence_contract_ref: ContentRef,
    mode: CapabilityMode,
    total_attempt_bound: u16,
    fact_selection_required: bool,
    bind_evidence: fn(&dyn Any, &dyn Any) -> Result<()>,
    prior_fact_selection: fn(&dyn Any) -> Result<Option<FactSelectionRequest>>,
}

struct ProgramCatalogInner {
    associations: BTreeMap<ContentRef, ValueAssociation>,
    capabilities: BTreeMap<ContentRef, CapabilityAssociation>,
}

/// Cloneable callback-free catalog owner.
#[derive(Clone)]
pub struct ProgramCatalog {
    inner: Arc<ProgramCatalogInner>,
}

impl ProgramCatalog {
    /// Starts a callback-free catalog builder.
    pub fn builder() -> ProgramCatalogBuilder {
        ProgramCatalogBuilder {
            associations: BTreeMap::new(),
            capabilities: BTreeMap::new(),
        }
    }

    /// Returns whether two catalog handles share the exact process-local type brand.
    pub fn same_catalog(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Qualifies one additional normalized Program under this exact catalog brand.
    pub fn program(&self, document: ProgramDocument) -> Result<Program> {
        validate_program_associations(&self.inner, &document)?;
        Program::new(document, Arc::clone(&self.inner))
    }

    /// Returns whether this catalog contains the exact nominal contract, descriptor, and Rust
    /// type association.
    pub fn contains_value<T: MfmValue>(&self, contract_ref: &ContentRef) -> bool {
        value_association::<T>().is_ok_and(|(expected_contract, expected)| {
            &expected_contract == contract_ref
                && self
                    .inner
                    .associations
                    .get(contract_ref)
                    .is_some_and(|registered| registered == &expected)
        })
    }

    /// Returns whether the catalog contains this exact capability association.
    pub fn contains_capability<C: AccessCapabilityContract>(
        &self,
        contract_ref: &ContentRef,
    ) -> bool {
        capability_contract_ref::<C>().is_ok_and(|expected| {
            &expected == contract_ref
                && self
                    .inner
                    .capabilities
                    .get(contract_ref)
                    .is_some_and(|registered| registered.type_id == TypeId::of::<C>())
        })
    }

    /// Creates one typed qualified value under this exact catalog instance.
    pub fn qualify<T: MfmValue>(
        &self,
        contract_ref: ContentRef,
        value: T,
    ) -> Result<QualifiedTypedValue<T>> {
        if !self.contains_value::<T>(&contract_ref) {
            return Err(ProgramError::InvalidValue);
        }
        let canonical = canonical_value(&value)?;
        T::schema_descriptor()
            .map_err(|_| ProgramError::InvalidCatalog)?
            .identity()
            .validate_canonical_value(canonical.as_bytes())
            .map_err(|_| ProgramError::InvalidValue)?;
        let value_ref = ContentRef::new(
            contract_ref.schema_id().clone(),
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, canonical.digest_bytes()),
        )
        .map_err(|_| ProgramError::InvalidValue)?;
        Ok(QualifiedTypedValue {
            contract_ref,
            value_ref,
            canonical_json: canonical,
            value,
            catalog: Arc::clone(&self.inner),
        })
    }

    /// Strictly decodes retained canonical bytes once under one exact registered association.
    pub fn qualify_retained<T: MfmValue>(
        &self,
        contract_ref: ContentRef,
        bytes: &[u8],
    ) -> Result<QualifiedTypedValue<T>> {
        self.qualify_retained_erased(contract_ref.clone(), bytes)?
            .try_downcast(self, &contract_ref)
    }

    /// Strictly qualifies retained bytes through their registered nominal association.
    pub fn qualify_retained_erased(
        &self,
        contract_ref: ContentRef,
        bytes: &[u8],
    ) -> Result<QualifiedValue> {
        let association = self
            .inner
            .associations
            .get(&contract_ref)
            .ok_or(ProgramError::InvalidValue)?;
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ProgramError::Canonical)?;
        let value = (association.reify)(&association.schema_identity, canonical.as_bytes())?;
        let value_ref = ContentRef::new(
            contract_ref.schema_id().clone(),
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, canonical.digest_bytes()),
        )
        .map_err(|_| ProgramError::InvalidValue)?;
        Ok(QualifiedValue {
            contract_ref,
            value_ref,
            canonical_json: canonical,
            value,
            catalog: Arc::clone(&self.inner),
        })
    }

    /// Validates an erased intent against the exact registered capability association.
    pub fn validate_access_intent(
        &self,
        capability_contract_ref: &ContentRef,
        intent: &QualifiedValue,
    ) -> Result<Option<FactSelectionRequest>> {
        let capability = self
            .inner
            .capabilities
            .get(capability_contract_ref)
            .ok_or(ProgramError::InvalidCatalog)?;
        if !intent.belongs_to_catalog(self)
            || intent.contract_ref() != &capability.intent_contract_ref
        {
            return Err(ProgramError::InvalidValue);
        }
        (capability.prior_fact_selection)(intent.value.as_ref())
    }

    /// Validates erased evidence and its binding to the exact registered capability intent.
    pub fn validate_access_evidence(
        &self,
        capability_contract_ref: &ContentRef,
        intent: &QualifiedValue,
        evidence: &QualifiedValue,
    ) -> Result<()> {
        let capability = self
            .inner
            .capabilities
            .get(capability_contract_ref)
            .ok_or(ProgramError::InvalidCatalog)?;
        if !intent.belongs_to_catalog(self)
            || !evidence.belongs_to_catalog(self)
            || intent.contract_ref() != &capability.intent_contract_ref
            || evidence.contract_ref() != &capability.evidence_contract_ref
        {
            return Err(ProgramError::InvalidValue);
        }
        (capability.bind_evidence)(intent.value.as_ref(), evidence.value.as_ref())
    }
}

/// Builder for callback-free Program catalog values.
pub struct ProgramCatalogBuilder {
    associations: BTreeMap<ContentRef, ValueAssociation>,
    capabilities: BTreeMap<ContentRef, CapabilityAssociation>,
}

impl ProgramCatalogBuilder {
    /// Registers one exact nominal contract, descriptor, and Rust type association.
    pub fn register_value<T: MfmValue>(&mut self) -> Result<ContentRef> {
        let (contract_ref, association) = value_association::<T>()?;
        match self.associations.get(&contract_ref) {
            Some(existing) if existing == &association => Ok(contract_ref),
            Some(_) => Err(ProgramError::InvalidCatalog),
            None => {
                self.associations.insert(contract_ref.clone(), association);
                Ok(contract_ref)
            }
        }
    }

    /// Registers one capability and its exact intent/evidence associations.
    pub fn register_capability<C: AccessCapabilityContract>(&mut self) -> Result<ContentRef> {
        C::validate().map_err(|_| ProgramError::InvalidCatalog)?;
        let intent_contract_ref = self.register_value::<C::Intent>()?;
        let evidence_contract_ref = self.register_value::<C::Evidence>()?;
        let capability_contract_ref = capability_contract_ref::<C>()?;
        let association = CapabilityAssociation {
            type_id: TypeId::of::<C>(),
            intent_contract_ref,
            evidence_contract_ref,
            mode: if TypeId::of::<C::Mode>() == TypeId::of::<ReadMode>() {
                CapabilityMode::Read
            } else if TypeId::of::<C::Mode>() == TypeId::of::<EffectMode>() {
                CapabilityMode::Effect
            } else {
                return Err(ProgramError::InvalidCatalog);
            },
            total_attempt_bound: C::total_attempt_bound().get(),
            fact_selection_required: C::Facts::REQUIRED,
            bind_evidence: bind_capability_evidence::<C>,
            prior_fact_selection: capability_prior_fact_selection::<C>,
        };
        match self.capabilities.get(&capability_contract_ref) {
            Some(existing) if existing.type_id == association.type_id => {}
            Some(_) => return Err(ProgramError::InvalidCatalog),
            None => {
                self.capabilities
                    .insert(capability_contract_ref.clone(), association);
            }
        }
        Ok(capability_contract_ref)
    }

    /// Finalizes the exact catalog and one normalized Program.
    pub fn finish(self, document: ProgramDocument) -> Result<(ProgramCatalog, Program)> {
        let catalog = ProgramCatalog {
            inner: Arc::new(ProgramCatalogInner {
                associations: self.associations,
                capabilities: self.capabilities,
            }),
        };
        let program = catalog.program(document)?;
        Ok((catalog, program))
    }
}

/// Hostile Program document ingress bound to one catalog and fixed limits.
pub struct ProgramIngress<'a> {
    catalog: &'a ProgramCatalog,
}

impl<'a> ProgramIngress<'a> {
    /// Creates strict ingress for one exact catalog.
    pub const fn new(catalog: &'a ProgramCatalog) -> Self {
        Self { catalog }
    }

    /// Strictly decodes and normalizes one Program document without expansion or callbacks.
    pub fn decode(&self, bytes: &[u8]) -> Result<Program> {
        let document = ProgramDocument::decode_canonical(bytes)?;
        self.catalog.program(document)
    }
}

/// One owned typed value qualified by a nominal contract and catalog instance.
pub struct QualifiedTypedValue<T: MfmValue> {
    contract_ref: ContentRef,
    value_ref: ContentRef,
    canonical_json: PlainCanonicalJsonBytes,
    value: T,
    catalog: Arc<ProgramCatalogInner>,
}

impl<T: MfmValue> QualifiedTypedValue<T> {
    /// Returns the nominal contract identity.
    pub const fn contract_ref(&self) -> &ContentRef {
        &self.contract_ref
    }

    /// Returns the content identity of the exact value bytes.
    pub const fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }

    /// Returns the exact canonical bytes without decoding them again.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical_json.as_bytes()
    }

    /// Borrows the retained typed value.
    pub const fn as_ref(&self) -> &T {
        &self.value
    }

    /// Returns whether this typed witness belongs to the exact catalog instance.
    pub fn belongs_to_catalog(&self, catalog: &ProgramCatalog) -> bool {
        Arc::ptr_eq(&self.catalog, &catalog.inner)
    }

    /// Consumes the witness and returns its typed value.
    pub fn into_value(self) -> T {
        self.value
    }

    /// Erases the Rust type while retaining the exact catalog qualification.
    pub fn erase(self) -> QualifiedValue {
        QualifiedValue {
            contract_ref: self.contract_ref,
            value_ref: self.value_ref,
            canonical_json: self.canonical_json,
            value: Box::new(self.value),
            catalog: self.catalog,
        }
    }
}

/// One safely erased owned value. There is no public unchecked downcast or free constructor.
pub struct QualifiedValue {
    contract_ref: ContentRef,
    value_ref: ContentRef,
    canonical_json: PlainCanonicalJsonBytes,
    value: Box<dyn Any + Send + Sync>,
    catalog: Arc<ProgramCatalogInner>,
}

impl QualifiedValue {
    /// Returns the nominal contract identity.
    pub const fn contract_ref(&self) -> &ContentRef {
        &self.contract_ref
    }

    /// Returns the content identity.
    pub const fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }

    /// Returns canonical bytes retained during qualification.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical_json.as_bytes()
    }

    /// Returns whether this value belongs to the exact catalog instance.
    pub fn belongs_to_catalog(&self, catalog: &ProgramCatalog) -> bool {
        Arc::ptr_eq(&self.catalog, &catalog.inner)
    }

    /// Fallibly downcasts this value under the exact catalog brand and nominal contract.
    ///
    /// The operation consumes the erased owner.  It never decodes canonical bytes and cannot
    /// succeed for a foreign catalog, a different contract, or a different Rust value type.
    pub fn try_downcast<T: MfmValue>(
        self,
        catalog: &ProgramCatalog,
        contract_ref: &ContentRef,
    ) -> Result<QualifiedTypedValue<T>> {
        if !Arc::ptr_eq(&self.catalog, &catalog.inner)
            || &self.contract_ref != contract_ref
            || !catalog.contains_value::<T>(contract_ref)
        {
            return Err(ProgramError::InvalidCatalog);
        }
        let value = self
            .value
            .downcast::<T>()
            .map_err(|_| ProgramError::InvalidValue)?;
        Ok(QualifiedTypedValue {
            contract_ref: self.contract_ref,
            value_ref: self.value_ref,
            canonical_json: self.canonical_json,
            value: *value,
            catalog: self.catalog,
        })
    }
}

/// Returns the one canonical nominal contract identity for an MFM value type.
pub fn nominal_contract_ref<T: MfmValue>() -> Result<ContentRef> {
    value_association::<T>().map(|(contract_ref, _)| contract_ref)
}

fn value_association<T: MfmValue>() -> Result<(ContentRef, ValueAssociation)> {
    let descriptor = T::schema_descriptor().map_err(|_| ProgramError::InvalidCatalog)?;
    let schema_identity = descriptor.identity().clone();
    let descriptor_schema_id = schema_identity
        .schema_id()
        .map_err(|_| ProgramError::InvalidCatalog)?;
    let schema_id = T::schema_id().map_err(|_| ProgramError::InvalidCatalog)?;
    if schema_id != descriptor_schema_id {
        return Err(ProgramError::InvalidCatalog);
    }
    let descriptor_identity = descriptor
        .identity_canonical_json()
        .map_err(|_| ProgramError::InvalidCatalog)?
        .as_bytes()
        .to_vec();
    let contract_ref = ContentRef::new(schema_id.clone(), raw_content_digest(b"mfm.contract.v1"))
        .map_err(|_| ProgramError::InvalidCatalog)?;
    Ok((
        contract_ref,
        ValueAssociation {
            type_id: TypeId::of::<T>(),
            schema_id,
            schema_identity,
            descriptor_identity,
            reify: reify_value::<T>,
        },
    ))
}

fn validate_program_associations(
    catalog: &ProgramCatalogInner,
    document: &ProgramDocument,
) -> Result<()> {
    let mut contracts = BTreeSet::from([
        document.root_contract_ref(),
        document.admitted_context_contract_ref(),
    ]);
    for declaration in document.declarations() {
        match declaration {
            Declaration::State(state) => {
                contracts.insert(state.input_contract_ref());
                contracts.insert(state.output_contract_ref());
                if let Some(failure) = state.failure_contract_ref() {
                    contracts.insert(failure);
                }
            }
            Declaration::Match(selector) => {
                contracts.insert(selector.selector_contract_ref());
                for variant in selector.variants() {
                    contracts.insert(variant.payload_contract_ref());
                    contracts.insert(variant.continuation_contract_ref());
                }
            }
        }
    }
    if !contracts
        .into_iter()
        .all(|contract| catalog.associations.contains_key(contract))
    {
        return Err(ProgramError::InvalidCatalog);
    }
    document.declarations().iter().try_for_each(|declaration| {
        let Declaration::State(state) = declaration else {
            return Ok(());
        };
        let Some(binding) = state.execution_binding() else {
            return state
                .execution()
                .is_pure()
                .then_some(())
                .ok_or(ProgramError::InvalidCatalog);
        };
        if binding.state_implementation_ref() != state.state_implementation_ref()
            || binding.capability_contract_ref() != state.execution().capability_contract_ref()
        {
            return Err(ProgramError::InvalidCatalog);
        }
        let capability = catalog
            .capabilities
            .get(
                state
                    .execution()
                    .capability_contract_ref()
                    .ok_or(ProgramError::InvalidCatalog)?,
            )
            .ok_or(ProgramError::InvalidCatalog)?;
        let valid_mode = match state.execution() {
            ExecutionMode::Read {
                total_attempt_bound,
                fact_selection_required,
                ..
            } => {
                capability.mode == CapabilityMode::Read
                    && capability.total_attempt_bound == *total_attempt_bound
                    && capability.fact_selection_required == *fact_selection_required
                    && binding.effect_domain().is_none()
            }
            ExecutionMode::Effect {
                effect_domain,
                fact_selection_required,
                ..
            } => {
                capability.mode == CapabilityMode::Effect
                    && capability.fact_selection_required == *fact_selection_required
                    && binding.effect_domain() == Some(effect_domain)
            }
            ExecutionMode::Pure => false,
        };
        valid_mode.then_some(()).ok_or(ProgramError::InvalidCatalog)
    })
}

/// Returns the one canonical identity for an Access capability contract.
pub fn capability_contract_ref<C: AccessCapabilityContract>() -> Result<ContentRef> {
    let contract_id = C::contract_id().map_err(|_| ProgramError::InvalidCatalog)?;
    let schema = SchemaId::new(
        "mfm.capability-contract",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0; 32]),
    )
    .map_err(|_| ProgramError::InvalidCatalog)?;
    ContentRef::new(schema, raw_content_digest(contract_id.as_str().as_bytes()))
        .map_err(|_| ProgramError::InvalidCatalog)
}

fn reify_value<T: MfmValue>(
    schema_identity: &SchemaIdentity,
    bytes: &[u8],
) -> Result<Box<dyn Any + Send + Sync>> {
    schema_identity
        .validate_canonical_value_for_prevalidated_owner(bytes)
        .map_err(|_| ProgramError::InvalidValue)?;
    serde_json::from_slice::<T>(bytes)
        .map(|value| Box::new(value) as Box<dyn Any + Send + Sync>)
        .map_err(|_| ProgramError::InvalidValue)
}

fn bind_capability_evidence<C: AccessCapabilityContract>(
    intent: &dyn Any,
    evidence: &dyn Any,
) -> Result<()> {
    C::bind_evidence(
        intent
            .downcast_ref::<C::Intent>()
            .ok_or(ProgramError::InvalidValue)?,
        evidence
            .downcast_ref::<C::Evidence>()
            .ok_or(ProgramError::InvalidValue)?,
    )
    .map_err(|_| ProgramError::InvalidValue)
}

fn capability_prior_fact_selection<C: AccessCapabilityContract>(
    intent: &dyn Any,
) -> Result<Option<FactSelectionRequest>> {
    C::prior_fact_selection(
        intent
            .downcast_ref::<C::Intent>()
            .ok_or(ProgramError::InvalidValue)?,
    )
    .map_err(|_| ProgramError::InvalidValue)
}

/// Canonicalizes one MFM value without a serialize/decode round trip.
pub fn canonical_value<T: MfmValue>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let encoded = serde_json::to_string(value).map_err(|_| ProgramError::Canonical)?;
    PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| ProgramError::Canonical)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let encoded = serde_json::to_string(value).map_err(|_| ProgramError::Canonical)?;
    PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| ProgramError::Canonical)
}

fn validate_graph(
    address: &SequentialControlAddress,
    declarations: &BTreeMap<SequentialControlAddress, &Declaration>,
    visiting: &mut BTreeSet<SequentialControlAddress>,
    visited: &mut BTreeSet<SequentialControlAddress>,
) -> Result<()> {
    if visited.contains(address) {
        return Ok(());
    }
    if !visiting.insert(address.clone()) {
        return Err(ProgramError::InvalidContract);
    }
    let declaration = declarations
        .get(address)
        .ok_or(ProgramError::InvalidContract)?;
    match declaration {
        Declaration::State(state) => {
            if let Some(next) = state.next_address() {
                validate_graph(next, declarations, visiting, visited)?;
            }
            if let Some(next) = state.failure_next_address() {
                validate_graph(next, declarations, visiting, visited)?;
            }
        }
        Declaration::Match(selector) => {
            for variant in selector.variants() {
                validate_graph(variant.entry_address(), declarations, visiting, visited)?;
            }
        }
    }
    visiting.remove(address);
    visited.insert(address.clone());
    Ok(())
}

fn validate_terminal_paths(
    address: &SequentialControlAddress,
    declarations: &BTreeMap<SequentialControlAddress, &Declaration>,
    visiting: &mut BTreeSet<SequentialControlAddress>,
    terminal_paths: &mut BTreeSet<SequentialControlAddress>,
) -> Result<()> {
    if terminal_paths.contains(address) {
        return Ok(());
    }
    if !visiting.insert(address.clone()) {
        return Err(ProgramError::InvalidContract);
    }
    let declaration = declarations
        .get(address)
        .ok_or(ProgramError::InvalidContract)?;
    match declaration {
        Declaration::State(state) => {
            if !state.terminal() {
                validate_terminal_paths(
                    state.next_address().ok_or(ProgramError::InvalidContract)?,
                    declarations,
                    visiting,
                    terminal_paths,
                )?;
            }
            if let Some(next) = state.failure_next_address() {
                validate_terminal_paths(next, declarations, visiting, terminal_paths)?;
            }
        }
        Declaration::Match(selector) => {
            for variant in selector.variants() {
                validate_terminal_paths(
                    variant.entry_address(),
                    declarations,
                    visiting,
                    terminal_paths,
                )?;
            }
        }
    }
    visiting.remove(address);
    terminal_paths.insert(address.clone());
    Ok(())
}

/// Keeps the generic marker visible in generated rustdoc without exposing erased internals.
#[allow(dead_code)]
struct _CatalogMarker<T>(PhantomData<T>);

#[cfg(test)]
#[path = "../tests/single_trust_unit.rs"]
mod tests;
