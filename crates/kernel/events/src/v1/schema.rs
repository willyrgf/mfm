use super::*;

/// Field cardinality in an event schema descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventFieldCardinality {
    /// Required scalar field.
    Required,
    /// Optional field.
    Optional,
    /// Repeated field.
    Repeated,
}

impl EventFieldCardinality {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::Repeated => "repeated",
        }
    }
}

fn schema_field(
    name: &'static str,
    type_name: &'static str,
    cardinality: EventFieldCardinality,
) -> serde_json::Value {
    serde_json::json!({
        "cardinality": cardinality.as_str(),
        "name": name,
        "type": event_type_schema(type_name),
    })
}

fn scalar_type(type_name: &'static str) -> serde_json::Value {
    serde_json::json!({
        "kind": "scalar",
        "name": type_name,
    })
}

fn visible_ascii_256_type(type_name: &'static str) -> serde_json::Value {
    serde_json::json!({
        "kind": "mfm_ids_visible_ascii",
        "max_len": 256,
        "name": type_name,
        "non_empty": true,
    })
}

fn stable_author_key_type(type_name: &'static str) -> serde_json::Value {
    serde_json::json!({
        "kind": "mfm_ids_stable_author_key",
        "name": type_name,
        "persisted_as": "string",
    })
}

fn public_field_path_type() -> serde_json::Value {
    serde_json::json!({
        "kind": "mfm_ids_field_path",
        "name": "PublicFieldPath",
        "persisted_as": "string",
    })
}

fn resource_namespace_type() -> serde_json::Value {
    serde_json::json!({
        "kind": "mfm_ids_resource_namespace",
        "name": "ResourceNamespace",
        "persisted_as": "string",
    })
}

fn mfm_identity_type(type_name: &'static str) -> serde_json::Value {
    serde_json::json!({
        "kind": "mfm_ids_identity",
        "name": type_name,
        "persisted_as": "string",
    })
}

fn mfm_version_type(type_name: &'static str) -> serde_json::Value {
    serde_json::json!({
        "kind": "mfm_ids_version",
        "name": type_name,
        "persisted_as": "string",
    })
}

fn external_type(type_name: &'static str) -> serde_json::Value {
    serde_json::json!({
        "kind": "external_contract_type",
        "name": type_name,
    })
}

fn struct_type(name: &'static str, fields: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "fields": fields,
        "kind": "struct",
        "name": name,
    })
}

fn unit_enum_type(name: &'static str, variants: &[&'static str]) -> serde_json::Value {
    serde_json::json!({
        "kind": "enum",
        "name": name,
        "variants": variants
            .iter()
            .map(|variant| serde_json::json!({
                "fields": [],
                "name": variant,
            }))
            .collect::<Vec<_>>(),
    })
}

fn artifact_role_schema_variants() -> Vec<&'static str> {
    ArtifactRole::ALL.iter().map(|role| role.as_str()).collect()
}

fn enum_type(name: &'static str, variants: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "kind": "enum",
        "name": name,
        "variants": variants,
    })
}

fn enum_variant(name: &'static str, fields: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "fields": fields,
        "name": name,
    })
}

