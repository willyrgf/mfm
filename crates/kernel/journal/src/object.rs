use mfm_canonical::CanonicalValue;
use mfm_ids::{
    ArtifactId, ContentDigest, ContentRef, EntryPointId, FieldPath, NodeId, ObjectEvidenceDigest,
    RunId, SchemaId, SemanticTypeId, SourceClosureDigest, StableId, StoreScopeId, TenantScopeId,
};

use super::codec::{
    array_field, artifact_id_field, content_digest_field, content_ref_from_canonical,
    cv_content_ref, cv_decimal, cv_string, define_schema_value, domain_digest,
    entry_point_id_field, field_path_field, object, required_field, schema_id_field,
    semantic_type_id_field, stable_id_field, store_scope_id_field, string_field,
    tenant_scope_id_field, u32_field, unsigned_field,
};
use super::{
    AuthorizationRef, CrossRunSourceRef, JournalError, RecordRef, Result, RetainedValueContract,
    SourceObjectRef,
};

define_schema_value! {
    /// Full producer-bound retained-object authority.
    pub struct ValueRef => "mfm.value-ref.v1";
    /// Logical producer binding embedded in a retained-object authority.
    pub struct ProducerBinding => "mfm.producer-binding.v1";
    /// Store- and tenant-scoped key of one configured value.
    pub struct ConfiguredValueKey => "mfm.configured-value-key.v1";
    /// Exact immutable configured-value authority resolved for one key.
    pub struct ConfiguredValueBinding => "mfm.configured-value-binding.v1";
    /// Exact object path and authority use within a candidate record.
    pub struct ObjectPathBinding => "mfm.object-path-binding.v1";
    /// Exact object admission mode requested by a candidate.
    pub struct ArtifactAdmissionIntent => "mfm.artifact-admission-intent.v1";
    /// Artifact semantic-identity preimage.
    pub struct ArtifactIdPreimage => "mfm.artifact-id-preimage.v1";
    /// Retained-object evidence preimage.
    pub struct ObjectEvidencePreimage => "mfm.object-evidence-preimage.v1";
    /// Complete canonical retained-object closure identity preimage.
    pub struct SourceClosurePreimage => "mfm.source-closure-preimage.v1";
}

/// One closed object-admission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtifactAdmissionMode {
    /// The exact object authority must already exist.
    RequireExisting,
    /// The exact bytes may be admitted or verified against existing authority.
    AdmitOrVerifyExact,
}

impl ArtifactAdmissionMode {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequireExisting => "require_existing",
            Self::AdmitOrVerifyExact => "admit_or_verify_exact",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "require_existing" => Ok(Self::RequireExisting),
            "admit_or_verify_exact" => Ok(Self::AdmitOrVerifyExact),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// How one candidate path obtains object authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityUse {
    /// The path consumes previously admitted authority.
    Preexisting,
    /// The path produces authority in the current commit.
    ProducedHere,
}

