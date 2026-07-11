use super::lifecycle_support::{
    append_independent_user_node, append_retention_lifecycle_node, append_user_receipt_consumer,
    clear_lifecycle_framework_metadata, find_lifecycle_node_mut, lifecycle_render_receipt_cell,
    lifecycle_retention_receipt_cell, predecessors_for_test_inputs,
};
use super::*;

macro_rules! assert_rejection_cases {
    ($fixture:ident, $($expected:expr => $mutate:expr),+ $(,)?) => {{
        let (registry, base) = $fixture();
        $(assert_rejects(&registry, &base, $expected, $mutate);)+
    }};
}

#[path = "behavior/core.rs"]
mod core;

#[path = "behavior/side_effect.rs"]
mod side_effect;

#[path = "behavior/saga.rs"]
mod saga;

fn assert_rejects(
    registry: &CertificationRegistry,
    base: &spec::TypedExecutionSpec,
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let mut mutated = base.clone();
    mutate(&mut mutated);
    let error = certify_untrusted_typed_spec(mutated, registry).expect_err("mutation must reject");
    assert_eq!(error.problem_class(), Some(expected), "{error}");
}

fn assert_problem_contains(error: CertifyError, expected: ProblemClass, needle: &str) {
    assert_eq!(error.problem_class(), Some(expected), "{error}");
    assert!(error.to_string().contains(needle), "{error}");
}

fn assert_invalid_semantic_contains(error: CertifyError, needle: &str) {
    assert_problem_contains(error, ProblemClass::InvalidSemanticTransition, needle);
}

fn assert_manual_certification_rejects(
    typed: spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
    needle: &str,
) {
    let error = certify_untrusted_typed_spec(typed, registry).expect_err(needle);
    assert_invalid_semantic_contains(error, needle);
}

fn assert_reference_rejects(
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let (registry, base) = reference_registry_and_spec();
    assert_rejects(&registry, &base, expected, mutate);
}

fn assert_side_effect_rejects(
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let (registry, base) = side_effect_registry_and_spec();
    assert_rejects(&registry, &base, expected, mutate);
}

fn assert_compensating_rejects(
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let (registry, base) = compensating_registry_and_spec();
    assert_rejects(&registry, &base, expected, mutate);
}

fn reference_registry_and_spec() -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = reference_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn context_bound_registry_and_spec(
    declare_second_context: bool,
) -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = context_bound_draft(declare_second_context);
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn retarget_context_ref(typed: &mut spec::TypedExecutionSpec, old: &ContextRef, new: &ContextRef) {
    for node in &mut typed.nodes {
        if let spec::NodeContextSpec::Required { context_ref } = &mut node.context {
            if context_ref == old {
                *context_ref = new.clone();
            }
        }
        retarget_input_binding_context_ref(&mut node.input_bindings.root, old, new);
        node.input_bindings.digest =
            content_digest_json(input_node_json(&node.input_bindings.root)).expect("input digest");
    }
    for cell in &mut typed.cells {
        if let spec::CellContextSpec::Bound { context_ref, .. } = &mut cell.context {
            if context_ref == old {
                *context_ref = new.clone();
            }
        }
    }
}

fn retarget_input_binding_context_ref(
    node: &mut spec::InputBindingNodeSpec,
    old: &ContextRef,
    new: &ContextRef,
) {
    match node {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => {
            if let spec::InputContextSpec::Required { context_ref, .. } = &mut cell.context {
                if context_ref == old {
                    *context_ref = new.clone();
                }
            }
        }
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                retarget_input_binding_context_ref(element, old, new);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                retarget_input_binding_context_ref(&mut field.node, old, new);
            }
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                retarget_input_binding_context_ref(element, old, new);
            }
        }
    }
}

fn descriptor_id_by_name(typed: &spec::TypedExecutionSpec, name: &str) -> DescriptorId {
    typed
        .descriptor_identities
        .iter()
        .find_map(|descriptor| match descriptor {
            spec::DescriptorIdentity::State(state) if state.name == name => {
                Some(state.descriptor_id.clone())
            }
            _ => None,
        })
        .expect("state descriptor by name")
}

