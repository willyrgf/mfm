use mfm_canonical::CanonicalValue;
use mfm_ids::{
    ContentRef, EffectKey, FieldPath, GenesisDigest, InvocationIdentity, JournalCommitDigest,
    JournalRecordHash, NodeId, RequestDigest, RunId, RunSemanticStateDigest, SpecHash, StableId,
    TenantScopeId,
};

use super::codec::{
    array_field, bool_field, content_ref_field, cv_array, cv_content_ref, cv_string,
    define_schema_value, domain_digest, effect_key_field, field_path_field,
    invocation_identity_field, node_id_field, nullable_field, object, required_field, run_id_field,
    spec_hash_field, stable_id_field, string_field, tagged_object, tenant_scope_id_field,
    u32_field,
};
use super::{
    BlockingSource, CapabilityBindingRef, ExternalAccessAuthorized, ExternalAccessObserved,
    InputManifestRef, NodePhase, Result, RunPhase, StateTransitionCommitted, TransitionRef,
    ValueRef,
};

define_schema_value! {
    /// Sole immutable root record of one admitted run.
    pub struct RunAdmitted => "mfm.run-admitted.v1";
    /// One initial typed binding admitted with the run root.
    pub struct InitialBinding => "mfm.initial-binding.v1";
    /// Sole structural run closure record.
    pub struct RunClosed => "mfm.run-closed.v1";
    /// Closed five-record journal algebra.
    pub struct RunJournalRecord => "mfm.run-journal-record.v2";
    /// Post-commit semantic closure coordinate.
    pub struct SemanticClosureCoordinate => "mfm.semantic-closure-coordinate.v1";
    /// Canonical run semantic-state digest preimage.
    pub struct RunSemanticStatePreimage => "mfm.run-semantic-state-preimage.v1";
    /// Closed terminal outcome of one node occurrence.
    pub struct NodeTerminalOutcome => "mfm.node-terminal-outcome.v1";
    /// One typed node component of the semantic run state.
    pub struct NodeSemanticState => "mfm.node-semantic-state.v1";
    /// One typed binding component of the semantic run state.
    pub struct SemanticBinding => "mfm.semantic-binding.v1";
    /// One unresolved effect component of the semantic run state.
    pub struct PendingEffectState => "mfm.pending-effect-state.v1";
    /// Certified run terminal contract.
    pub struct RunTerminalContract => "mfm.run-terminal-contract.v1";
}

/// Typed fields of one initial run binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialBindingFields {
    /// Exact destination field path.
    pub field_path: FieldPath,
    /// Full retained value authority.
    pub value_ref: ValueRef,
    /// Exact certified source role.
    pub source_role_ref: ContentRef,
}

impl InitialBinding {
    /// Constructs and validates one initial binding.
    pub fn new(
        field_path: &FieldPath,
        value_ref: &ValueRef,
        source_role_ref: &ContentRef,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("field_path", cv_string(field_path.as_str())),
            ("value_ref", value_ref.canonical_value()?),
            ("source_role_ref", cv_content_ref(source_role_ref)?),
        ])?)
    }

    /// Projects every initial-binding field.
    pub fn fields(&self) -> Result<InitialBindingFields> {
        Ok(InitialBindingFields {
            field_path: field_path_field(self, "field_path")?,
            value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            source_role_ref: content_ref_field(self, "source_role_ref")?,
        })
    }
}

/// Typed fields of the certified run terminal contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTerminalContractFields {
    /// Certified node occurrences that must succeed.
    pub required_success_nodes: Vec<NodeId>,
    /// Whether closure requires an assembled certified public output.
    pub requires_public_output: bool,
}