impl AuthorityUse {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preexisting => "preexisting",
            Self::ProducedHere => "produced_here",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "preexisting" => Ok(Self::Preexisting),
            "produced_here" => Ok(Self::ProducedHere),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Closed producer-binding variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProducerBindingKind {
    /// An exact field of the run-admission record.
    RunAdmission,
    /// A named slot produced by the admission being constructed.
    ThisAdmission,
    /// A field produced by the record being constructed.
    ThisRecord,
    /// A complete input tree assembled for one node occurrence.
    InputAssembly,
    /// A transition output identified without a future journal coordinate.
    TransitionOutput,
    /// A transition fact component identified without a future journal coordinate.
    TransitionFact,
    /// The run's certified public-output assembly.
    PublicOutputAssembly,
    /// One field from an admitted qualified support graph.
    QualifiedSupport,
    /// One field from an admitted configured value.
    ConfiguredValue,
    /// The result of an external authorization.
    ExternalObservation,
    /// An object retained by a closed source run.
    SourceRun,
}

/// Closed producer-binding variant and all of its typed fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProducerBindingFields {
    /// An exact field of the immutable run admission.
    RunAdmission {
        /// Exact admission record.
        record_ref: RecordRef,
        /// Exact admission field.
        field_path: FieldPath,
    },
    /// A logical slot produced by the admission being constructed.
    ThisAdmission {
        /// Closed admission slot.
        slot: StableId,
    },
    /// A field produced by the record being constructed.
    ThisRecord {
        /// Exact record field.
        field_path: FieldPath,
    },
    /// A complete input tree assembled for one exact node occurrence.
    InputAssembly {
        /// Run containing the node occurrence.
        run_id: RunId,
        /// Node whose exact input tree was assembled.
        node_id: NodeId,
    },
    /// An output produced by one exact node occurrence.
    TransitionOutput {
        /// Run containing the producer.
        run_id: RunId,
        /// Producing node occurrence.
        node_id: NodeId,
        /// Output ordinal within its settlement.
        output_ordinal: u32,
    },
    /// One retained component of a fact emitted by an exact node occurrence.
    TransitionFact {
        /// Run containing the producer.
        run_id: RunId,
        /// Producing node occurrence.
        node_id: NodeId,
        /// Emission ordinal within its settlement.
        emission_ordinal: u32,
        /// Exact retained component of the fact value.
        component: FactValueComponent,
    },
    /// The certified public-output assembly for one run.
    PublicOutputAssembly {
        /// Run whose public output was assembled.
        run_id: RunId,
    },
    /// One field from a qualification-scoped admitted support graph.
    QualifiedSupport {
        /// Exact semantic qualification scope.
        qualification_scope_id: SemanticTypeId,
        /// Exact field within the qualified support graph.
        field_path: FieldPath,
    },
    /// One field from the exact admitted configured value.
    ConfiguredValue {
        /// Physical store scope owning the configuration.
        store_scope_id: StoreScopeId,
        /// Tenant owning the configuration.
        tenant_scope_id: TenantScopeId,
        /// Published entry point whose target was resolved.
        entry_point_id: EntryPointId,
        /// Stable configured-value target.
        target: StableId,
    },
    /// A result observed under one exact authorization.
    ExternalObservation {
        /// Authorization that minted observation authority.
        authorization_ref: AuthorizationRef,
        /// Stable observation field whose bytes were retained.
        field_path: FieldPath,
    },
    /// An object retained by one closed source run.
    SourceRun {
        /// Complete source-run authority.
        source_ref: CrossRunSourceRef,
    },
}

/// One closed retained component of an emitted fact value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactValueComponent {
    /// Retained fact subject.
    Subject,
    /// Retained fact response.
    Response,
    /// Store-authored claim envelope.
    Claim,
}

impl FactValueComponent {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Subject => "subject",
            Self::Response => "response",
            Self::Claim => "claim",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "subject" => Ok(Self::Subject),
            "response" => Ok(Self::Response),
            "claim" => Ok(Self::Claim),
            _ => Err(JournalError::Projection),
        }
    }
}