fn node_with_descriptor_name<'a>(
    typed: &'a spec::TypedExecutionSpec,
    name: &str,
) -> &'a spec::NodeSpec {
    let descriptor_id = descriptor_id_by_name(typed, name);
    typed
        .nodes
        .iter()
        .find(|node| node.descriptor_id == descriptor_id)
        .expect("node by descriptor name")
}

fn node_with_descriptor_name_mut<'a>(
    typed: &'a mut spec::TypedExecutionSpec,
    name: &str,
) -> &'a mut spec::NodeSpec {
    let descriptor_id = descriptor_id_by_name(typed, name);
    typed
        .nodes
        .iter_mut()
        .find(|node| node.descriptor_id == descriptor_id)
        .expect("node by descriptor name")
}

fn compensating_registry_and_spec() -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = compensating_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn side_effect_registry_and_spec() -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = side_effect_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn side_effect_submit_node(typed: &spec::TypedExecutionSpec) -> &spec::NodeSpec {
    typed
        .nodes
        .iter()
        .find(|node| node.side_effect.is_some() && node.framework.is_none())
        .expect("side-effect submit node")
}

fn side_effect_verify_node(
    typed: &spec::TypedExecutionSpec,
) -> (&spec::NodeSpec, &spec::SideEffectVerifyNodeSpec) {
    typed
        .nodes
        .iter()
        .find_map(|node| match &node.framework {
            Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => Some((node, verify)),
            _ => None,
        })
        .expect("side-effect verify node")
}

fn side_effect_spec_with_manual(
    manual: spec::ManualResolutionEvidenceSpec,
) -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = side_effect_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let mut typed = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    typed.saga = spec::SagaPolicySpec::ManualResolution { manual };
    (registry, typed)
}

fn manual_resolution_spec(byte: u8) -> spec::ManualResolutionEvidenceSpec {
    let evidence_schema = SchemaId::new(
        "mfm.certify.test.manual_evidence",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_byte(byte),
    )
    .expect("manual evidence schema");
    spec::ManualResolutionEvidenceSpec {
        evidence_schema,
        authorization: spec::ManualResolutionAuthorizationSpec {
            verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
                "mfm.certify.test.manual.verifier.{byte:02x}"
            ))
            .expect("verifier id"),
            signing_scheme: spec::ManualSigningSchemeSpec::new(MANUAL_RESOLUTION_SIGNING_SCHEME)
                .expect("signing scheme"),
            authority: spec::OperatorAuthoritySnapshotSpec {
                authority_id: spec::OperatorAuthorityId::new(format!(
                    "mfm.certify.test.manual.authority.{byte:02x}"
                ))
                .expect("authority id"),
                operators: vec![spec::OperatorAuthorityMemberSpec {
                    operator_id: spec::OperatorId::new(format!("operator.certify.{byte:02x}"))
                        .expect("operator id"),
                    public_identity: spec::OperatorPublicIdentity::new(format!(
                        "operator-certify-public-{byte:02x}"
                    ))
                    .expect("operator public identity"),
                }],
            },
            quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
        },
    }
}

fn register_manual_authority(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
) {
    register_manual_evidence_schema(registry, manual);
    register_manual_verifier(registry, manual);
    register_manual_authority_snapshot(registry, manual.authorization.authority.clone());
}

fn register_manual_evidence_schema(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
) {
    register_manual_schema_role(
        registry,
        manual,
        CertifiedSchemaRole::ManualResolutionEvidence,
    );
}

fn register_manual_schema_role(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
    role: CertifiedSchemaRole,
) {
    registry
        .register_schema_role(manual.evidence_schema.clone(), role)
        .expect("register schema role");
}

fn register_manual_verifier(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
) {
    registry
        .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
        .expect("register verifier");
}