impl RunTerminalContract {
    /// Constructs one exact certified run terminal contract.
    pub fn new(required_success_nodes: &[NodeId], requires_public_output: bool) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "required_success_nodes",
                cv_array(
                    required_success_nodes
                        .iter()
                        .map(|node_id| Ok(cv_string(node_id.as_str()))),
                )?,
            ),
            (
                "requires_public_output",
                CanonicalValue::Bool(requires_public_output),
            ),
        ])?)
    }

    /// Projects every certified run terminal-contract field.
    pub fn fields(&self) -> Result<RunTerminalContractFields> {
        Ok(RunTerminalContractFields {
            required_success_nodes: array_field(self, "required_success_nodes")?
                .into_iter()
                .map(|value| {
                    let CanonicalValue::String(value) = value else {
                        return Err(super::JournalError::Projection);
                    };
                    NodeId::parse(value).map_err(Into::into)
                })
                .collect::<Result<_>>()?,
            requires_public_output: bool_field(self, "requires_public_output")?,
        })
    }
}

/// Typed fields of the immutable run-admission root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAdmittedFields {
    /// Deterministically derived run identity.
    pub run_id: RunId,
    /// App-owned tenant scope.
    pub tenant_scope_id: TenantScopeId,
    /// Caller-supplied UUIDv4 invocation identity.
    pub invocation_identity: InvocationIdentity,
    /// Exact admitted entry-point operation.
    pub entry_point_operation_id: StableId,
    /// Authored operation contract.
    pub operation_contract_ref: ContentRef,
    /// Admitted executable identity.
    pub executable_identity_ref: ContentRef,
    /// Certified spec hash.
    pub spec_hash: SpecHash,
    /// Exact certified spec.
    pub certified_spec_ref: ContentRef,
    /// Exact certificate.
    pub certificate_ref: ContentRef,
    /// Exact state implementation manifest.
    pub state_implementation_manifest_ref: ContentRef,
    /// Exact capability binding manifest.
    pub capability_binding_manifest_ref: ContentRef,
    /// Exact configuration manifest.
    pub config_manifest_ref: ContentRef,
    /// Exact seed manifest.
    pub seed_manifest_ref: ContentRef,
    /// Exact context manifest.
    pub context_manifest_ref: ContentRef,
    /// Exact cross-run source manifest.
    pub cross_run_source_manifest_ref: ContentRef,
    /// Complete ordered initial typed bindings.
    pub initial_bindings: Vec<InitialBinding>,
    /// Frozen per-run genesis digest.
    pub genesis_digest: GenesisDigest,
    /// Initial canonical semantic-state digest.
    pub initial_run_state_digest: RunSemanticStateDigest,
}