impl ProducerBinding {
    /// Constructs a producer binding to one immutable admission field.
    pub fn run_admission(record_ref: &RecordRef, field_path: &FieldPath) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "run_admission",
            [
                ("record_ref", record_ref.canonical_value()?),
                ("field_path", cv_string(field_path.as_str())),
            ],
        )?)
    }

    /// Constructs a producer binding to a logical slot of this admission.
    pub fn this_admission(slot: &StableId) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "this_admission",
            [("slot", cv_string(slot.as_str()))],
        )?)
    }

    /// Constructs a producer binding to a field of this candidate record.
    pub fn this_record(field_path: &FieldPath) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "this_record",
            [("field_path", cv_string(field_path.as_str()))],
        )?)
    }

    /// Constructs a producer binding to the input assembly for one node occurrence.
    pub fn input_assembly(run_id: &RunId, node_id: &NodeId) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "input_assembly",
            [
                ("run_id", cv_string(run_id.as_str())),
                ("node_id", cv_string(node_id.as_str())),
            ],
        )?)
    }

    /// Constructs a producer binding to one transition output.
    pub fn transition_output(
        run_id: &RunId,
        node_id: &NodeId,
        output_ordinal: u32,
    ) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "transition_output",
            [
                ("run_id", cv_string(run_id.as_str())),
                ("node_id", cv_string(node_id.as_str())),
                (
                    "output_ordinal",
                    CanonicalValue::Unsigned(u64::from(output_ordinal)),
                ),
            ],
        )?)
    }

    /// Constructs a producer binding to one transition fact emission.
    pub fn transition_fact(
        run_id: &RunId,
        node_id: &NodeId,
        emission_ordinal: u32,
        component: FactValueComponent,
    ) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "transition_fact",
            [
                ("run_id", cv_string(run_id.as_str())),
                ("node_id", cv_string(node_id.as_str())),
                (
                    "emission_ordinal",
                    CanonicalValue::Unsigned(u64::from(emission_ordinal)),
                ),
                ("component", cv_string(component.as_str())),
            ],
        )?)
    }

    /// Constructs a producer binding to the certified public output of one run.
    pub fn public_output_assembly(run_id: &RunId) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "public_output_assembly",
            [("run_id", cv_string(run_id.as_str()))],
        )?)
    }

    /// Constructs a producer binding to one qualified support-graph field.
    pub fn qualified_support(
        qualification_scope_id: &SemanticTypeId,
        field_path: &FieldPath,
    ) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "qualified_support",
            [
                (
                    "qualification_scope_id",
                    cv_string(qualification_scope_id.as_str()),
                ),
                ("field_path", cv_string(field_path.as_str())),
            ],
        )?)
    }

    /// Constructs a producer binding to one exact configured-value target.
    pub fn configured_value(
        store_scope_id: &StoreScopeId,
        tenant_scope_id: &TenantScopeId,
        entry_point_id: &EntryPointId,
        target: &StableId,
    ) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "configured_value",
            [
                ("store_scope_id", cv_string(store_scope_id.as_str())),
                ("tenant_scope_id", cv_string(tenant_scope_id.as_str())),
                ("entry_point_id", cv_string(entry_point_id.as_str())),
                ("target", cv_string(target.as_str())),
            ],
        )?)
    }

    /// Constructs a producer binding to the result of one authorization.
    pub fn external_observation(
        authorization_ref: &AuthorizationRef,
        field_path: &FieldPath,
    ) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "external_observation",
            [
                ("authorization_ref", authorization_ref.canonical_value()?),
                ("field_path", cv_string(field_path.as_str())),
            ],
        )?)
    }

    /// Constructs a producer binding to one closed source run.
    pub fn source_run(source_ref: &CrossRunSourceRef) -> Result<Self> {
        Self::from_canonical_value(super::codec::tagged_object(
            "source_run",
            [("source_ref", source_ref.canonical_value()?)],
        )?)
    }

    /// Returns the closed producer-binding variant.
    pub fn kind(&self) -> Result<ProducerBindingKind> {
        match super::codec::tag(self)?.as_str() {
            "run_admission" => Ok(ProducerBindingKind::RunAdmission),
            "this_admission" => Ok(ProducerBindingKind::ThisAdmission),
            "this_record" => Ok(ProducerBindingKind::ThisRecord),
            "input_assembly" => Ok(ProducerBindingKind::InputAssembly),
            "transition_output" => Ok(ProducerBindingKind::TransitionOutput),
            "transition_fact" => Ok(ProducerBindingKind::TransitionFact),
            "public_output_assembly" => Ok(ProducerBindingKind::PublicOutputAssembly),
            "qualified_support" => Ok(ProducerBindingKind::QualifiedSupport),
            "configured_value" => Ok(ProducerBindingKind::ConfiguredValue),
            "external_observation" => Ok(ProducerBindingKind::ExternalObservation),
            "source_run" => Ok(ProducerBindingKind::SourceRun),
            _ => Err(super::JournalError::Projection),
        }
    }

    /// Projects the closed producer-binding variant and all of its typed fields.
    pub fn fields(&self) -> Result<ProducerBindingFields> {
        match self.kind()? {
            ProducerBindingKind::RunAdmission => Ok(ProducerBindingFields::RunAdmission {
                record_ref: RecordRef::from_canonical_value(required_field(self, "record_ref")?)?,
                field_path: field_path_field(self, "field_path")?,
            }),
            ProducerBindingKind::ThisAdmission => Ok(ProducerBindingFields::ThisAdmission {
                slot: stable_id_field(self, "slot")?,
            }),
            ProducerBindingKind::ThisRecord => Ok(ProducerBindingFields::ThisRecord {
                field_path: field_path_field(self, "field_path")?,
            }),
            ProducerBindingKind::InputAssembly => Ok(ProducerBindingFields::InputAssembly {
                run_id: super::codec::run_id_field(self, "run_id")?,
                node_id: super::codec::node_id_field(self, "node_id")?,
            }),
            ProducerBindingKind::TransitionOutput => Ok(ProducerBindingFields::TransitionOutput {
                run_id: super::codec::run_id_field(self, "run_id")?,
                node_id: super::codec::node_id_field(self, "node_id")?,
                output_ordinal: u32_field(self, "output_ordinal")?,
            }),
            ProducerBindingKind::TransitionFact => Ok(ProducerBindingFields::TransitionFact {
                run_id: super::codec::run_id_field(self, "run_id")?,
                node_id: super::codec::node_id_field(self, "node_id")?,
                emission_ordinal: u32_field(self, "emission_ordinal")?,
                component: FactValueComponent::parse(&string_field(self, "component")?)?,
            }),
            ProducerBindingKind::PublicOutputAssembly => {
                Ok(ProducerBindingFields::PublicOutputAssembly {
                    run_id: super::codec::run_id_field(self, "run_id")?,
                })
            }
            ProducerBindingKind::QualifiedSupport => Ok(ProducerBindingFields::QualifiedSupport {
                qualification_scope_id: semantic_type_id_field(self, "qualification_scope_id")?,
                field_path: field_path_field(self, "field_path")?,
            }),
            ProducerBindingKind::ConfiguredValue => Ok(ProducerBindingFields::ConfiguredValue {
                store_scope_id: store_scope_id_field(self, "store_scope_id")?,
                tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
                entry_point_id: entry_point_id_field(self, "entry_point_id")?,
                target: stable_id_field(self, "target")?,
            }),
            ProducerBindingKind::ExternalObservation => {
                Ok(ProducerBindingFields::ExternalObservation {
                    authorization_ref: AuthorizationRef::from_canonical_value(required_field(
                        self,
                        "authorization_ref",
                    )?)?,
                    field_path: field_path_field(self, "field_path")?,
                })
            }
            ProducerBindingKind::SourceRun => Ok(ProducerBindingFields::SourceRun {
                source_ref: CrossRunSourceRef::from_canonical_value(required_field(
                    self,
                    "source_ref",
                )?)?,
            }),
        }
    }
}