fn register_manual_authority_snapshot(
    registry: &mut CertificationRegistry,
    authority: spec::OperatorAuthoritySnapshotSpec,
) {
    registry
        .register_operator_authority_snapshot(authority)
        .expect("register authority");
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(byte))
}

fn retarget_first_remediation_input(typed: &mut spec::TypedExecutionSpec, cell_id: CellId) {
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == cell_id)
        .expect("input cell")
        .clone();
    let root = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
        field_path: spec::PublicFieldPath::new("input").expect("field path"),
        cell_id: cell_id.clone(),
        semantic_type_id: input_cell.semantic_type_id,
        schema_id: input_cell.schema_id,
        required_terminal: spec::RequiredTerminal::ProducedOnly,
        value_lineage: input_cell.value_lineage,
        context: spec::InputContextSpec::no_context(),
    }));
    let digest = content_digest_json(input_node_json(&root)).expect("input digest");
    let predecessors = predecessors_for_test_inputs(typed, std::slice::from_ref(&cell_id));
    let original_output_cell = typed
        .remediations
        .values()
        .next()
        .expect("remediation")
        .output_cell
        .clone();
    let original_output = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == original_output_cell)
        .expect("remediation output cell")
        .clone();
    let (
        previous_node_id,
        previous_output_cell,
        node_id,
        output_cell,
        scope_id,
        planning_lineage,
        config_digest,
    ) = {
        let remediation = typed.remediations.values_mut().next().expect("remediation");
        let previous_node_id = remediation.node_id.clone();
        remediation.input_bindings.root = root;
        remediation.input_bindings.digest = digest;
        remediation.deterministic_predecessors = predecessors;
        let config_digest = config_ref_digest(&remediation.config_ref).expect("config digest");
        remediation.node_id =
            state_node_id_from_spec(remediation, &config_digest).expect("remediation node id");
        remediation.output_cell = cell_id_from_parts(
            &remediation.scope_id,
            &spec::CellProducer::Node(remediation.node_id.clone()),
            &original_output.semantic_type_id,
            &original_output.schema_id,
        )
        .expect("remediation output cell");
        (
            previous_node_id,
            original_output_cell,
            remediation.node_id.clone(),
            remediation.output_cell.clone(),
            remediation.scope_id.clone(),
            remediation.planning_lineage.clone(),
            config_digest,
        )
    };
    let lineage = render_value_lineage_ref(
        &scope_id,
        &node_id,
        std::slice::from_ref(&cell_id),
        &planning_lineage,
        &config_digest,
    )
    .expect("lineage");
    typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == previous_output_cell)
        .expect("remediation output cell")
        .cell_id = output_cell.clone();
    let output = typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == output_cell)
        .expect("retargeted remediation output cell");
    output.producer = spec::CellProducer::Node(node_id.clone());
    output.value_lineage = lineage.clone();
    let value_lineage = typed
        .value_lineages
        .iter_mut()
        .find(|lineage| lineage.producer == spec::CellProducer::Node(previous_node_id.clone()))
        .expect("remediation value lineage");
    value_lineage.producer = spec::CellProducer::Node(node_id);
    value_lineage.lineage_ref = lineage;
    value_lineage.input_cells = vec![cell_id];
    value_lineage.config_ref_digest = Some(config_digest);
    value_lineage.planning_lineage = planning_lineage;
}

fn reference_persisted_spec_certificate_parts() -> (
    CertificationRegistry,
    CertifiedTypedSpec,
    PersistedSpecCertificateParts,
) {
    let draft = reference_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let certified = certify_program_draft(&draft).expect("certified");
    assert_eq!(
        certified.certificate().evidence.registry_digest,
        registry.digest().expect("registry digest")
    );
    let persisted_parts = certified
        .to_persisted_parts()
        .expect("persisted spec/certificate parts");
    (registry, certified, persisted_parts)
}

fn certificate_bytes(certificate: &CertifiedSpecCertificate) -> Vec<u8> {
    certificate
        .canonical_json()
        .expect("certificate canonical json")
        .to_vec()
}