fn event_type_schema(type_name: &'static str) -> serde_json::Value {
    match type_name {
        "bool" | "u32" | "u64" => scalar_type(type_name),
        "String" | "PlainCanonicalJsonBytes" => serde_json::json!({
            "kind": "string",
            "name": type_name,
        }),
        "AdapterKind" | "ArtifactId" | "AttemptId" | "CapabilityKind" | "CellId"
        | "ContentDigest" | "ContextRef" | "DescriptorId" | "EffectKind" | "EventId" | "NodeId"
        | "OperationKind" | "RunId" | "SchemaId" | "ScopeId" | "SeedId" | "SemanticTypeId"
        | "SideEffectPairId" | "SpecHash" | "StateKind" | "StoreScopeId" => {
            mfm_identity_type(type_name)
        }
        "AdapterVersion" | "CapabilityVersion" | "LoweringVersion" | "OperationVersion"
        | "SpecVersion" | "StateVersion" => mfm_version_type(type_name),
        "DigestAlgorithm" => unit_enum_type("DigestAlgorithm", &["sha256-jcs-v1"]),
        "SideEffectPairRole" => unit_enum_type("SideEffectPairRole", &["submit", "verify"]),
        "ResourceLaneReleaseAuthority" => unit_enum_type(
            "ResourceLaneReleaseAuthority",
            &["verify_terminal", "manual_resolution"],
        ),
        "MediaType"
        | "CanonicalizerIdentity"
        | "ContextResourceKind"
        | "ContextStage"
        | "RendererVersion" => visible_ascii_256_type(type_name),
        "PublicFieldPath" => public_field_path_type(),
        "ResourceNamespace" => resource_namespace_type(),
        "RendererKind" => stable_author_key_type(type_name),
        "RunnerFactoryId"
        | "NixDerivationHash"
        | "NixOutputHash"
        | "SideEffectLedgerKey"
        | "ResourceKey"
        | "ResourceLaneClaimId"
        | "ResourceLaneReleaseId"
        | "ResourceLaneReleaseReason"
        | "RunnerInvocationId"
        | "IdempotencyKeyRef"
        | "ReplayVerifierId"
        | "AmbiguityCode"
        | "ErrorCode"
        | "ClaimFencingToken"
        | "EntryPointOpId" => visible_ascii_256_type(type_name),
        "EntryPointLaunchEvidence" => struct_type(
            "EntryPointLaunchEvidence",
            vec![
                schema_field(
                    "resolved_op_id",
                    "EntryPointOpId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "entry_point_registry_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "FactKind" | "FactKey" => visible_ascii_256_type(type_name),
        "FactAudience" => unit_enum_type("FactAudience", &["control", "platform"]),
        "FactVisibilityScope" => unit_enum_type("FactVisibilityScope", &["default"]),
        "FactVisibility" => enum_type(
            "FactVisibility",
            vec![
                enum_variant("run_private", Vec::new()),
                enum_variant(
                    "indexed",
                    vec![
                        schema_field("audience", "FactAudience", EventFieldCardinality::Required),
                        schema_field(
                            "scope",
                            "FactVisibilityScope",
                            EventFieldCardinality::Required,
                        ),
                    ],
                ),
            ],
        ),
        "FactSubjectEvidence" => struct_type(
            "FactSubjectEvidence",
            vec![
                schema_field(
                    "fact_subject_namespace_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "subject_material",
                    "PlainCanonicalJsonBytes",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "subject_material_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field("fact_key", "FactKey", EventFieldCardinality::Required),
            ],
        ),
        "FactRequestEvidence" => struct_type(
            "FactRequestEvidence",
            vec![
                schema_field(
                    "request_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "request_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "FactResponseEvidence" => struct_type(
            "FactResponseEvidence",
            vec![
                schema_field(
                    "response_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "response_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                schema_field(
                    "artifact_evidence_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "FactProducerProvenance" => struct_type(
            "FactProducerProvenance",
            vec![
                schema_field(
                    "capability_kind",
                    "CapabilityKind",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "capability_version",
                    "CapabilityVersion",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "adapter_kind",
                    "AdapterKind",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "adapter_version",
                    "AdapterVersion",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "FactClaim" => struct_type(
            "FactClaim",
            vec![
                schema_field(
                    "visibility",
                    "FactVisibility",
                    EventFieldCardinality::Required,
                ),
                schema_field("fact_kind", "FactKind", EventFieldCardinality::Required),
                schema_field(
                    "fact_descriptor_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "subject",
                    "FactSubjectEvidence",
                    EventFieldCardinality::Required,
                ),
                schema_field("observed_at", "String", EventFieldCardinality::Optional),
                schema_field(
                    "request",
                    "FactRequestEvidence",
                    EventFieldCardinality::Optional,
                ),
                schema_field(
                    "response",
                    "FactResponseEvidence",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "producer",
                    "FactProducerProvenance",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "RunIdentityMaterialV1" => struct_type(
            "RunIdentityMaterialV1",
            vec![
                schema_field(
                    "certified_spec_hash",
                    "SpecHash",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "store_scope_id",
                    "StoreScopeId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "invocation_key_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "DescriptorIdentity" => enum_type(
            "DescriptorIdentity",
            vec![
                enum_variant(
                    "state",
                    vec![schema_field(
                        "identity",
                        "StateDescriptorIdentity",
                        EventFieldCardinality::Required,
                    )],
                ),
                enum_variant(
                    "operation",
                    vec![schema_field(
                        "identity",
                        "OperationDescriptorIdentity",
                        EventFieldCardinality::Required,
                    )],
                ),
                enum_variant(
                    "renderer",
                    vec![schema_field(
                        "identity",
                        "RendererDescriptorIdentity",
                        EventFieldCardinality::Required,
                    )],
                ),
            ],
        ),
        "StateDescriptorIdentity" => struct_type(
            "StateDescriptorIdentity",
            vec![
                schema_field(
                    "descriptor_id",
                    "DescriptorId",
                    EventFieldCardinality::Required,
                ),
                schema_field("state_kind", "StateKind", EventFieldCardinality::Required),
                schema_field(
                    "state_version",
                    "StateVersion",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "config_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "input_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "output_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "output_semantic_type_id",
                    "SemanticTypeId",
                    EventFieldCardinality::Required,
                ),
                schema_field("effect_kind", "EffectKind", EventFieldCardinality::Required),
                schema_field(
                    "capabilities",
                    "CapabilitySetDescriptor",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "emitted_fact_descriptors",
                    "FactDescriptorRef",
                    EventFieldCardinality::Repeated,
                ),
            ],
        ),
        "FactDescriptorRef" => struct_type(
            "FactDescriptorRef",
            vec![schema_field(
                "descriptor_hash",
                "ContentDigest",
                EventFieldCardinality::Required,
            )],
        ),
        "OperationDescriptorIdentity" => struct_type(
            "OperationDescriptorIdentity",
            vec![
                schema_field(
                    "descriptor_id",
                    "DescriptorId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "operation_kind",
                    "OperationKind",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "operation_version",
                    "OperationVersion",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "config_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "input_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "output_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field("expansion_abi", "String", EventFieldCardinality::Required),
            ],
        ),
        "RendererDescriptorIdentity" => struct_type(
            "RendererDescriptorIdentity",
            vec![
                schema_field(
                    "descriptor_id",
                    "DescriptorId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "renderer_kind",
                    "RendererKind",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "renderer_version",
                    "RendererVersion",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "public_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "canonicalizer_identity",
                    "CanonicalizerIdentity",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "CapabilitySetDescriptor" => struct_type(
            "CapabilitySetDescriptor",
            vec![schema_field(
                "capabilities",
                "CapabilityDescriptor",
                EventFieldCardinality::Repeated,
            )],
        ),
        "CapabilityDescriptor" => struct_type(
            "CapabilityDescriptor",
            vec![
                schema_field("kind", "CapabilityKind", EventFieldCardinality::Required),
                schema_field(
                    "version",
                    "CapabilityVersion",
                    EventFieldCardinality::Required,
                ),
                schema_field("role", "CapabilityRole", EventFieldCardinality::Required),
                schema_field("name", "String", EventFieldCardinality::Required),
            ],
        ),
        "CapabilityRole" => unit_enum_type(
            "CapabilityRole",
            &[
                "read_external",
                "managed_platform_write",
                "support",
                "external_mutation_authority",
            ],
        ),
        "SeedCellRef" => struct_type(
            "SeedCellRef",
            vec![
                schema_field("seed_id", "SeedId", EventFieldCardinality::Required),
                schema_field("cell_id", "CellId", EventFieldCardinality::Required),
                schema_field("scope_id", "ScopeId", EventFieldCardinality::Required),
                schema_field(
                    "semantic_type_id",
                    "SemanticTypeId",
                    EventFieldCardinality::Required,
                ),
                schema_field("schema_id", "SchemaId", EventFieldCardinality::Required),
                schema_field("digest", "ContentDigest", EventFieldCardinality::Required),
                schema_field(
                    "seed_artifact",
                    "ArtifactEvidenceRef",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "RunArtifactEvidenceRef" => struct_type(
            "RunArtifactEvidenceRef",
            vec![
                schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                schema_field("role", "ArtifactRole", EventFieldCardinality::Required),
                schema_field("schema_id", "SchemaId", EventFieldCardinality::Optional),
                schema_field(
                    "semantic_type_id",
                    "SemanticTypeId",
                    EventFieldCardinality::Optional,
                ),
                schema_field(
                    "content_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "evidence_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field("byte_len", "u64", EventFieldCardinality::Required),
                schema_field("media_type", "MediaType", EventFieldCardinality::Required),
            ],
        ),
        "ExecutableIdentity" => struct_type(
            "ExecutableIdentity",
            vec![
                schema_field(
                    "factory_id",
                    "RunnerFactoryId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "cargo_package_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "binary_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "nix_derivation_hash",
                    "NixDerivationHash",
                    EventFieldCardinality::Optional,
                ),
                schema_field(
                    "nix_output_hash",
                    "NixOutputHash",
                    EventFieldCardinality::Optional,
                ),
            ],
        ),
        "ArtifactEvidenceRef" => struct_type(
            "ArtifactEvidenceRef",
            vec![
                schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                schema_field("role", "ArtifactRole", EventFieldCardinality::Required),
                schema_field("schema_id", "SchemaId", EventFieldCardinality::Required),
                schema_field(
                    "semantic_type_id",
                    "SemanticTypeId",
                    EventFieldCardinality::Optional,
                ),
                schema_field(
                    "content_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "evidence_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field("byte_len", "u64", EventFieldCardinality::Required),
                schema_field("media_type", "MediaType", EventFieldCardinality::Required),
            ],
        ),
        "ArtifactRole" => unit_enum_type("ArtifactRole", &artifact_role_schema_variants()),
        "ResourceKeyEvidence" => struct_type(
            "ResourceKeyEvidence",
            vec![
                schema_field(
                    "namespace",
                    "ResourceNamespace",
                    EventFieldCardinality::Required,
                ),
                schema_field("key_schema_id", "SchemaId", EventFieldCardinality::Required),
                schema_field("key", "ResourceKey", EventFieldCardinality::Required),
            ],
        ),
        "ResourceTouchedSetEvidence" => struct_type(
            "ResourceTouchedSetEvidence",
            vec![
                schema_field(
                    "namespace",
                    "ResourceNamespace",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "evidence_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "evidence_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "evidence_artifact_id",
                    "ArtifactId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "evidence_artifact_evidence_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "NamedTypedCellRef" => struct_type(
            "NamedTypedCellRef",
            vec![
                schema_field(
                    "public_field_path",
                    "PublicFieldPath",
                    EventFieldCardinality::Required,
                ),
                schema_field("cell_id", "CellId", EventFieldCardinality::Required),
                schema_field("producer", "CellProducer", EventFieldCardinality::Required),
                schema_field("scope_id", "ScopeId", EventFieldCardinality::Required),
                schema_field(
                    "semantic_type_id",
                    "SemanticTypeId",
                    EventFieldCardinality::Required,
                ),
                schema_field("schema_id", "SchemaId", EventFieldCardinality::Required),
                schema_field(
                    "value_lineage",
                    "ValueLineageRef",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "content_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                schema_field(
                    "evidence_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "CellProducer" => enum_type(
            "CellProducer",
            vec![
                enum_variant(
                    "node",
                    vec![schema_field(
                        "node_id",
                        "NodeId",
                        EventFieldCardinality::Required,
                    )],
                ),
                enum_variant(
                    "seed",
                    vec![schema_field(
                        "seed_id",
                        "SeedId",
                        EventFieldCardinality::Required,
                    )],
                ),
            ],
        ),
        "ValueLineageRef" => struct_type(
            "ValueLineageRef",
            vec![schema_field(
                "lineage_digest",
                "ContentDigest",
                EventFieldCardinality::Required,
            )],
        ),
        "ContextProducerSpec" => struct_type(
            "ContextProducerSpec",
            vec![
                schema_field(
                    "producer_descriptor_ids",
                    "DescriptorId",
                    EventFieldCardinality::Repeated,
                ),
                schema_field(
                    "seed_producers_allowed",
                    "bool",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "CellContextSpec" => enum_type(
            "CellContextSpec",
            vec![
                enum_variant("no_context", Vec::new()),
                enum_variant(
                    "bound",
                    vec![
                        schema_field("context_ref", "ContextRef", EventFieldCardinality::Required),
                        schema_field(
                            "resource_kind",
                            "ContextResourceKind",
                            EventFieldCardinality::Required,
                        ),
                        schema_field("stage", "ContextStage", EventFieldCardinality::Required),
                        schema_field(
                            "producer",
                            "ContextProducerSpec",
                            EventFieldCardinality::Required,
                        ),
                    ],
                ),
            ],
        ),
        "MfmErrorInfo" => struct_type(
            "MfmErrorInfo",
            vec![
                schema_field("code", "ErrorCode", EventFieldCardinality::Required),
                schema_field("category", "ErrorCategory", EventFieldCardinality::Required),
                schema_field("retryable", "bool", EventFieldCardinality::Required),
                schema_field("safe_message", "String", EventFieldCardinality::Required),
                schema_field(
                    "public_details",
                    "RedactedJson",
                    EventFieldCardinality::Optional,
                ),
                schema_field(
                    "diagnostic_ref",
                    "ArtifactEvidenceRef",
                    EventFieldCardinality::Optional,
                ),
            ],
        ),
        "ErrorCategory" => unit_enum_type(
            "ErrorCategory",
            &[
                "planning",
                "validation",
                "capability",
                "side_effect",
                "runtime",
                "storage",
                "cancelled",
            ],
        ),
        "RedactedJson" => struct_type(
            "RedactedJson",
            vec![schema_field(
                "content_digest",
                "ContentDigest",
                EventFieldCardinality::Required,
            )],
        ),
        "SkipReason" => struct_type(
            "SkipReason",
            vec![
                schema_field("code", "ErrorCode", EventFieldCardinality::Required),
                schema_field("safe_message", "String", EventFieldCardinality::Required),
            ],
        ),
        "RetentionRef" => struct_type(
            "RetentionRef",
            vec![
                schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                schema_field("role", "ArtifactRole", EventFieldCardinality::Required),
                schema_field(
                    "evidence_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "content_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "RetentionReason" => unit_enum_type(
            "RetentionReason",
            &[
                "run_admitted",
                "runtime_evidence",
                "public_output",
                "manifest_projection",
            ],
        ),
        "PublicOutputCompletionEvidence" => struct_type(
            "PublicOutputCompletionEvidence",
            vec![
                schema_field(
                    "public_output_schema_id",
                    "SchemaId",
                    EventFieldCardinality::Required,
                ),
                schema_field(
                    "public_output_event_id",
                    "EventId",
                    EventFieldCardinality::Required,
                ),
            ],
        ),
        "SideEffectLedgerPurpose" => enum_type(
            "SideEffectLedgerPurpose",
            vec![
                enum_variant("forward", Vec::new()),
                enum_variant(
                    "remediation",
                    vec![schema_field(
                        "forward_pair_id",
                        "SideEffectPairId",
                        EventFieldCardinality::Required,
                    )],
                ),
            ],
        ),
        "ManualResolutionOutcome" => unit_enum_type(
            "ManualResolutionOutcome",
            &["confirm_remediated", "fail_without_acdc_claim"],
        ),
        "ManualResolutionNote" => struct_type(
            "ManualResolutionNote",
            vec![schema_field(
                "text",
                "String",
                EventFieldCardinality::Required,
            )],
        ),
        "RunCompletionOutcome" => enum_type(
            "RunCompletionOutcome",
            vec![
                enum_variant(
                    "completed",
                    vec![schema_field(
                        "public_output",
                        "PublicOutputCompletionEvidence",
                        EventFieldCardinality::Required,
                    )],
                ),
                enum_variant("compensated", Vec::new()),
                enum_variant("manually_resolved", Vec::new()),
                enum_variant("failed_without_acdc_claim", Vec::new()),
            ],
        ),
        "FailurePhase" => unit_enum_type(
            "FailurePhase",
            &["before_invocation_started", "after_not_submitted_proven"],
        ),
        name => external_type(name),
    }
}

/// Field descriptor for a typed event payload schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventFieldDescriptor {
    /// Persisted field name.
    pub name: &'static str,
    /// Field type name expanded into the hash-defining structural schema.
    pub type_name: &'static str,
    /// Field cardinality.
    pub cardinality: EventFieldCardinality,
}

impl EventFieldDescriptor {
    const fn required(name: &'static str, type_name: &'static str) -> Self {
        Self {
            name,
            type_name,
            cardinality: EventFieldCardinality::Required,
        }
    }

    const fn optional(name: &'static str, type_name: &'static str) -> Self {
        Self {
            name,
            type_name,
            cardinality: EventFieldCardinality::Optional,
        }
    }

    const fn repeated(name: &'static str, type_name: &'static str) -> Self {
        Self {
            name,
            type_name,
            cardinality: EventFieldCardinality::Repeated,
        }
    }

    fn json(self) -> serde_json::Value {
        schema_field(self.name, self.type_name, self.cardinality)
    }
}

/// Hash-defining event schema descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSchemaDescriptor {
    /// Stable schema name.
    pub schema_name: &'static str,
    /// Stable Rust payload path.
    pub rust_type_path: &'static str,
    /// Schema version.
    pub schema_version: &'static str,
    /// Event fields.
    pub fields: &'static [EventFieldDescriptor],
}

impl EventSchemaDescriptor {
    /// Returns canonical JSON bytes for the hash-defining event schema descriptor.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(serde_json::json!({
            "canonicalization": DigestAlgorithm::Sha256JcsV1.as_str(),
            "fields": self.fields.iter().copied().map(EventFieldDescriptor::json).collect::<Vec<_>>(),
            "schema_family": "mfm.kernel.event",
            "schema_name": self.schema_name,
            "schema_version": self.schema_version,
        }))
    }

    /// Returns the schema id derived from this event schema descriptor.
    pub fn schema_id(&self) -> Result<SchemaId> {
        let digest = self.canonical_json()?.digest_bytes();
        Ok(SchemaId::new(
            self.schema_name,
            self.schema_version,
            DigestAlgorithm::Sha256JcsV1,
            digest,
        )?)
    }
}

/// Returns every closed v1 event schema descriptor.
pub fn all_event_schema_descriptors() -> Vec<EventSchemaDescriptor> {
    vec![
        RUN_ADMITTED_SCHEMA,
        STATE_ATTEMPT_STARTED_SCHEMA,
        FACT_RECORDED_SCHEMA,
        ARTIFACT_REFERENCED_SCHEMA,
        CELL_PRODUCED_SCHEMA,
        CELL_SKIPPED_SCHEMA,
        SIDE_EFFECT_INTENT_PERSISTED_SCHEMA,
        SIDE_EFFECT_CLAIMED_SCHEMA,
        SIDE_EFFECT_CLAIM_TAKEN_OVER_SCHEMA,
        RESOURCE_LANE_CLAIMED_SCHEMA,
        RESOURCE_LANE_CLAIM_INTENT_SCHEMA,
        SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA,
        SIDE_EFFECT_INVOCATION_STARTED_SCHEMA,
        SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA,
        SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA,
        SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA,
        SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA,
        SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA,
        SIDE_EFFECT_AMBIGUOUS_SCHEMA,
        SIDE_EFFECT_FAILED_SCHEMA,
        RESOURCE_LANE_RELEASED_SCHEMA,
        RESOURCE_LANE_RELEASE_INTENT_SCHEMA,
        PUBLIC_OUTPUT_PRODUCED_SCHEMA,
        PUBLIC_OUTPUT_RENDER_FAILED_SCHEMA,
        STATE_ATTEMPT_COMPLETED_SCHEMA,
        STATE_ATTEMPT_INTERRUPTED_SCHEMA,
        STATE_ATTEMPT_FAILED_SCHEMA,
        MANUAL_RESOLUTION_RECORDED_SCHEMA,
        RUN_COMPLETED_SCHEMA,
        RETENTION_REFS_APPENDED_SCHEMA,
        RETENTION_MANIFEST_PROJECTED_SCHEMA,
    ]
}

macro_rules! fields {
        ($($field:expr),+ $(,)?) => {
            &[$($field),+]
        };
    }

pub(super) const RUN_ADMITTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.run_admitted",
    rust_type_path: "mfm_events::v1::RunAdmitted",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("run_id", "RunId"),
        EventFieldDescriptor::required("identity_material", "RunIdentityMaterialV1"),
        EventFieldDescriptor::required("entry_point", "EntryPointLaunchEvidence"),
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("spec_artifact", "RunArtifactEvidenceRef"),
        EventFieldDescriptor::required("certificate_artifact", "RunArtifactEvidenceRef"),
        EventFieldDescriptor::repeated("config_artifacts", "RunArtifactEvidenceRef"),
        EventFieldDescriptor::repeated("fact_descriptor_artifacts", "RunArtifactEvidenceRef"),
        EventFieldDescriptor::required("spec_version", "SpecVersion"),
        EventFieldDescriptor::required("lowering_version", "LoweringVersion"),
        EventFieldDescriptor::required("public_output_schema_id", "SchemaId"),
        EventFieldDescriptor::repeated("descriptor_identities", "DescriptorIdentity"),
        EventFieldDescriptor::repeated("runner_executables", "ExecutableIdentity"),
        EventFieldDescriptor::repeated("adapter_executables", "ExecutableIdentity"),
        EventFieldDescriptor::required("admitted_binding_digest", "ContentDigest"),
        EventFieldDescriptor::required("canonicalizer_identity", "CanonicalizerIdentity"),
        EventFieldDescriptor::repeated("seed_cells", "SeedCellRef"),
    ],
};

pub(super) const STATE_ATTEMPT_STARTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.state_attempt_started",
    rust_type_path: "mfm_events::v1::StateAttemptStarted",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("attempt_no", "u32"),
        EventFieldDescriptor::required("state_kind", "StateKind"),
        EventFieldDescriptor::required("state_version", "StateVersion"),
    ],
};

pub(super) const FACT_RECORDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.fact_recorded",
    rust_type_path: "mfm_events::v1::FactRecorded",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("claim", "FactClaim"),
    ],
};

pub(super) const ARTIFACT_REFERENCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.artifact_referenced",
    rust_type_path: "mfm_events::v1::ArtifactReferenced",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::optional("node_id", "NodeId"),
        EventFieldDescriptor::optional("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("artifact_ref", "ArtifactEvidenceRef"),
    ],
};

pub(super) const CELL_PRODUCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.cell_produced",
    rust_type_path: "mfm_events::v1::CellProduced",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("cell_id", "CellId"),
        EventFieldDescriptor::required("scope_id", "ScopeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("semantic_type_id", "SemanticTypeId"),
        EventFieldDescriptor::required("schema_id", "SchemaId"),
        EventFieldDescriptor::required("value_lineage", "ValueLineageRef"),
        EventFieldDescriptor::required("context", "CellContextSpec"),
        EventFieldDescriptor::required("artifact_id", "ArtifactId"),
        EventFieldDescriptor::required("content_digest", "ContentDigest"),
        EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
        EventFieldDescriptor::optional("producer_state_kind", "StateKind"),
        EventFieldDescriptor::optional("producer_state_version", "StateVersion"),
    ],
};

pub(super) const CELL_SKIPPED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.cell_skipped",
    rust_type_path: "mfm_events::v1::CellSkipped",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("cell_id", "CellId"),
        EventFieldDescriptor::required("scope_id", "ScopeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("semantic_type_id", "SemanticTypeId"),
        EventFieldDescriptor::required("schema_id", "SchemaId"),
        EventFieldDescriptor::required("value_lineage", "ValueLineageRef"),
        EventFieldDescriptor::required("context", "CellContextSpec"),
        EventFieldDescriptor::required("skip_reason", "SkipReason"),
    ],
};

pub(super) const SIDE_EFFECT_INTENT_PERSISTED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.intent_persisted",
        rust_type_path: "mfm_events::v1::side_effect::IntentPersisted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("scope_id", "ScopeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("intent_schema_id", "SchemaId"),
            EventFieldDescriptor::required("intent_hash", "ContentDigest"),
            EventFieldDescriptor::required("intent_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("intent_artifact_evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("idempotency_input_schema_id", "SchemaId"),
            EventFieldDescriptor::required("idempotency_input_hash", "ContentDigest"),
            EventFieldDescriptor::required("idempotency_key", "IdempotencyKeyRef"),
            EventFieldDescriptor::required("capability_kind", "CapabilityKind"),
            EventFieldDescriptor::required("capability_version", "CapabilityVersion"),
            EventFieldDescriptor::required("adapter_kind", "AdapterKind"),
            EventFieldDescriptor::required("adapter_version", "AdapterVersion"),
        ],
    };

pub(super) const SIDE_EFFECT_CLAIMED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.side_effect.claimed",
    rust_type_path: "mfm_events::v1::side_effect::Claimed",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
        EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
        EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
        EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
        EventFieldDescriptor::required("claim_owner", "RunnerInvocationId"),
        EventFieldDescriptor::required("invocation_epoch", "u32"),
        EventFieldDescriptor::required("claim_generation", "u32"),
        EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
    ],
};

pub(super) const SIDE_EFFECT_CLAIM_TAKEN_OVER_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.claim_taken_over",
        rust_type_path: "mfm_events::v1::side_effect::ClaimTakenOver",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("previous_claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("new_claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("previous_claim_generation", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
        ],
    };

pub(super) const RESOURCE_LANE_CLAIMED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.resource_lane.claimed",
    rust_type_path: "mfm_events::v1::ResourceLaneClaimed",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
        EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
        EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
        EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
        EventFieldDescriptor::required("invocation_epoch", "u32"),
        EventFieldDescriptor::required("resource_key", "ResourceKeyEvidence"),
        EventFieldDescriptor::required("requirement_digest", "ContentDigest"),
        EventFieldDescriptor::required("resolved_by_capability_impl", "RunnerFactoryId"),
        EventFieldDescriptor::required("claim_id", "ResourceLaneClaimId"),
        EventFieldDescriptor::required("claim_fencing_token", "u64"),
        EventFieldDescriptor::required("lane_transition_seq", "u64"),
    ],
};

pub(super) const RESOURCE_LANE_CLAIM_INTENT_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.resource_lane.claim_intent",
    rust_type_path: "mfm_events::v1::ResourceLaneClaimIntent",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
        EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
        EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
        EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
        EventFieldDescriptor::required("invocation_epoch", "u32"),
        EventFieldDescriptor::required("resource_key", "ResourceKeyEvidence"),
        EventFieldDescriptor::required("requirement_digest", "ContentDigest"),
        EventFieldDescriptor::required("resolved_by_capability_impl", "RunnerFactoryId"),
    ],
};

pub(super) const SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.invocation_prepared",
        rust_type_path: "mfm_events::v1::side_effect::InvocationPrepared",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
            EventFieldDescriptor::optional("resource_key", "ResourceKeyEvidence"),
            EventFieldDescriptor::optional("prepared_artifact_id", "ArtifactId"),
            EventFieldDescriptor::optional("prepared_hash", "ContentDigest"),
            EventFieldDescriptor::optional("prepared_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_INVOCATION_STARTED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.invocation_started",
        rust_type_path: "mfm_events::v1::side_effect::InvocationStarted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
        ],
    };

pub(super) const SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.not_submitted_proven",
        rust_type_path: "mfm_events::v1::side_effect::NotSubmittedProven",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("proof_schema_id", "SchemaId"),
            EventFieldDescriptor::required("proof_hash", "ContentDigest"),
            EventFieldDescriptor::required("proof_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("proof_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.submission_observed",
        rust_type_path: "mfm_events::v1::side_effect::SubmissionObserved",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("submission_schema_id", "SchemaId"),
            EventFieldDescriptor::required("submission_hash", "ContentDigest"),
            EventFieldDescriptor::required("submission_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("submission_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.submission_unknown",
        rust_type_path: "mfm_events::v1::side_effect::SubmissionUnknown",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("evidence_schema_id", "SchemaId"),
            EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("evidence_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("evidence_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.receipt_observed",
        rust_type_path: "mfm_events::v1::side_effect::ReceiptObserved",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("receipt_schema_id", "SchemaId"),
            EventFieldDescriptor::required("receipt_hash", "ContentDigest"),
            EventFieldDescriptor::required("receipt_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("receipt_artifact_evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
            EventFieldDescriptor::optional("resource_touched_set", "ResourceTouchedSetEvidence",),
        ],
    };

pub(super) const SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.confirmation_observed",
        rust_type_path: "mfm_events::v1::side_effect::ConfirmationObserved",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("confirmation_schema_id", "SchemaId"),
            EventFieldDescriptor::required("confirmation_hash", "ContentDigest"),
            EventFieldDescriptor::required("confirmation_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("confirmation_artifact_evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
            EventFieldDescriptor::optional("resource_touched_set", "ResourceTouchedSetEvidence",),
        ],
    };

pub(super) const SIDE_EFFECT_AMBIGUOUS_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.side_effect.ambiguous",
    rust_type_path: "mfm_events::v1::side_effect::Ambiguous",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
        EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
        EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
        EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
        EventFieldDescriptor::required("invocation_epoch", "u32"),
        EventFieldDescriptor::required("ambiguity_code", "AmbiguityCode"),
        EventFieldDescriptor::required("evidence_schema_id", "SchemaId"),
        EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
        EventFieldDescriptor::required("evidence_artifact_id", "ArtifactId"),
        EventFieldDescriptor::required("evidence_artifact_evidence_hash", "ContentDigest"),
    ],
};

pub(super) const SIDE_EFFECT_FAILED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.side_effect.failed",
    rust_type_path: "mfm_events::v1::side_effect::Failed",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
        EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
        EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
        EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
        EventFieldDescriptor::required("invocation_epoch", "u32"),
        EventFieldDescriptor::required("failure_phase", "FailurePhase"),
        EventFieldDescriptor::required("retryable", "bool"),
        EventFieldDescriptor::required("error", "MfmErrorInfo"),
    ],
};

pub(super) const RESOURCE_LANE_RELEASED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.resource_lane.released",
    rust_type_path: "mfm_events::v1::ResourceLaneReleased",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
        EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
        EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
        EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
        EventFieldDescriptor::required("invocation_epoch", "u32"),
        EventFieldDescriptor::required("claim_id", "ResourceLaneClaimId"),
        EventFieldDescriptor::required("release_id", "ResourceLaneReleaseId"),
        EventFieldDescriptor::required("claim_fencing_token", "u64"),
        EventFieldDescriptor::required("release_authority", "ResourceLaneReleaseAuthority",),
        EventFieldDescriptor::required("release_reason", "ResourceLaneReleaseReason"),
        EventFieldDescriptor::required("lane_transition_seq", "u64"),
    ],
};

pub(super) const RESOURCE_LANE_RELEASE_INTENT_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.resource_lane.release_intent",
        rust_type_path: "mfm_events::v1::ResourceLaneReleaseIntent",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_id", "ResourceLaneClaimId"),
            EventFieldDescriptor::required("release_authority", "ResourceLaneReleaseAuthority",),
            EventFieldDescriptor::required("release_reason", "ResourceLaneReleaseReason"),
        ],
    };

pub(super) const PUBLIC_OUTPUT_PRODUCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.public_output_produced",
    rust_type_path: "mfm_events::v1::PublicOutputProduced",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("receipt_cell_id", "CellId"),
        EventFieldDescriptor::required("public_schema_id", "SchemaId"),
        EventFieldDescriptor::required("output_spec_digest", "ContentDigest"),
        EventFieldDescriptor::repeated("cells", "NamedTypedCellRef"),
        EventFieldDescriptor::required("rendered_digest", "ContentDigest"),
        EventFieldDescriptor::optional("rendered_artifact_id", "ArtifactId"),
        EventFieldDescriptor::optional("rendered_artifact_evidence_hash", "ContentDigest"),
        EventFieldDescriptor::required("renderer_descriptor_id", "DescriptorId"),
    ],
};

pub(super) const PUBLIC_OUTPUT_RENDER_FAILED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.public_output_render_failed",
        rust_type_path: "mfm_events::v1::PublicOutputRenderFailed",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("public_schema_id", "SchemaId"),
            EventFieldDescriptor::required("renderer_descriptor_id", "DescriptorId"),
            EventFieldDescriptor::required("error", "MfmErrorInfo"),
        ],
    };

pub(super) const STATE_ATTEMPT_COMPLETED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.state_attempt_completed",
    rust_type_path: "mfm_events::v1::StateAttemptCompleted",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("output_cell_id", "CellId"),
    ],
};

pub(super) const STATE_ATTEMPT_INTERRUPTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.state_attempt_interrupted",
    rust_type_path: "mfm_events::v1::StateAttemptInterrupted",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
    ],
};

pub(super) const STATE_ATTEMPT_FAILED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.state_attempt_failed",
    rust_type_path: "mfm_events::v1::StateAttemptFailed",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
        EventFieldDescriptor::required("retryable", "bool"),
        EventFieldDescriptor::required("error", "MfmErrorInfo"),
    ],
};

