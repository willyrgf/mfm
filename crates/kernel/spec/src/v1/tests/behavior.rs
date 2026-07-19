use super::*;

#[test]
fn certified_spec_hash_golden() {
    let spec = test_spec();
    let canonical = spec.canonical_json().expect("canonical spec");
    assert_eq!(
        spec.public_outputs
            .digest()
            .expect("public output digest")
            .as_str(),
        "content:sha256-jcs-v1:e8d43ce660104518546731c9e7120d4df954e4f610e3c295996fceb377d27f4e"
    );
    assert_eq!(
        spec.spec_hash().expect("spec hash").as_str(),
        "spec:sha256-jcs-v1:2a2cc678c27df0580379244e4e825799888991bb602b0a03f5733109690b1319"
    );
    assert!(canonical
        .as_str()
        .contains(r#""spec_version":"mfm.typed.execution_spec.v1""#));
    assert!(canonical.as_str().contains(r#""remediations":{}"#));
    assert!(canonical
        .as_str()
        .contains(r#""saga":{"kind":"no_side_effects"}"#));
    let audit = TypedExecutionSpecAudit {
        source_package_refs: vec![SourcePackageRef {
            name: "mfm-spec-test".to_owned(),
            version: "0.1.0".to_owned(),
            artifact_id: Some(artifact(0x90)),
        }],
        ..TypedExecutionSpecAudit::default()
    };
    let envelope = HashedSpecEnvelope::new(spec.clone(), audit).expect("env");
    assert_eq!(envelope.spec_hash, spec.spec_hash().expect("spec hash"));
    envelope.verify_hash().expect("hash verifies");

    let mut stale = envelope;
    stale.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0x99));
    assert!(matches!(
        stale.verify_hash(),
        Err(SpecError::HashMismatch { .. })
    ));
}

#[test]
fn fact_descriptor_allowlists_are_hash_defining() {
    let base = test_spec();
    let base_hash = base.spec_hash().expect("base hash");
    let fact_ref = FactDescriptorRef {
        descriptor_hash: content(0xa1),
    };

    let mut descriptor_changed = base.clone();
    let DescriptorIdentity::State(state) = &mut descriptor_changed.descriptor_identities[0] else {
        panic!("first descriptor is state");
    };
    state.emitted_fact_descriptors.push(fact_ref.clone());
    assert_ne!(
        descriptor_changed.spec_hash().expect("descriptor hash"),
        base_hash
    );

    let mut node_changed = base;
    node_changed.nodes[0]
        .fact_descriptor_allowlist
        .push(fact_ref);
    assert_ne!(node_changed.spec_hash().expect("node hash"), base_hash);
    assert!(node_changed
        .canonical_json()
        .expect("canonical spec")
        .as_str()
        .contains(r#""fact_descriptor_allowlist":[{"descriptor_hash":"content:"#));
}

#[test]
fn certified_contexts_and_constraints_are_hash_defining() {
    let base = test_spec();
    let base_hash = base.spec_hash().expect("base hash");
    let context = test_context();
    let context_ref = context.context_ref.clone();
    let producer_descriptor_id = base.nodes[0].descriptor_id.clone();

    let mut contexted = base.clone();
    contexted.contexts.push(context);
    contexted.nodes[0].context = NodeContextSpec::Required {
        context_ref: context_ref.clone(),
    };
    contexted.cells[1].context = CellContextSpec::Bound {
        context_ref: context_ref.clone(),
        resource_kind: ContextResourceKind::new("contract_instance").expect("resource kind"),
        stage: ContextStage::new("deployed").expect("stage"),
        producer: Box::new(context_producer(producer_descriptor_id.clone())),
    };
    let InputBindingNodeSpec::Cell(input_cell) = &mut contexted.nodes[1].input_bindings.root else {
        panic!("bridge input is a cell");
    };
    input_cell.context = InputContextSpec::Required {
        context_ref,
        resource_kind: ContextResourceKind::new("contract_instance").expect("resource kind"),
        stage: ContextStage::new("deployed").expect("stage"),
        producer: Box::new(context_producer(producer_descriptor_id)),
    };

    let contexted_hash = contexted.spec_hash().expect("contexted hash");
    assert_ne!(
        contexted_hash, base_hash,
        "certified context table and constraints must be hash-defining"
    );

    let canonical = contexted.canonical_json().expect("contexted canonical");
    assert!(canonical.as_str().contains(r#""contexts":[{"#));
    assert!(canonical.as_str().contains(r#""kind":"required""#));
    assert!(canonical
        .as_str()
        .contains(r#""context":{"context_ref":"context:"#));
    let parsed =
        TypedExecutionSpec::from_json_slice(canonical.as_bytes()).expect("parse contexted spec");
    assert_eq!(parsed, contexted);
}

#[test]
fn certified_context_table_rejects_duplicate_unknown_or_mismatched_refs() {
    let context = test_context();

    let mut duplicate = test_spec();
    duplicate.contexts.push(context.clone());
    duplicate.contexts.push(context.clone());
    let err = duplicate
        .spec_hash()
        .expect_err("duplicate context refs reject");
    assert!(
        err.to_string().contains("duplicate certified context"),
        "{err}"
    );

    let mut unknown_node = test_spec();
    unknown_node.nodes[0].context = NodeContextSpec::Required {
        context_ref: context.context_ref.clone(),
    };
    let err = unknown_node
        .spec_hash()
        .expect_err("unknown node context ref rejects");
    assert!(
        err.to_string().contains("unknown certified context ref"),
        "{err}"
    );

    let mut unknown_cell = test_spec();
    unknown_cell.cells[0].context = CellContextSpec::Bound {
        context_ref: context.context_ref.clone(),
        resource_kind: ContextResourceKind::new("contract_instance").expect("resource kind"),
        stage: ContextStage::new("deployed").expect("stage"),
        producer: Box::new(context_producer(descriptor(0xa5))),
    };
    let err = unknown_cell
        .spec_hash()
        .expect_err("unknown cell context ref rejects");
    assert!(
        err.to_string().contains("unknown certified context ref"),
        "{err}"
    );

    let mut bad_digest = test_spec();
    bad_digest.contexts.push(CertifiedContextSpec {
        canonical_context_digest: content(0xaf),
        ..context.clone()
    });
    let err = bad_digest
        .spec_hash()
        .expect_err("context digest mismatch rejects");
    assert!(err.to_string().contains("digest mismatch"), "{err}");

    let mut bad_ref = test_spec();
    bad_ref.contexts.push(CertifiedContextSpec {
        context_ref: ContextRef::from_digest(DigestAlgorithm::Sha256JcsV1, digest(0xae)),
        ..context
    });
    let err = bad_ref
        .spec_hash()
        .expect_err("context ref mismatch rejects");
    assert!(err.to_string().contains("context ref mismatch"), "{err}");
}

#[test]
fn persisted_specs_without_context_table_or_with_float_contexts_reject() {
    let spec = test_spec();
    let canonical = spec.canonical_json().expect("canonical spec");
    let mut value: serde_json::Value = serde_json::from_str(canonical.as_str()).expect("spec JSON");
    value
        .as_object_mut()
        .expect("spec object")
        .remove("contexts");
    let input = serde_json::to_string(&value).expect("JSON");
    let err = TypedExecutionSpec::from_json_str(&input).expect_err("missing contexts rejects");
    assert!(err.to_string().contains("contexts"), "{err}");

    let mut floated = test_spec();
    let mut context = test_context();
    context.canonical_context = PlainCanonicalJsonBytes::from_json_str(r#"{"network_id":"local"}"#)
        .expect("canonical context");
    context.canonical_context_digest = context.canonical_context.content_digest();
    context.canonical_context_byte_len = context.canonical_context.as_bytes().len() as u64;
    context.context_ref = CertifiedContextSpec::derive_context_ref(
        &context.context_descriptor_id,
        &context.schema_id,
        &context.semantic_type_id,
        &context.canonicalizer_identity,
        &context.canonical_context,
    )
    .expect("context ref");
    floated.contexts.push(context);
    let canonical = floated.canonical_json().expect("canonical floated base");
    let mut value: serde_json::Value = serde_json::from_str(canonical.as_str()).expect("spec JSON");
    value["contexts"][0]["canonical_context"] =
        serde_json::json!({"expected_chain_id": 31337.5, "network_id": "local"});
    let input = serde_json::to_string(&value).expect("JSON");
    let err = TypedExecutionSpec::from_json_str(&input).expect_err("float context rejects");
    assert!(
        err.to_string().contains("floats are not allowed")
            || err.to_string().contains("number forms are not allowed"),
        "{err}"
    );
}

#[test]
fn saga_policy_and_remediations_are_hash_defining() {
    let base = test_spec();
    let base_hash = base.spec_hash().expect("base hash");
    let manual = ManualResolutionEvidenceSpec {
        evidence_schema: schema("mfm.spec.test.manual_evidence", 0x91),
        authorization: manual_authorization(0x92),
    };

    let mut manual_spec = base.clone();
    manual_spec.saga = SagaPolicySpec::ManualResolution {
        manual: manual.clone(),
    };
    assert_ne!(
        manual_spec.spec_hash().expect("manual hash"),
        base_hash,
        "run-level saga policy must be hash-defining"
    );

    let mut compensated = base.clone();
    let forward_node = compensated.nodes[0].node_id.clone();
    compensated.saga = SagaPolicySpec::CompensateCompleted {
        on_remediation_unresolved: RemediationUnresolvedSpec::ManualResolution {
            manual: Box::new(manual),
        },
    };
    compensated
        .remediations
        .insert(forward_node, compensated.nodes[0].clone());
    let canonical = compensated.canonical_json().expect("compensated canonical");
    let parsed = TypedExecutionSpec::from_json_slice(canonical.as_bytes())
        .expect("parse compensated saga spec");

    assert_eq!(parsed, compensated);
    assert_ne!(
        compensated.spec_hash().expect("compensated hash"),
        base_hash,
        "remediation node collection must be hash-defining"
    );
}

#[test]
fn resource_claims_are_mandatory_and_hash_defining() {
    let mut manual = test_spec();
    manual.nodes[0].side_effect = Some(SideEffectContractSpec {
        contract_digest: content(0x91),
        resource_claim: ResourceClaimSpec::ManualOnly,
        verification: SideEffectVerificationSpec::Receipt,
    });
    let manual_hash = manual.spec_hash().expect("manual claim hash");

    let mut exclusive = manual.clone();
    exclusive.nodes[0]
        .side_effect
        .as_mut()
        .expect("side-effect")
        .resource_claim = ResourceClaimSpec::Exclusive {
        namespace: ResourceNamespace::new("mfm.spec.test.account_nonce").expect("namespace"),
        key_schema: schema("mfm.spec.test.resource_key", 0x92),
    };
    assert_ne!(
        exclusive.spec_hash().expect("exclusive claim hash"),
        manual_hash,
        "side-effect resource claim must be hash-defining"
    );

    let canonical = exclusive.canonical_json().expect("exclusive canonical");
    let parsed = TypedExecutionSpec::from_json_slice(canonical.as_bytes())
        .expect("parse exclusive resource claim");
    assert_eq!(parsed, exclusive);

    let mut missing: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    missing["nodes"][0]["side_effect"]
        .as_object_mut()
        .expect("side-effect object")
        .remove("resource_claim");
    let missing = serde_json::to_string(&missing).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&missing).expect_err("missing resource claim rejects");
    assert!(matches!(err, SpecError::Json(message) if message.contains("resource_claim")));

    let mut missing: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    missing["nodes"][0]["side_effect"]
        .as_object_mut()
        .expect("side-effect object")
        .remove("verification");
    let missing = serde_json::to_string(&missing).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&missing).expect_err("missing verification rejects");
    assert!(matches!(err, SpecError::Json(message) if message.contains("verification")));

    let mut finalized_1 = exclusive.clone();
    finalized_1.nodes[0]
        .side_effect
        .as_mut()
        .expect("side-effect")
        .verification = SideEffectVerificationSpec::Finalized { depth: 1 };
    let mut finalized_12 = exclusive.clone();
    finalized_12.nodes[0]
        .side_effect
        .as_mut()
        .expect("side-effect")
        .verification = SideEffectVerificationSpec::Finalized { depth: 12 };
    assert_ne!(
        finalized_1.spec_hash().expect("finalized one hash"),
        exclusive.spec_hash().expect("receipt hash"),
        "side-effect verification policy must be hash-defining"
    );
    assert_ne!(
        finalized_12.spec_hash().expect("finalized twelve hash"),
        finalized_1.spec_hash().expect("finalized one hash"),
        "side-effect verification depth must be hash-defining"
    );

    let mut unknown: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    unknown["nodes"][0]["side_effect"]["resource_claim"]["kind"] = serde_json::json!("optimistic");
    let unknown = serde_json::to_string(&unknown).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&unknown).expect_err("unknown resource claim rejects");
    assert!(matches!(
        err,
        SpecError::Json(message) if message.contains("unsupported resource claim kind")
    ));

    let mut unknown: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    unknown["nodes"][0]["side_effect"]["verification"]["kind"] =
        serde_json::json!("mutable_registry");
    let unknown = serde_json::to_string(&unknown).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&unknown).expect_err("unknown verification rejects");
    assert!(matches!(
        err,
        SpecError::Json(message) if message.contains("unsupported side-effect verification kind")
    ));

    let mut zero_depth: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("json value");
    zero_depth["nodes"][0]["side_effect"]["verification"] = serde_json::json!({
        "finalized": {
            "depth": 0,
        },
        "kind": "finalized",
    });
    let zero_depth = serde_json::to_string(&zero_depth).expect("json");
    let err =
        TypedExecutionSpec::from_json_str(&zero_depth).expect_err("zero finalized depth rejects");
    assert!(matches!(
        err,
        SpecError::Json(message) if message.contains("depth must be positive")
    ));
}

#[test]
fn persisted_spec_json_round_trips_and_rejects_unknown_fields() {
    let spec = test_spec();
    let canonical = spec.canonical_json().expect("canonical spec");
    let parsed =
        TypedExecutionSpec::from_json_slice(canonical.as_bytes()).expect("parse canonical spec");

    assert_eq!(parsed, spec);
    assert_eq!(
        parsed.canonical_json().expect("canonical parsed"),
        canonical
    );
    assert_eq!(
        parsed.spec_hash().expect("parsed hash"),
        spec.spec_hash().expect("spec hash")
    );

    let mut unknown_field: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("spec JSON");
    unknown_field
        .as_object_mut()
        .expect("spec object")
        .insert("unknown_field".to_owned(), serde_json::json!(true));
    let input = serde_json::to_string(&unknown_field).expect("JSON");
    let err = TypedExecutionSpec::from_json_str(&input).expect_err("unknown field rejects");

    assert!(matches!(err, SpecError::Json(message) if message.contains("unknown")));
}

#[test]
fn persisted_node_descriptor_refs_are_checked_against_descriptor_table() {
    let spec = test_spec();
    let canonical = spec.canonical_json().expect("canonical spec");
    let value: serde_json::Value = serde_json::from_str(canonical.as_str()).expect("spec JSON");
    let first_node = &value["nodes"][0];

    assert!(first_node.get("descriptor_ref").is_some());
    assert!(first_node.get("descriptor_id").is_none());
    assert!(first_node.get("state_kind").is_none());
    assert!(first_node.get("state_version").is_none());
    assert!(first_node.get("effect_kind").is_none());
    assert!(first_node.get("capability_bindings").is_none());

    let mut wrong_digest = value.clone();
    wrong_digest["nodes"][0]["descriptor_ref"]["descriptor_digest"] =
        serde_json::json!(content(0xfe).as_str());
    let input = serde_json::to_string(&wrong_digest).expect("JSON");
    let err =
        TypedExecutionSpec::from_json_str(&input).expect_err("descriptor digest mismatch rejects");
    assert!(err.to_string().contains("descriptor ref mismatch"), "{err}");

    let mut wrong_family = value;
    wrong_family["nodes"][0]["descriptor_ref"]["descriptor_family"] =
        serde_json::json!("operation");
    let input = serde_json::to_string(&wrong_family).expect("JSON");
    let err =
        TypedExecutionSpec::from_json_str(&input).expect_err("descriptor family mismatch rejects");
    assert!(err.to_string().contains("expected state"), "{err}");
}

#[test]
fn lifecycle_framework_node_json_round_trips_through_checked_parser() {
    let mut variants = Vec::new();
    variants.push(FrameworkNodeSpec::ProjectRetentionManifest(
        ProjectRetentionManifestNodeSpec {
            public_schema_id: schema("mfm.spec.test.lifecycle_public", 0x70),
            public_output_receipt_cell: cell(0x71),
        },
    ));
    variants.push(FrameworkNodeSpec::CompleteRun(CompleteRunNodeSpec {
        public_schema_id: schema("mfm.spec.test.lifecycle_complete", 0x72),
        retention_manifest_receipt_cell: cell(0x73),
    }));
    variants.push(FrameworkNodeSpec::ResolveSagaTerminal(
        ResolveSagaTerminalNodeSpec {
            public_schema_id: schema("mfm.spec.test.lifecycle_resolve", 0x74),
        },
    ));

    for framework in variants {
        let mut spec = test_spec();
        spec.nodes[0].framework = Some(framework);
        let canonical = spec.canonical_json().expect("canonical spec");
        let parsed = TypedExecutionSpec::from_json_slice(canonical.as_bytes())
            .expect("parse lifecycle framework node");

        assert_eq!(parsed, spec);
    }
}