/// Typed fields of one admitted configured-value key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredValueKeyFields {
    /// Physical store scope owning the configured value.
    pub store_scope_id: StoreScopeId,
    /// Tenant owning the configured value.
    pub tenant_scope_id: TenantScopeId,
    /// Published entry point whose target is configured.
    pub entry_point_id: EntryPointId,
    /// Stable configured-value target.
    pub target: StableId,
}

impl ConfiguredValueKey {
    /// Constructs one exact store- and tenant-scoped configured-value key.
    pub fn new(
        store_scope_id: &StoreScopeId,
        tenant_scope_id: &TenantScopeId,
        entry_point_id: &EntryPointId,
        target: &StableId,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("store_scope_id", cv_string(store_scope_id.as_str())),
            ("tenant_scope_id", cv_string(tenant_scope_id.as_str())),
            ("entry_point_id", cv_string(entry_point_id.as_str())),
            ("target", cv_string(target.as_str())),
        ])?)
    }

    /// Projects every configured-value key field.
    pub fn fields(&self) -> Result<ConfiguredValueKeyFields> {
        Ok(ConfiguredValueKeyFields {
            store_scope_id: store_scope_id_field(self, "store_scope_id")?,
            tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
            entry_point_id: entry_point_id_field(self, "entry_point_id")?,
            target: stable_id_field(self, "target")?,
        })
    }
}

/// Typed fields of one exact configured-value binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredValueBindingFields {
    /// Exact admitted configured-value key.
    pub key: ConfiguredValueKey,
    /// Complete immutable configured-value authority.
    pub value_ref: ValueRef,
}

impl ConfiguredValueBinding {
    /// Constructs one exact immutable configured-value binding.
    pub fn new(key: &ConfiguredValueKey, value_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.configured-value-binding.v1".to_owned()),
            ),
            ("key", key.canonical_value()?),
            ("value_ref", value_ref.canonical_value()?),
        ])?)
    }

    /// Projects every configured-value binding field.
    pub fn fields(&self) -> Result<ConfiguredValueBindingFields> {
        Ok(ConfiguredValueBindingFields {
            key: ConfiguredValueKey::from_canonical_value(required_field(self, "key")?)?,
            value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
        })
    }
}