impl RunAdmitted {
    /// Constructs and validates the immutable run-admission root.
    pub fn new(fields: &RunAdmittedFields) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.run-admitted.v1".to_owned()),
            ),
            ("run_id", cv_string(fields.run_id.as_str())),
            (
                "tenant_scope_id",
                cv_string(fields.tenant_scope_id.as_str()),
            ),
            (
                "invocation_identity",
                cv_string(fields.invocation_identity.as_str()),
            ),
            (
                "entry_point_operation_id",
                cv_string(fields.entry_point_operation_id.as_str()),
            ),
            (
                "operation_contract_ref",
                cv_content_ref(&fields.operation_contract_ref)?,
            ),
            (
                "executable_identity_ref",
                cv_content_ref(&fields.executable_identity_ref)?,
            ),
            ("spec_hash", cv_string(fields.spec_hash.as_str())),
            (
                "certified_spec_ref",
                cv_content_ref(&fields.certified_spec_ref)?,
            ),
            ("certificate_ref", cv_content_ref(&fields.certificate_ref)?),
            (
                "state_implementation_manifest_ref",
                cv_content_ref(&fields.state_implementation_manifest_ref)?,
            ),
            (
                "capability_binding_manifest_ref",
                cv_content_ref(&fields.capability_binding_manifest_ref)?,
            ),
            (
                "config_manifest_ref",
                cv_content_ref(&fields.config_manifest_ref)?,
            ),
            (
                "seed_manifest_ref",
                cv_content_ref(&fields.seed_manifest_ref)?,
            ),
            (
                "context_manifest_ref",
                cv_content_ref(&fields.context_manifest_ref)?,
            ),
            (
                "cross_run_source_manifest_ref",
                cv_content_ref(&fields.cross_run_source_manifest_ref)?,
            ),
            (
                "initial_bindings",
                cv_array(
                    fields
                        .initial_bindings
                        .iter()
                        .map(|value| value.canonical_value()),
                )?,
            ),
            ("genesis_digest", cv_string(fields.genesis_digest.as_str())),
            (
                "initial_run_state_digest",
                cv_string(fields.initial_run_state_digest.as_str()),
            ),
        ])?)
    }

    /// Projects every immutable admission-root field.
    pub fn fields(&self) -> Result<RunAdmittedFields> {
        Ok(RunAdmittedFields {
            run_id: run_id_field(self, "run_id")?,
            tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
            invocation_identity: invocation_identity_field(self, "invocation_identity")?,
            entry_point_operation_id: stable_id_field(self, "entry_point_operation_id")?,
            operation_contract_ref: content_ref_field(self, "operation_contract_ref")?,
            executable_identity_ref: content_ref_field(self, "executable_identity_ref")?,
            spec_hash: spec_hash_field(self, "spec_hash")?,
            certified_spec_ref: content_ref_field(self, "certified_spec_ref")?,
            certificate_ref: content_ref_field(self, "certificate_ref")?,
            state_implementation_manifest_ref: content_ref_field(
                self,
                "state_implementation_manifest_ref",
            )?,
            capability_binding_manifest_ref: content_ref_field(
                self,
                "capability_binding_manifest_ref",
            )?,
            config_manifest_ref: content_ref_field(self, "config_manifest_ref")?,
            seed_manifest_ref: content_ref_field(self, "seed_manifest_ref")?,
            context_manifest_ref: content_ref_field(self, "context_manifest_ref")?,
            cross_run_source_manifest_ref: content_ref_field(
                self,
                "cross_run_source_manifest_ref",
            )?,
            initial_bindings: array_field(self, "initial_bindings")?
                .into_iter()
                .map(InitialBinding::from_canonical_value)
                .collect::<Result<_>>()?,
            genesis_digest: GenesisDigest::parse(string_field(self, "genesis_digest")?)?,
            initial_run_state_digest: RunSemanticStateDigest::parse(string_field(
                self,
                "initial_run_state_digest",
            )?)?,
        })
    }
}

/// Typed fields of the sole closure record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunClosedFields {
    /// Exact semantic hash of the immediately preceding terminal transition.
    pub terminal_transition_record_hash: JournalRecordHash,
}

impl RunClosed {
    /// Constructs and validates a structural closure record.
    pub fn new(terminal_transition_record_hash: &JournalRecordHash) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.run-closed.v1".to_owned()),
            ),
            (
                "terminal_transition_record_hash",
                cv_string(terminal_transition_record_hash.as_str()),
            ),
        ])?)
    }

    /// Projects the exact terminal transition hash.
    pub fn fields(&self) -> Result<RunClosedFields> {
        Ok(RunClosedFields {
            terminal_transition_record_hash: JournalRecordHash::parse(string_field(
                self,
                "terminal_transition_record_hash",
            )?)?,
        })
    }
}

/// Closed top-level journal record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunJournalRecordFields {
    /// Immutable run-admission root.
    RunAdmitted(RunAdmitted),
    /// Complete semantic transition.
    StateTransitionCommitted(StateTransitionCommitted),
    /// External access authorization.
    ExternalAccessAuthorized(ExternalAccessAuthorized),
    /// External access observation.
    ExternalAccessObserved(ExternalAccessObserved),
    /// Structural run closure.
    RunClosed(RunClosed),
}

/// Closed top-level journal record kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunJournalRecordKind {
    /// Immutable run-admission root.
    RunAdmitted,
    /// Complete semantic transition.
    StateTransitionCommitted,
    /// External access authorization.
    ExternalAccessAuthorized,
    /// External access observation.
    ExternalAccessObserved,
    /// Structural run closure.
    RunClosed,
}

