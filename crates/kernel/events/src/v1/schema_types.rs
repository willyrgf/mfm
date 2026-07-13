use super::*;

pub(super) fn schema_field(
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