/// Typed fields of a full retained-value reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueRefFields {
    /// Artifact semantic identity.
    pub artifact_id: ArtifactId,
    /// Digest of exact retained bytes.
    pub content_digest: ContentDigest,
    /// Semantic object-evidence digest.
    pub evidence_hash: ObjectEvidenceDigest,
    /// Retained-value schema identity.
    pub schema_id: SchemaId,
    /// Semantic type identity.
    pub semantic_type_id: SemanticTypeId,
    /// Certified role of the value.
    pub role: StableId,
    /// Exact retained byte length.
    pub byte_length: u64,
    /// Reviewed media type.
    pub media_type: String,
    /// Exact contract used to rederive retained-object evidence.
    pub evidence_contract_ref: ContentRef,
    /// Logical producer authority.
    pub producer_binding: ProducerBinding,
}

impl ValueRef {
    /// Constructs and annex-validates a complete retained-value reference.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        artifact_id: &ArtifactId,
        content_digest: &ContentDigest,
        evidence_hash: &ObjectEvidenceDigest,
        schema_id: &SchemaId,
        semantic_type_id: &SemanticTypeId,
        role: &StableId,
        byte_length: u64,
        media_type: impl Into<String>,
        evidence_contract_ref: &ContentRef,
        producer_binding: &ProducerBinding,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("artifact_id", cv_string(artifact_id.as_str())),
            ("content_digest", cv_string(content_digest.as_str())),
            ("evidence_hash", cv_string(evidence_hash.as_str())),
            ("schema_id", cv_string(schema_id.as_str())),
            ("semantic_type_id", cv_string(semantic_type_id.as_str())),
            ("role", cv_string(role.as_str())),
            ("byte_length", cv_decimal(byte_length)?),
            ("media_type", CanonicalValue::String(media_type.into())),
            (
                "evidence_contract_ref",
                cv_content_ref(evidence_contract_ref)?,
            ),
            ("producer_binding", producer_binding.canonical_value()?),
        ])?)
    }

    /// Projects all authority-bearing retained-value fields.
    pub fn fields(&self) -> Result<ValueRefFields> {
        Ok(ValueRefFields {
            artifact_id: artifact_id_field(self, "artifact_id")?,
            content_digest: content_digest_field(self, "content_digest")?,
            evidence_hash: ObjectEvidenceDigest::parse(string_field(self, "evidence_hash")?)?,
            schema_id: schema_id_field(self, "schema_id")?,
            semantic_type_id: semantic_type_id_field(self, "semantic_type_id")?,
            role: stable_id_field(self, "role")?,
            byte_length: unsigned_field(self, "byte_length")?,
            media_type: string_field(self, "media_type")?,
            evidence_contract_ref: super::codec::content_ref_field(self, "evidence_contract_ref")?,
            producer_binding: ProducerBinding::from_canonical_value(required_field(
                self,
                "producer_binding",
            )?)?,
        })
    }

    /// Verifies this authority against one exact producer-independent contract.
    pub fn validate_contract(&self, contract: &RetainedValueContract) -> Result<()> {
        let value = self.fields()?;
        if contract.schema_id() == &value.schema_id
            && contract.semantic_type_id() == &value.semantic_type_id
            && contract.role() == &value.role
            && contract.media_type() == value.media_type
            && contract.evidence_contract_ref() == &value.evidence_contract_ref
        {
            Ok(())
        } else {
            Err(JournalError::ReferenceMismatch)
        }
    }
}

/// Typed fields of an object path binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectPathBindingFields {
    /// Candidate record ordinal.
    pub record_ordinal: u32,
    /// Exact canonical field path.
    pub field_path: FieldPath,
    /// Whether the path consumes or produces authority.
    pub authority_use: AuthorityUse,
    /// Complete producer-bound retained-value authority.
    pub value_ref: ValueRef,
    /// Repeated exact contract used to rederive object evidence.
    pub evidence_contract_ref: ContentRef,
}