impl RunJournalRecord {
    /// Wraps an admission root in the closed journal algebra.
    pub fn run_admitted(payload: &RunAdmitted) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "run_admitted",
            [("payload", payload.canonical_value()?)],
        )?)
    }

    /// Wraps a semantic transition in the closed journal algebra.
    pub fn state_transition_committed(payload: &StateTransitionCommitted) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "state_transition_committed",
            [("payload", payload.canonical_value()?)],
        )?)
    }

    /// Wraps an external authorization in the closed journal algebra.
    pub fn external_access_authorized(payload: &ExternalAccessAuthorized) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "external_access_authorized",
            [("payload", payload.canonical_value()?)],
        )?)
    }

    /// Wraps an external observation in the closed journal algebra.
    pub fn external_access_observed(payload: &ExternalAccessObserved) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "external_access_observed",
            [("payload", payload.canonical_value()?)],
        )?)
    }

    /// Wraps a structural closure in the closed journal algebra.
    pub fn run_closed(payload: &RunClosed) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "run_closed",
            [("payload", payload.canonical_value()?)],
        )?)
    }

    /// Returns the closed record kind.
    pub fn kind(&self) -> Result<RunJournalRecordKind> {
        match super::codec::tag(self)?.as_str() {
            "run_admitted" => Ok(RunJournalRecordKind::RunAdmitted),
            "state_transition_committed" => Ok(RunJournalRecordKind::StateTransitionCommitted),
            "external_access_authorized" => Ok(RunJournalRecordKind::ExternalAccessAuthorized),
            "external_access_observed" => Ok(RunJournalRecordKind::ExternalAccessObserved),
            "run_closed" => Ok(RunJournalRecordKind::RunClosed),
            _ => Err(super::JournalError::Projection),
        }
    }

    /// Projects the exact typed record payload.
    pub fn fields(&self) -> Result<RunJournalRecordFields> {
        let payload = required_field(self, "payload")?;
        match self.kind()? {
            RunJournalRecordKind::RunAdmitted => {
                RunAdmitted::from_canonical_value(payload).map(RunJournalRecordFields::RunAdmitted)
            }
            RunJournalRecordKind::StateTransitionCommitted => {
                StateTransitionCommitted::from_canonical_value(payload)
                    .map(RunJournalRecordFields::StateTransitionCommitted)
            }
            RunJournalRecordKind::ExternalAccessAuthorized => {
                ExternalAccessAuthorized::from_canonical_value(payload)
                    .map(RunJournalRecordFields::ExternalAccessAuthorized)
            }
            RunJournalRecordKind::ExternalAccessObserved => {
                ExternalAccessObserved::from_canonical_value(payload)
                    .map(RunJournalRecordFields::ExternalAccessObserved)
            }
            RunJournalRecordKind::RunClosed => {
                RunClosed::from_canonical_value(payload).map(RunJournalRecordFields::RunClosed)
            }
        }
    }

    /// Returns whether this record embeds transition fact emissions.
    pub fn emits_facts(&self) -> Result<bool> {
        match self.fields()? {
            RunJournalRecordFields::StateTransitionCommitted(transition) => {
                Ok(!transition.fact_emissions()?.is_empty())
            }
            RunJournalRecordFields::RunAdmitted(_)
            | RunJournalRecordFields::ExternalAccessAuthorized(_)
            | RunJournalRecordFields::ExternalAccessObserved(_)
            | RunJournalRecordFields::RunClosed(_) => Ok(false),
        }
    }
}

/// Typed fields of a semantic closure coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticClosureCoordinateFields {
    /// Exact terminal transition.
    pub terminal_transition_ref: TransitionRef,
    /// Digest of the commit containing transition and closure.
    pub containing_commit_digest: JournalCommitDigest,
}

impl SemanticClosureCoordinate {
    /// Constructs the post-commit semantic closure coordinate.
    pub fn new(
        terminal_transition_ref: &TransitionRef,
        containing_commit_digest: &JournalCommitDigest,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "terminal_transition_ref",
                terminal_transition_ref.canonical_value()?,
            ),
            (
                "containing_commit_digest",
                cv_string(containing_commit_digest.as_str()),
            ),
        ])?)
    }

    /// Projects every semantic closure coordinate field.
    pub fn fields(&self) -> Result<SemanticClosureCoordinateFields> {
        Ok(SemanticClosureCoordinateFields {
            terminal_transition_ref: TransitionRef::from_canonical_value(required_field(
                self,
                "terminal_transition_ref",
            )?)?,
            containing_commit_digest: JournalCommitDigest::parse(string_field(
                self,
                "containing_commit_digest",
            )?)?,
        })
    }
}