pub(super) const MANUAL_RESOLUTION_RECORDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.manual_resolution_recorded",
    rust_type_path: "mfm_events::v1::ManualResolutionRecorded",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("run_id", "RunId"),
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("outcome", "ManualResolutionOutcome"),
        EventFieldDescriptor::required("evidence_schema_id", "SchemaId"),
        EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
        EventFieldDescriptor::required("evidence_artifact_id", "ArtifactId"),
        EventFieldDescriptor::required("evidence_artifact_evidence_hash", "ContentDigest"),
        EventFieldDescriptor::required("authorization_schema_id", "SchemaId"),
        EventFieldDescriptor::required("authorization_hash", "ContentDigest"),
        EventFieldDescriptor::required("authorization_artifact_id", "ArtifactId"),
        EventFieldDescriptor::required("authorization_artifact_evidence_hash", "ContentDigest",),
        EventFieldDescriptor::optional("note", "ManualResolutionNote"),
    ],
};

pub(super) const RUN_COMPLETED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.run_completed",
    rust_type_path: "mfm_events::v1::RunCompleted",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("run_id", "RunId"),
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("outcome", "RunCompletionOutcome"),
    ],
};

pub(super) const RETENTION_REFS_APPENDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.retention_refs_appended",
    rust_type_path: "mfm_events::v1::RetentionRefsAppended",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("run_id", "RunId"),
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::repeated("refs", "RetentionRef"),
        EventFieldDescriptor::required("reason", "RetentionReason"),
    ],
};

pub(super) const RETENTION_MANIFEST_PROJECTED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
        schema_name: "mfm.events.v1.retention_manifest_projected",
        rust_type_path: "mfm_events::v1::RetentionManifestProjected",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("run_id", "RunId"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("manifest_seq", "u64"),
            EventFieldDescriptor::required("manifest_digest", "ContentDigest"),
            EventFieldDescriptor::optional("previous_manifest_digest", "ContentDigest"),
            EventFieldDescriptor::required("manifest_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("manifest_artifact_evidence_hash", "ContentDigest"),
        ],
    };