impl ObjectPathBinding {
    /// Constructs and validates one exact object path binding.
    pub fn new(
        record_ordinal: u32,
        field_path: &FieldPath,
        authority_use: AuthorityUse,
        value_ref: &ValueRef,
        evidence_contract_ref: &ContentRef,
    ) -> Result<Self> {
        require_evidence_contract(value_ref, evidence_contract_ref)?;
        Self::from_canonical_value(object([
            (
                "record_ordinal",
                CanonicalValue::Unsigned(u64::from(record_ordinal)),
            ),
            ("field_path", cv_string(field_path.as_str())),
            (
                "authority_use",
                CanonicalValue::String(authority_use.as_str().to_owned()),
            ),
            ("value_ref", value_ref.canonical_value()?),
            (
                "evidence_contract_ref",
                cv_content_ref(evidence_contract_ref)?,
            ),
        ])?)
    }

    /// Projects all exact object path binding fields.
    pub fn fields(&self) -> Result<ObjectPathBindingFields> {
        let fields = ObjectPathBindingFields {
            record_ordinal: u32_field(self, "record_ordinal")?,
            field_path: field_path_field(self, "field_path")?,
            authority_use: AuthorityUse::parse(&string_field(self, "authority_use")?)?,
            value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            evidence_contract_ref: super::codec::content_ref_field(self, "evidence_contract_ref")?,
        };
        require_evidence_contract(&fields.value_ref, &fields.evidence_contract_ref)?;
        Ok(fields)
    }
}

/// Typed fields of an artifact admission intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactAdmissionIntentFields {
    /// Complete producer-bound retained-value authority.
    pub value_ref: ValueRef,
    /// Repeated exact contract used to rederive object evidence.
    pub evidence_contract_ref: ContentRef,
    /// Closed admission mode.
    pub mode: ArtifactAdmissionMode,
}

impl ArtifactAdmissionIntent {
    /// Constructs and validates one exact artifact admission intent.
    pub fn new(
        value_ref: &ValueRef,
        evidence_contract_ref: &ContentRef,
        mode: ArtifactAdmissionMode,
    ) -> Result<Self> {
        require_evidence_contract(value_ref, evidence_contract_ref)?;
        Self::from_canonical_value(object([
            ("value_ref", value_ref.canonical_value()?),
            (
                "evidence_contract_ref",
                cv_content_ref(evidence_contract_ref)?,
            ),
            ("mode", CanonicalValue::String(mode.as_str().to_owned())),
        ])?)
    }

    /// Projects all exact admission-intent fields.
    pub fn fields(&self) -> Result<ArtifactAdmissionIntentFields> {
        let fields = ArtifactAdmissionIntentFields {
            value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            evidence_contract_ref: super::codec::content_ref_field(self, "evidence_contract_ref")?,
            mode: ArtifactAdmissionMode::parse(&string_field(self, "mode")?)?,
        };
        require_evidence_contract(&fields.value_ref, &fields.evidence_contract_ref)?;
        Ok(fields)
    }
}

impl ArtifactIdPreimage {
    /// Constructs the frozen artifact-identity preimage.
    pub fn new(
        schema_id: &SchemaId,
        content_digest: &ContentDigest,
        semantic_type_id: &SemanticTypeId,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("schema_id", cv_string(schema_id.as_str())),
            ("content_digest", cv_string(content_digest.as_str())),
            ("semantic_type_id", cv_string(semantic_type_id.as_str())),
        ])?)
    }

    /// Derives the frozen artifact semantic identity.
    pub fn artifact_id(&self) -> Result<ArtifactId> {
        let digest = domain_digest("mfm.artifact-id.v1", self)?;
        Ok(ArtifactId::from_digest(
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            *digest.digest(),
        ))
    }
}

impl ObjectEvidencePreimage {
    /// Constructs the frozen retained-object evidence preimage.
    pub fn new(
        artifact_id: &ArtifactId,
        content_digest: &ContentDigest,
        schema_id: &SchemaId,
        byte_length: u64,
        media_type: impl Into<String>,
        evidence_contract_ref: &ContentRef,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("artifact_id", cv_string(artifact_id.as_str())),
            ("content_digest", cv_string(content_digest.as_str())),
            ("schema_id", cv_string(schema_id.as_str())),
            ("byte_length", cv_decimal(byte_length)?),
            ("media_type", CanonicalValue::String(media_type.into())),
            (
                "evidence_contract_ref",
                cv_content_ref(evidence_contract_ref)?,
            ),
        ])?)
    }

    /// Derives the frozen object-evidence semantic digest.
    pub fn evidence_hash(&self) -> Result<ObjectEvidenceDigest> {
        domain_digest("mfm.object-evidence.v1", self)
            .map(ObjectEvidenceDigest::from_semantic_digest)
    }
}