/// Closed terminal-outcome variant and its typed fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeTerminalOutcomeFields {
    /// The node settled successfully.
    Succeeded,
    /// The node settled with a typed failure.
    Failed {
        /// Full retained typed-failure authority.
        typed_failure_ref: ValueRef,
    },
    /// The node was skipped because its certified dependencies became impossible.
    Skipped {
        /// Complete direct blocking-source set.
        blocking_sources: Vec<BlockingSource>,
    },
}

impl NodeTerminalOutcome {
    /// Constructs the successful terminal outcome.
    pub fn succeeded() -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "succeeded",
            std::iter::empty::<(String, CanonicalValue)>(),
        )?)
    }

    /// Constructs a terminal typed failure.
    pub fn failed(typed_failure_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "failed",
            [("typed_failure_ref", typed_failure_ref.canonical_value()?)],
        )?)
    }

    /// Constructs a dependency-skipped terminal outcome.
    pub fn skipped(blocking_sources: &[BlockingSource]) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "skipped",
            [(
                "blocking_sources",
                cv_array(
                    blocking_sources
                        .iter()
                        .map(|source| source.canonical_value()),
                )?,
            )],
        )?)
    }

    /// Projects the closed terminal outcome and all typed fields.
    pub fn fields(&self) -> Result<NodeTerminalOutcomeFields> {
        match super::codec::tag(self)?.as_str() {
            "succeeded" => Ok(NodeTerminalOutcomeFields::Succeeded),
            "failed" => Ok(NodeTerminalOutcomeFields::Failed {
                typed_failure_ref: ValueRef::from_canonical_value(required_field(
                    self,
                    "typed_failure_ref",
                )?)?,
            }),
            "skipped" => Ok(NodeTerminalOutcomeFields::Skipped {
                blocking_sources: array_field(self, "blocking_sources")?
                    .into_iter()
                    .map(BlockingSource::from_canonical_value)
                    .collect::<Result<_>>()?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of one node semantic-state component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSemanticStateFields {
    /// Certified node occurrence.
    pub node_id: NodeId,
    /// Current semantic phase.
    pub phase: NodePhase,
    /// Terminal outcome, present exactly when the node is terminal.
    pub terminal_outcome: Option<NodeTerminalOutcome>,
}

impl NodeSemanticState {
    /// Constructs one exact typed node semantic-state component.
    pub fn new(
        node_id: &NodeId,
        phase: NodePhase,
        terminal_outcome: Option<&NodeTerminalOutcome>,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("node_id", cv_string(node_id.as_str())),
            ("phase", cv_string(phase.as_str())),
            (
                "terminal_outcome",
                terminal_outcome
                    .map(NodeTerminalOutcome::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])?)
    }

    /// Projects every typed node semantic-state field.
    pub fn fields(&self) -> Result<NodeSemanticStateFields> {
        Ok(NodeSemanticStateFields {
            node_id: node_id_field(self, "node_id")?,
            phase: NodePhase::parse(&string_field(self, "phase")?)?,
            terminal_outcome: nullable_field(self, "terminal_outcome")?
                .map(NodeTerminalOutcome::from_canonical_value)
                .transpose()?,
        })
    }
}

/// Closed semantic-binding variant and all typed fields.
///
/// The owned fields keep this cold projection API direct and allocation-free.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticBindingFields {
    /// One materialized input field.
    Input {
        /// Consuming node occurrence.
        node_id: NodeId,
        /// Exact destination field.
        field_path: FieldPath,
        /// Full retained input authority.
        value_ref: ValueRef,
    },
    /// One produced output.
    Output {
        /// Producing node occurrence.
        node_id: NodeId,
        /// Output ordinal.
        output_ordinal: u32,
        /// Full retained output authority.
        value_ref: ValueRef,
    },
    /// One emitted fact and its complete retained roots.
    Fact {
        /// Producing node occurrence.
        node_id: NodeId,
        /// Emission ordinal.
        emission_ordinal: u32,
        /// Store-authored claim envelope authority.
        claim_ref: ValueRef,
        /// Full retained subject authority.
        subject_ref: ValueRef,
        /// Full retained response authority.
        response_ref: ValueRef,
    },
}

impl SemanticBinding {
    /// Constructs one materialized input binding.
    pub fn input(node_id: &NodeId, field_path: &FieldPath, value_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "input",
            [
                ("node_id", cv_string(node_id.as_str())),
                ("field_path", cv_string(field_path.as_str())),
                ("value_ref", value_ref.canonical_value()?),
            ],
        )?)
    }

    /// Constructs one produced output binding.
    pub fn output(node_id: &NodeId, output_ordinal: u32, value_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "output",
            [
                ("node_id", cv_string(node_id.as_str())),
                (
                    "output_ordinal",
                    CanonicalValue::Unsigned(u64::from(output_ordinal)),
                ),
                ("value_ref", value_ref.canonical_value()?),
            ],
        )?)
    }

    /// Constructs one complete emitted-fact binding.
    pub fn fact(
        node_id: &NodeId,
        emission_ordinal: u32,
        claim_ref: &ValueRef,
        subject_ref: &ValueRef,
        response_ref: &ValueRef,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "fact",
            [
                ("node_id", cv_string(node_id.as_str())),
                (
                    "emission_ordinal",
                    CanonicalValue::Unsigned(u64::from(emission_ordinal)),
                ),
                ("claim_ref", claim_ref.canonical_value()?),
                ("subject_ref", subject_ref.canonical_value()?),
                ("response_ref", response_ref.canonical_value()?),
            ],
        )?)
    }

    /// Projects the closed semantic binding and all typed fields.
    pub fn fields(&self) -> Result<SemanticBindingFields> {
        match super::codec::tag(self)?.as_str() {
            "input" => Ok(SemanticBindingFields::Input {
                node_id: node_id_field(self, "node_id")?,
                field_path: field_path_field(self, "field_path")?,
                value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            }),
            "output" => Ok(SemanticBindingFields::Output {
                node_id: node_id_field(self, "node_id")?,
                output_ordinal: u32_field(self, "output_ordinal")?,
                value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            }),
            "fact" => Ok(SemanticBindingFields::Fact {
                node_id: node_id_field(self, "node_id")?,
                emission_ordinal: u32_field(self, "emission_ordinal")?,
                claim_ref: ValueRef::from_canonical_value(required_field(self, "claim_ref")?)?,
                subject_ref: ValueRef::from_canonical_value(required_field(self, "subject_ref")?)?,
                response_ref: ValueRef::from_canonical_value(required_field(
                    self,
                    "response_ref",
                )?)?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of one unresolved effect semantic-state component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEffectStateFields {
    /// Awaiting node occurrence.
    pub node_id: NodeId,
    /// Deterministic effect identity.
    pub effect_key: EffectKey,
    /// Immutable semantic request digest.
    pub request_digest: RequestDigest,
    /// Exact certified executor binding.
    pub executor_binding_ref: CapabilityBindingRef,
    /// Full retained input-manifest authority.
    pub input_manifest_ref: InputManifestRef,
    /// Full retained semantic-request authority.
    pub semantic_request_ref: ValueRef,
}

impl PendingEffectState {
    /// Constructs one complete unresolved-effect component.
    pub fn new(
        node_id: &NodeId,
        effect_key: &EffectKey,
        request_digest: &RequestDigest,
        executor_binding_ref: &CapabilityBindingRef,
        input_manifest_ref: &InputManifestRef,
        semantic_request_ref: &ValueRef,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("node_id", cv_string(node_id.as_str())),
            ("effect_key", cv_string(effect_key.as_str())),
            ("request_digest", cv_string(request_digest.as_str())),
            (
                "executor_binding_ref",
                executor_binding_ref.canonical_value()?,
            ),
            ("input_manifest_ref", input_manifest_ref.canonical_value()?),
            (
                "semantic_request_ref",
                semantic_request_ref.canonical_value()?,
            ),
        ])?)
    }

    /// Projects every complete unresolved-effect field.
    pub fn fields(&self) -> Result<PendingEffectStateFields> {
        Ok(PendingEffectStateFields {
            node_id: node_id_field(self, "node_id")?,
            effect_key: effect_key_field(self, "effect_key")?,
            request_digest: RequestDigest::parse(string_field(self, "request_digest")?)?,
            executor_binding_ref: CapabilityBindingRef::from_canonical_value(required_field(
                self,
                "executor_binding_ref",
            )?)?,
            input_manifest_ref: InputManifestRef::from_canonical_value(required_field(
                self,
                "input_manifest_ref",
            )?)?,
            semantic_request_ref: ValueRef::from_canonical_value(required_field(
                self,
                "semantic_request_ref",
            )?)?,
        })
    }
}

/// Frozen typed fields of the canonical run semantic-state preimage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSemanticStatePreimageFields {
    /// Certified spec whose deterministic order governs the state.
    pub spec_hash: SpecHash,
    /// Current semantic run phase.
    pub run_phase: RunPhase,
    /// Non-empty states in certified node order.
    pub ordered_node_states: Vec<NodeSemanticState>,
    /// Typed bindings in canonical JSON order.
    pub typed_bindings: Vec<SemanticBinding>,
    /// Unresolved effects in certified node order.
    pub pending_effects: Vec<PendingEffectState>,
    /// Optional certified public-output semantic component.
    pub public_output: Option<ValueRef>,
}

impl RunSemanticStatePreimage {
    /// Constructs the exact frozen outer semantic-state digest preimage.
    ///
    /// Callers must supply only the closed components produced by the shared
    /// structural reducer. Accepting annex-declared canonical subvalues here
    /// does not make this codec a semantic-state derivation authority.
    pub fn new(
        spec_hash: &SpecHash,
        run_phase: RunPhase,
        ordered_node_states: &[NodeSemanticState],
        typed_bindings: &[SemanticBinding],
        pending_effects: &[PendingEffectState],
        public_output: Option<&ValueRef>,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("spec_hash", cv_string(spec_hash.as_str())),
            (
                "digest_contract_version",
                CanonicalValue::String("mfm.run-semantic-state.v1".to_owned()),
            ),
            ("run_phase", cv_string(run_phase.as_str())),
            (
                "ordered_node_states",
                cv_array(
                    ordered_node_states
                        .iter()
                        .map(|value| value.canonical_value()),
                )?,
            ),
            (
                "typed_bindings",
                cv_array(typed_bindings.iter().map(|value| value.canonical_value()))?,
            ),
            (
                "pending_effects",
                cv_array(pending_effects.iter().map(|value| value.canonical_value()))?,
            ),
            (
                "public_output",
                public_output
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])?)
    }

    /// Projects every frozen outer semantic-state preimage field.
    pub fn fields(&self) -> Result<RunSemanticStatePreimageFields> {
        Ok(RunSemanticStatePreimageFields {
            spec_hash: spec_hash_field(self, "spec_hash")?,
            run_phase: RunPhase::parse(&string_field(self, "run_phase")?)?,
            ordered_node_states: array_field(self, "ordered_node_states")?
                .into_iter()
                .map(NodeSemanticState::from_canonical_value)
                .collect::<Result<_>>()?,
            typed_bindings: array_field(self, "typed_bindings")?
                .into_iter()
                .map(SemanticBinding::from_canonical_value)
                .collect::<Result<_>>()?,
            pending_effects: array_field(self, "pending_effects")?
                .into_iter()
                .map(PendingEffectState::from_canonical_value)
                .collect::<Result<_>>()?,
            public_output: nullable_field(self, "public_output")?
                .map(ValueRef::from_canonical_value)
                .transpose()?,
        })
    }

    /// Derives the frozen canonical run semantic-state digest.
    pub fn run_state_digest(&self) -> Result<RunSemanticStateDigest> {
        domain_digest("mfm.run-semantic-state.v1", self)
            .map(RunSemanticStateDigest::from_semantic_digest)
    }
}