/// Typed fields of the complete retained-object source closure preimage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceClosurePreimageFields {
    /// Root source manifest whose complete closure is identified.
    pub root_source_manifest_ref: ContentRef,
    /// Complete canonically ordered unique source-manifest dependencies.
    pub ordered_dependency_refs: Vec<ContentRef>,
    /// Complete canonically ordered unique retained-object authorities.
    pub ordered_object_refs: Vec<ValueRef>,
}

impl SourceClosurePreimage {
    /// Constructs one complete canonical retained-object source closure preimage.
    pub fn new(
        root_source_manifest_ref: &ContentRef,
        ordered_dependency_refs: &[ContentRef],
        ordered_object_refs: &[ValueRef],
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "root_source_manifest_ref",
                cv_content_ref(root_source_manifest_ref)?,
            ),
            (
                "ordered_dependency_refs",
                super::codec::cv_array(ordered_dependency_refs.iter().map(cv_content_ref))?,
            ),
            (
                "ordered_object_refs",
                super::codec::cv_array(
                    ordered_object_refs
                        .iter()
                        .map(|value| value.canonical_value()),
                )?,
            ),
        ])?)
    }

    /// Projects every complete source-closure preimage field.
    pub fn fields(&self) -> Result<SourceClosurePreimageFields> {
        Ok(SourceClosurePreimageFields {
            root_source_manifest_ref: super::codec::content_ref_field(
                self,
                "root_source_manifest_ref",
            )?,
            ordered_dependency_refs: array_field(self, "ordered_dependency_refs")?
                .into_iter()
                .map(content_ref_from_canonical)
                .collect::<Result<_>>()?,
            ordered_object_refs: array_field(self, "ordered_object_refs")?
                .into_iter()
                .map(ValueRef::from_canonical_value)
                .collect::<Result<_>>()?,
        })
    }

    /// Derives the frozen complete source-closure semantic digest.
    pub fn source_closure_digest(&self) -> Result<SourceClosureDigest> {
        domain_digest("mfm.source-closure.v1", self).map(SourceClosureDigest::from_semantic_digest)
    }
}

/// Typed fields common to a source object reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObjectRefFields {
    /// Artifact identity.
    pub artifact_id: ArtifactId,
    /// Digest of exact retained bytes.
    pub content_digest: ContentDigest,
    /// Object-evidence digest.
    pub evidence_hash: ObjectEvidenceDigest,
    /// Retained-value schema identity.
    pub schema_id: SchemaId,
    /// Semantic type identity.
    pub semantic_type_id: SemanticTypeId,
    /// Certified role.
    pub role: StableId,
    /// Exact byte length.
    pub byte_length: u64,
    /// Reviewed media type.
    pub media_type: String,
    /// Exact contract used to rederive retained-object evidence.
    pub evidence_contract_ref: ContentRef,
}

impl SourceObjectRef {
    /// Projects every field carried directly by the source object reference.
    pub fn fields(&self) -> Result<SourceObjectRefFields> {
        Ok(SourceObjectRefFields {
            artifact_id: artifact_id_field(self, "artifact_id")?,
            content_digest: content_digest_field(self, "content_digest")?,
            evidence_hash: ObjectEvidenceDigest::parse(string_field(self, "evidence_hash")?)?,
            schema_id: schema_id_field(self, "schema_id")?,
            semantic_type_id: semantic_type_id_field(self, "semantic_type_id")?,
            role: stable_id_field(self, "role")?,
            byte_length: unsigned_field(self, "byte_length")?,
            media_type: string_field(self, "media_type")?,
            evidence_contract_ref: super::codec::content_ref_field(self, "evidence_contract_ref")?,
        })
    }
}

fn require_evidence_contract(value_ref: &ValueRef, expected: &ContentRef) -> Result<()> {
    if value_ref.fields()?.evidence_contract_ref == *expected {
        Ok(())
    } else {
        Err(JournalError::ReferenceMismatch)
    }
}
