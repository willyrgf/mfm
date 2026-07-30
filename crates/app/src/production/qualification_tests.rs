use serde_json::Value;

use super::*;

#[test]
fn frozen_foundation_objects_keep_exact_bytes_and_identities() {
    let (evidence, evidence_ref) = object_evidence_contract().expect("evidence contract");
    assert_eq!(
        evidence.canonical(),
        &mfm_values::component_object_evidence_contract_canonical()
            .expect("shared evidence canonical")
    );
    assert_eq!(
        evidence_ref,
        mfm_values::component_object_evidence_contract_ref().expect("shared evidence ref")
    );
    assert_eq!(
        evidence.value_contract().semantic_type_id().as_str(),
        "semantic:mfm.product:component-object-evidence-contract:1:sha256-jcs-v1:4b996fbc61b28b6cb505cad6fe32a7f87f39b20ba3275306c31df064d36b57b6"
    );
    assert_eq!(
        evidence.value_contract().evidence_contract_ref(),
        &evidence_ref
    );

    let (profile, profile_ref) =
        qualification_profile(evidence_ref.clone()).expect("qualification profile");
    assert_eq!(
        profile.canonical().as_str(),
        r#"{"version":"mfm.component-qualification-profile.v1"}"#
    );
    assert_eq!(
        profile_ref.schema_id(),
        &annex_schema_id(QUALIFICATION_PROFILE_CONTRACT).expect("profile schema")
    );
    assert_eq!(
        profile_ref.content_digest().as_str(),
        "content:sha256-v1:e384b0ed353d7e9e115d7238feae1b6517bbd5d2cfbbe88ddb655633afa03d30"
    );
    assert_eq!(
        profile.value_contract().semantic_type_id().as_str(),
        "semantic:mfm.product:component-qualification-profile:1:sha256-jcs-v1:c15789545f01147d9c45488411d4a14a5b3cafb5150fe75f82a37e7a1dda1abd"
    );
    assert_eq!(
        profile.value_contract().evidence_contract_ref(),
        &evidence_ref
    );

    let planner_surface = mfm_program::CompositePlannerSurface::current().expect("planner surface");
    let (planner_contract, planner_callbacks) =
        planner_surface_support_members(&planner_surface, evidence_ref.clone())
            .expect("planner support members");
    assert_eq!(
        planner_contract.field_path().as_str(),
        PLANNER_SEMANTIC_PATH
    );
    assert_eq!(
        planner_contract.value_contract().role().as_str(),
        PLANNER_SEMANTIC_SUPPORT_ROLE
    );
    assert_eq!(
        planner_contract.canonical(),
        planner_surface.semantic_contract_canonical()
    );
    assert_eq!(
        support_content_ref(&planner_contract).expect("semantic support ref"),
        *planner_surface.semantic_contract_ref()
    );
    assert_eq!(
        planner_contract.value_contract().evidence_contract_ref(),
        &evidence_ref
    );

    assert_eq!(
        planner_callbacks.field_path().as_str(),
        PLANNER_CALLBACK_PATH
    );
    assert_eq!(
        planner_callbacks.value_contract().role().as_str(),
        PLANNER_CALLBACK_SUPPORT_ROLE
    );
    assert_eq!(
        planner_callbacks.canonical(),
        planner_surface.callback_surface_canonical()
    );
    assert_eq!(
        support_content_ref(&planner_callbacks).expect("callback support ref"),
        *planner_surface.callback_surface_ref()
    );
    assert_eq!(
        planner_callbacks.value_contract().evidence_contract_ref(),
        &evidence_ref
    );
}

#[test]
fn product_qualification_is_one_acyclic_fourteen_component_bijection() {
    let (executable, executable_bytes) = test_executable();
    let (executor_contract, _, _) = test_executor_material();
    let product = qualify_product_components(
        executable.clone(),
        executable_bytes.as_bytes(),
        &executor_contract,
    )
    .expect("product qualification");

    assert_eq!(
        product.implementations.ordered().len(),
        PRODUCT_COMPONENT_COUNT
    );
    assert!(product
        .implementations
        .ordered()
        .iter()
        .all(|descriptor| descriptor.qualification_ref() == &product.qualification_ref));
    assert_eq!(product.support_members.len(), PRODUCT_SUPPORT_MEMBER_COUNT);

    let qualification_member = product
        .support_members
        .iter()
        .find(|member| member.field_path().as_str() == QUALIFICATION_PATH)
        .expect("qualification member");
    let decoded: Value = serde_json::from_slice(qualification_member.canonical().as_bytes())
        .expect("qualification JSON");
    assert_eq!(decoded["version"], QUALIFICATION_VERSION);
    assert_eq!(
        decoded["executable_identity_ref"],
        serde_json::to_value(executable).expect("executable ref JSON")
    );
    let components = decoded["components"]
        .as_array()
        .expect("component inventory");
    assert_eq!(components.len(), PRODUCT_COMPONENT_COUNT);
    assert_eq!(
        components
            .iter()
            .filter(|component| component["component_kind"] == "planner")
            .count(),
        1
    );
    assert_eq!(
        components
            .iter()
            .filter(|component| { component["component_kind"] == "executor_client_verifier" })
            .count(),
        1
    );
    assert_eq!(
        components
            .iter()
            .filter(|component| component["component_kind"] == "state")
            .count(),
        STATE_COMPONENT_COUNT
    );
    assert_eq!(
        components
            .iter()
            .filter(|component| {
                component["component_kind"] == "read_capability_adapter_verifier"
            })
            .count(),
        1
    );
    assert!(!qualification_member
        .canonical()
        .as_str()
        .contains("implementation"));
}

#[test]
fn component_support_closure_paths_and_roles_are_frozen() {
    let (executable, executable_bytes) = test_executable();
    let (executor_contract, _, _) = test_executor_material();
    let product =
        qualify_product_components(executable, executable_bytes.as_bytes(), &executor_contract)
            .expect("product qualification");
    let paths = component_support_paths();
    let expected_component_paths = paths
        .iter()
        .flat_map(|paths| {
            [
                paths.semantic_path,
                paths.callback_path,
                paths.implementation_path,
            ]
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        expected_component_paths.len(),
        COMPONENT_SUPPORT_MEMBER_COUNT
    );
    let actual_component_paths = product
        .support_members
        .iter()
        .map(|member| member.field_path().as_str())
        .filter(|path| expected_component_paths.contains(path))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_component_paths, expected_component_paths);

    for paths in paths {
        let semantic_member = support_member_at(&product.support_members, paths.semantic_path)
            .expect("semantic support member");
        let callback_member = support_member_at(&product.support_members, paths.callback_path)
            .expect("callback support member");
        let implementation_member =
            support_member_at(&product.support_members, paths.implementation_path)
                .expect("implementation support member");
        assert_eq!(
            semantic_member.value_contract().role().as_str(),
            paths.semantic_role
        );
        assert_eq!(
            callback_member.value_contract().role().as_str(),
            paths.callback_role
        );
        assert_eq!(
            implementation_member.value_contract().role().as_str(),
            paths.implementation_role
        );
        assert_eq!(
            implementation_member
                .value_contract()
                .semantic_type_id()
                .as_str(),
            "semantic:mfm.product:component-implementation-descriptor:1:sha256-jcs-v1:2fe4bed438b858b7f0b52da9263a0c69ddf580092d5a116155e853b0e24252dc"
        );
    }
}

#[test]
fn evm_live_support_keeps_exact_catalog_order_paths_roles_and_sources() {
    let transport = test_evm_transport();
    let evidence_ref = mfm_values::component_object_evidence_contract_ref().expect("evidence ref");
    let (members, catalog_ref, reviewed_source_ref) =
        evm_routing_support_members(&transport, evidence_ref.clone()).expect("routing support");

    assert_eq!(members.len(), 4);
    assert_eq!(
        support_content_ref(
            support_member_at(&members, EVM_ROUTING_CATALOG_PATH).expect("catalog member")
        )
        .expect("catalog ref"),
        catalog_ref
    );
    assert_eq!(
        support_member_at(&members, EVM_ROUTING_CATALOG_PATH)
            .expect("catalog member")
            .value_contract()
            .role()
            .as_str(),
        EVM_ROUTING_CATALOG_ROLE
    );

    for (index, (_, descriptor)) in transport.routing_generation_descriptors().enumerate() {
        let member = support_member_at(
            &members,
            &evm_routing_generation_path(index).expect("generation path"),
        )
        .expect("generation member");
        assert_eq!(
            support_content_ref(member).expect("generation ref"),
            descriptor.content_ref().expect("descriptor ref")
        );
        assert_eq!(
            member.value_contract().role().as_str(),
            EVM_ROUTING_GENERATION_ROLE
        );
        assert_eq!(
            member.value_contract().evidence_contract_ref(),
            &evidence_ref
        );
    }

    let reviewed =
        support_member_at(&members, EVM_REVIEWED_SOURCE_SCOPE_PATH).expect("reviewed scope");
    assert_eq!(
        reviewed.canonical().as_str(),
        r#"{"source_refs":["primary","secondary"],"version":"mfm.evm-live.reviewed-source-scope.v1"}"#
    );
    assert_eq!(
        support_content_ref(reviewed).expect("reviewed source ref"),
        reviewed_source_ref
    );
    assert_eq!(
        reviewed.value_contract().role().as_str(),
        EVM_REVIEWED_SOURCE_SCOPE_ROLE
    );
    assert!(members
        .iter()
        .all(|member| !member.field_path().as_str().contains("operation_catalog")));
    for member in &members {
        for forbidden in [
            "127.0.0.1",
            "sentinel-secret",
            "authorization",
            "endpoint",
            "rpc_url",
        ] {
            assert!(
                !member.canonical().as_str().contains(forbidden),
                "support member {} exposed {forbidden}",
                member.field_path().as_str()
            );
        }
    }
    assert_eq!(
        evm_routing_generation_path(MAX_EVM_ROUTING_GENERATIONS - 1).expect("last generation path"),
        "capability.evm_live.routing_generation.4095"
    );
    assert!(evm_routing_generation_path(MAX_EVM_ROUTING_GENERATIONS).is_err());
}

#[test]
fn evm_live_binding_closes_over_the_one_fixed_adapter_and_dynamic_contracts() {
    let (executable, executable_bytes) = test_executable();
    let (executor_contract, executor_deployment, resource_ownership) = test_executor_material();
    let product =
        qualify_product_components(executable, executable_bytes.as_bytes(), &executor_contract)
            .expect("product qualification");
    let transport = test_evm_transport();
    let (route_generation_ref, initial_nonce, signer) =
        test_wallet_qualification_inputs(&transport, &executor_deployment, &resource_ownership);
    let live = qualify_evm_live_support(
        &transport,
        &product,
        executor_contract,
        executor_deployment,
        resource_ownership,
        route_generation_ref,
        initial_nonce,
        &signer,
    )
    .expect("live EVM qualification");

    assert_eq!(
        live.support_members.len(),
        EVM_LIVE_FIXED_SUPPORT_MEMBER_COUNT + 2
    );
    let fields = live
        .read_capability_binding
        .fields()
        .expect("binding fields");
    assert_eq!(
        fields.capability_contract_ref,
        mfm_evm::evm_read_capability_contract_ref().expect("capability ref")
    );
    assert_eq!(
        fields.admitted_implementation_ref,
        product
            .implementations
            .evm_read_adapter
            .content_ref()
            .expect("adapter implementation ref")
    );
    assert_eq!(
        support_content_ref(
            support_member_at(&live.support_members, EVM_READ_CAPABILITY_BINDING_PATH)
                .expect("binding member")
        )
        .expect("binding ref"),
        live.read_capability_binding_ref
    );
    assert_eq!(
        support_member_at(&live.support_members, EVM_READ_CAPABILITY_BINDING_PATH)
            .expect("binding member")
            .value_contract()
            .role()
            .as_str(),
        EVM_READ_CAPABILITY_BINDING_ROLE
    );
    assert_eq!(
        support_content_ref(
            support_member_at(&live.support_members, EVM_EXECUTOR_BINDING_PATH)
                .expect("executor binding member")
        )
        .expect("executor binding ref"),
        live.executor_binding.binding_ref().as_content_ref().clone()
    );
    let wallet_paths = [
        (
            EVM_WALLET_SIGNER_BINDING_PATH,
            EVM_WALLET_SIGNER_BINDING_ROLE,
            "semantic:mfm.signing:",
        ),
        (
            EVM_WALLET_NONCE_POLICY_PATH,
            EVM_WALLET_NONCE_POLICY_ROLE,
            "semantic:mfm.evm:",
        ),
        (
            EVM_WALLET_INITIAL_NONCE_PATH,
            EVM_WALLET_INITIAL_NONCE_ROLE,
            "semantic:mfm.evm:",
        ),
        (
            EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_PATH,
            EVM_WALLET_ALREADY_KNOWN_CLASSIFIER_ROLE,
            "semantic:mfm.evm-live:",
        ),
        (
            EVM_WALLET_FINALITY_POLICY_PATH,
            EVM_WALLET_FINALITY_POLICY_ROLE,
            "semantic:mfm.evm:",
        ),
        (
            EVM_WALLET_ASSURANCE_POLICY_PATH,
            EVM_WALLET_ASSURANCE_POLICY_ROLE,
            "semantic:mfm.evm:",
        ),
        (
            EVM_WALLET_REQUEST_QUALIFICATION_PATH,
            EVM_WALLET_REQUEST_QUALIFICATION_ROLE,
            "semantic:mfm.evm-live:",
        ),
    ];
    for (path, role, semantic_prefix) in wallet_paths {
        let member = support_member_at(&live.support_members, path).expect("wallet support member");
        assert_eq!(member.value_contract().role().as_str(), role);
        assert!(member
            .value_contract()
            .semantic_type_id()
            .as_str()
            .starts_with(semantic_prefix));
        for forbidden in [
            "http://",
            "https://",
            "authorization",
            "private_key",
            "keystore",
            "unlock",
            "signature",
            "signed_bytes",
            "provider_body",
            "mnemonic",
            "password",
        ] {
            assert!(
                !member.canonical().as_str().contains(forbidden),
                "{path} exposed {forbidden}",
            );
        }
    }
    assert!(wallet_support_members_match(
        &live.support_members,
        live.wallet_request_qualification.as_ref(),
    )
    .expect("wallet support closure"));
    let qualification_wire: Value =
        serde_json::from_slice(live.wallet_request_qualification.canonical().as_bytes())
            .expect("qualification JSON");
    let qualification = live.wallet_request_qualification.as_ref();
    let binding = qualification.executor_binding();
    let ownership = binding.resource_ownership().expect("resource ownership");
    let bounds = binding.contract().evidence_bounds();
    let route_chain_map = qualification
        .route_chain_map()
        .iter()
        .map(|(reference, chain_id)| {
            serde_json::json!({
                "chain_id": chain_id,
                "route_generation_ref": reference,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        qualification_wire,
        serde_json::json!({
            "already_known_classifier_ref": qualification.already_known_classifier_ref(),
            "assurance_policy_ref": qualification.assurance_policy_ref(),
            "chain_id": qualification.chain_id(),
            "evidence_bounds": {
                "completion_reserve_bytes": bounds.completion_reserve_bytes().to_string(),
                "completion_reserve_records": bounds.completion_reserve_records(),
                "max_attempts": bounds.max_attempts(),
                "max_completion_record_bytes": bounds.max_completion_record_bytes().to_string(),
                "max_records": bounds.max_records(),
                "max_retained_bytes": bounds.max_retained_bytes().to_string(),
            },
            "executor_binding_ref": binding.binding_ref().as_content_ref(),
            "executor_generation_ref": binding.deployment().durable_ledger_generation_ref(),
            "finality_policy_ref": qualification.finality_policy_ref(),
            "generation_fence_ref": qualification.generation_fence_ref(),
            "initial_nonce": qualification
                .initial_nonce_descriptor()
                .initial_nonce()
                .expect("initial nonce")
                .to_string(),
            "nonce_configuration_ref": qualification
                .resource_policy_binding()
                .policy_configuration_ref(),
            "nonce_policy_ref": qualification.resource_policy_binding().policy_ref(),
            "object_evidence_contract_ref": qualification.object_evidence_contract_ref(),
            "resource_ownership_ref": ownership
                .reference()
                .expect("ownership ref")
                .as_content_ref(),
            "route_chain_map": route_chain_map,
            "route_generation_ref": qualification
                .route_generation_ref()
                .to_content_ref()
                .expect("route generation ref"),
            "routing_catalog_ref": qualification.routing_catalog_ref(),
            "sender": format!("{:#x}", qualification.sender()),
            "signer_descriptor_ref": qualification.signer_descriptor().reference(),
            "tenant_scope_id": binding.deployment().tenant_scope_id(),
            "version": "mfm.evm-live.wallet-request-qualification.v1",
            "wallet_domain_ref": qualification.wallet_domain_ref(),
        }),
    );

    let evidence_ref = product.object_evidence_contract_ref.clone();
    let admitted_implementation_ref = product
        .implementations
        .evm_read_adapter
        .content_ref()
        .expect("adapter implementation");
    let mut removed = live.support_members.clone();
    removed.retain(|member| member.field_path().as_str() != EVM_WALLET_REQUEST_QUALIFICATION_PATH);
    assert!(validate_evm_live_support_closure(
        &transport,
        &live.read_capability_binding,
        &live.read_capability_binding_ref,
        &admitted_implementation_ref,
        &evidence_ref,
        &live.executor_binding,
        live.wallet_request_qualification.as_ref(),
        &removed,
    )
    .is_err());
    let mut substituted = live.support_members.clone();
    let member = substituted
        .iter_mut()
        .find(|member| member.field_path().as_str() == EVM_WALLET_ASSURANCE_POLICY_PATH)
        .expect("assurance member");
    *member = QualifiedSupportMember::new(
        member.field_path().clone(),
        PlainCanonicalJsonBytes::from_json_str("{}").expect("substitute canonical"),
        member.value_contract().clone(),
    );
    assert!(validate_evm_live_support_closure(
        &transport,
        &live.read_capability_binding,
        &live.read_capability_binding_ref,
        &admitted_implementation_ref,
        &evidence_ref,
        &live.executor_binding,
        live.wallet_request_qualification.as_ref(),
        &substituted,
    )
    .is_err());
}

#[test]
fn wallet_qualification_rejects_route_chain_signer_and_executor_semantic_substitution() {
    {
        let (executable, executable_bytes) = test_executable();
        let (contract, deployment, ownership) = test_executor_material();
        let product =
            qualify_product_components(executable, executable_bytes.as_bytes(), &contract)
                .expect("product");
        let transport = test_evm_transport();
        let (_, initial_nonce, signer) =
            test_wallet_qualification_inputs(&transport, &deployment, &ownership);
        let missing_route =
            mfm_evm::EvmRoutingGenerationRef::from_content_ref(test_content_ref("missing-route"))
                .expect("route ref");
        assert!(qualify_evm_live_support(
            &transport,
            &product,
            contract,
            deployment,
            ownership,
            missing_route,
            initial_nonce,
            &signer,
        )
        .is_err());
    }

    {
        let (executable, executable_bytes) = test_executable();
        let (contract, deployment, ownership) = test_executor_material();
        let product =
            qualify_product_components(executable, executable_bytes.as_bytes(), &contract)
                .expect("product");
        let transport = test_evm_transport();
        let (route, initial_nonce, signer) =
            test_wallet_qualification_inputs(&transport, &deployment, &ownership);
        let wrong_chain_nonce = mfm_evm::EvmWalletInitialNonceDescriptor::new(
            initial_nonce.initial_nonce().expect("nonce"),
            initial_nonce.source_attestation_ref().clone(),
            initial_nonce.wallet_domain_ref().clone(),
            initial_nonce.chain_id() + 1,
            initial_nonce.sender_address().expect("sender"),
            initial_nonce.durable_generation_ref().clone(),
        )
        .expect("wrong-chain descriptor");
        assert!(qualify_evm_live_support(
            &transport,
            &product,
            contract,
            deployment,
            ownership,
            route,
            wrong_chain_nonce,
            &signer,
        )
        .is_err());
    }

    {
        let (executable, executable_bytes) = test_executable();
        let (contract, deployment, ownership) = test_executor_material();
        let product =
            qualify_product_components(executable, executable_bytes.as_bytes(), &contract)
                .expect("product");
        let transport = test_evm_transport();
        let (route, initial_nonce, signer) =
            test_wallet_qualification_inputs(&transport, &deployment, &ownership);
        let alternate_algorithm =
            mfm_signing::SigningAlgorithmId::new("secp256k1.alternate.recoverable")
                .expect("algorithm");
        let wrong_signer = mfm_signing::VerifiedGenerationGuardedSignerBinding::verify(
            signer.signer_ref().clone(),
            signer.provider_implementation_id().as_str(),
            alternate_algorithm.clone(),
            signer.profile().clone(),
            mfm_signing::PublicSigningIdentity::new(
                alternate_algorithm,
                None,
                signer
                    .expected_public_identity()
                    .account_id()
                    .map(str::to_owned),
            )
            .expect("identity"),
            signer.durable_generation_ref().clone(),
            signer.fence_attestation_ref().clone(),
            signer.direct_sign_exclusion_ref().clone(),
        )
        .expect("wrong signer");
        assert!(qualify_evm_live_support(
            &transport,
            &product,
            contract,
            deployment,
            ownership,
            route,
            initial_nonce,
            &wrong_signer,
        )
        .is_err());
    }

    {
        let (executable, executable_bytes) = test_executable();
        let (contract, deployment, ownership) =
            test_executor_material_with_safe_failure(test_content_ref("wrong-safe-failure"));
        let product =
            qualify_product_components(executable, executable_bytes.as_bytes(), &contract)
                .expect("product");
        let transport = test_evm_transport();
        let (route, initial_nonce, signer) =
            test_wallet_qualification_inputs(&transport, &deployment, &ownership);
        assert!(qualify_evm_live_support(
            &transport,
            &product,
            contract,
            deployment,
            ownership,
            route,
            initial_nonce,
            &signer,
        )
        .is_err());
    }
}

#[test]
fn wallet_qualification_rejects_every_guarded_signer_nonce_and_executor_axis() {
    let transport = test_evm_transport();
    let (contract, deployment, ownership) = test_executor_material();
    let (route, initial_nonce, signer) =
        test_wallet_qualification_inputs(&transport, &deployment, &ownership);
    let evidence_ref =
        mfm_values::component_object_evidence_contract_ref().expect("object evidence ref");
    let binding =
        test_verified_executor_binding(contract.clone(), deployment.clone(), ownership.clone());
    mfm_evm_live::EvmWalletRequestQualification::qualify(
        &transport,
        route.clone(),
        binding.clone(),
        &signer,
        initial_nonce.clone(),
        evidence_ref.clone(),
    )
    .expect("valid qualification");

    for mutation in [
        TestExecutorContractMutation::SemanticRequestContract,
        TestExecutorContractMutation::SemanticRequestEvidence,
        TestExecutorContractMutation::SafeFailureEvidence,
        TestExecutorContractMutation::AttemptResultContract,
        TestExecutorContractMutation::AttemptResultEvidence,
        TestExecutorContractMutation::EnsureResultEvidence,
        TestExecutorContractMutation::DeliveryAuditEvidence,
        TestExecutorContractMutation::ExecutorFrontierEvidence,
        TestExecutorContractMutation::TerminalEvidenceEvidence,
        TestExecutorContractMutation::TerminalTombstoneEvidence,
        TestExecutorContractMutation::TerminalProofEvidence,
        TestExecutorContractMutation::SafeFailureReference,
        TestExecutorContractMutation::DownstreamCallback,
        TestExecutorContractMutation::MissingPlanExpansion,
        TestExecutorContractMutation::WrongPlanExpansion,
        TestExecutorContractMutation::ExtraPlanExpansion,
    ] {
        let hostile_contract = test_executor_contract_mutation(&contract, mutation);
        let hostile_binding =
            test_verified_executor_binding(hostile_contract, deployment.clone(), ownership.clone());
        assert!(
            mfm_evm_live::EvmWalletRequestQualification::qualify(
                &transport,
                route.clone(),
                hostile_binding,
                &signer,
                initial_nonce.clone(),
                evidence_ref.clone(),
            )
            .is_err(),
            "executor mutation was admitted",
        );
    }
    let wrong_domain_contract =
        test_executor_contract_mutation(&contract, TestExecutorContractMutation::ResourceDomain);
    assert!(try_test_verified_executor_binding(
        wrong_domain_contract,
        deployment.clone(),
        ownership.clone(),
    )
    .is_err());
    assert!(mfm_evm_live::EvmWalletRequestQualification::qualify(
        &transport,
        route.clone(),
        binding.clone(),
        &signer,
        initial_nonce.clone(),
        test_content_ref("wrong-object-evidence"),
    )
    .is_err(),);

    let signer_variants = [
        test_signer_variant(
            &signer,
            mfm_signing::SigningProfileId::new("secp256k1.alternate.profile")
                .expect("alternate profile"),
            signer
                .expected_public_identity()
                .account_id()
                .expect("account"),
            signer.durable_generation_ref().clone(),
            signer.fence_attestation_ref().clone(),
        ),
        test_signer_variant(
            &signer,
            signer.profile().clone(),
            "0x2222222222222222222222222222222222222222",
            signer.durable_generation_ref().clone(),
            signer.fence_attestation_ref().clone(),
        ),
        test_signer_variant(
            &signer,
            signer.profile().clone(),
            signer
                .expected_public_identity()
                .account_id()
                .expect("account"),
            test_content_ref("wrong-signer-generation"),
            signer.fence_attestation_ref().clone(),
        ),
        test_signer_variant(
            &signer,
            signer.profile().clone(),
            signer
                .expected_public_identity()
                .account_id()
                .expect("account"),
            signer.durable_generation_ref().clone(),
            test_content_ref("wrong-signer-fence"),
        ),
    ];
    for hostile_signer in signer_variants {
        assert!(
            mfm_evm_live::EvmWalletRequestQualification::qualify(
                &transport,
                route.clone(),
                binding.clone(),
                &hostile_signer,
                initial_nonce.clone(),
                evidence_ref.clone(),
            )
            .is_err(),
            "signer mutation was admitted",
        );
    }

    let nonce_variants = [
        mfm_evm::EvmWalletInitialNonceDescriptor::new(
            initial_nonce.initial_nonce().expect("nonce"),
            initial_nonce.source_attestation_ref().clone(),
            mfm_evm::EvmWalletReference::from_content_ref(test_content_ref("wrong-wallet-domain")),
            initial_nonce.chain_id(),
            initial_nonce.sender_address().expect("sender"),
            initial_nonce.durable_generation_ref().clone(),
        )
        .expect("domain-mutated nonce"),
        mfm_evm::EvmWalletInitialNonceDescriptor::new(
            initial_nonce.initial_nonce().expect("nonce"),
            initial_nonce.source_attestation_ref().clone(),
            initial_nonce.wallet_domain_ref().clone(),
            initial_nonce.chain_id(),
            alloy_primitives::Address::from([0x22; 20]),
            initial_nonce.durable_generation_ref().clone(),
        )
        .expect("sender-mutated nonce"),
        mfm_evm::EvmWalletInitialNonceDescriptor::new(
            initial_nonce.initial_nonce().expect("nonce"),
            initial_nonce.source_attestation_ref().clone(),
            initial_nonce.wallet_domain_ref().clone(),
            initial_nonce.chain_id(),
            initial_nonce.sender_address().expect("sender"),
            mfm_evm::EvmWalletReference::from_content_ref(test_content_ref(
                "wrong-nonce-generation",
            )),
        )
        .expect("generation-mutated nonce"),
    ];
    for hostile_nonce in nonce_variants {
        assert!(
            mfm_evm_live::EvmWalletRequestQualification::qualify(
                &transport,
                route.clone(),
                binding.clone(),
                &signer,
                hostile_nonce,
                evidence_ref.clone(),
            )
            .is_err(),
            "nonce mutation was admitted",
        );
    }

    let unfenced_ownership = ResourceOwnership::new(
        ownership.coordination_namespace_ref().clone(),
        ownership.external_resource_domain_ref().clone(),
        ownership.durable_ledger_generation_ref().clone(),
        None,
    )
    .expect("unfenced ownership");
    let unfenced_deployment = ExecutorDeployment::new(
        deployment.executor_namespace_ref().clone(),
        deployment.durable_ledger_generation_ref().clone(),
        deployment.tenant_scope_id().clone(),
        deployment.evidence_authority_ref().clone(),
        Some(
            unfenced_ownership
                .reference()
                .expect("unfenced ownership ref"),
        ),
    )
    .expect("unfenced deployment");
    let unfenced_binding =
        test_verified_executor_binding(contract, unfenced_deployment, unfenced_ownership);
    assert!(mfm_evm_live::EvmWalletRequestQualification::qualify(
        &transport,
        route,
        unfenced_binding,
        &signer,
        initial_nonce,
        evidence_ref,
    )
    .is_err(),);
}

#[test]
fn wallet_qualification_closes_over_every_route_in_the_sealed_catalog() {
    let transport = test_evm_transport_with_unrelated_route(false);
    let expanded_transport = test_evm_transport_with_unrelated_route(true);
    let selected = transport
        .routing_generation_descriptors()
        .find(|(_, descriptor)| descriptor.chain_id() == 1)
        .map(|(reference, _)| reference.clone())
        .expect("selected route");
    let expanded_selected = expanded_transport
        .routing_generation_descriptors()
        .find(|(_, descriptor)| descriptor.chain_id() == 1)
        .map(|(reference, _)| reference.clone())
        .expect("expanded selected route");
    assert_eq!(selected, expanded_selected);

    let qualification = test_wallet_qualification(&transport);
    let expanded_qualification = test_wallet_qualification(&expanded_transport);
    assert_eq!(
        qualification.route_generation_ref(),
        expanded_qualification.route_generation_ref(),
    );
    assert_eq!(qualification.chain_id(), expanded_qualification.chain_id());
    assert_ne!(
        qualification.routing_catalog_ref(),
        expanded_qualification.routing_catalog_ref(),
    );
    assert_ne!(
        qualification.route_chain_map(),
        expanded_qualification.route_chain_map(),
    );
    assert_ne!(
        qualification.canonical(),
        expanded_qualification.canonical(),
    );
    assert_ne!(
        qualification.reference(),
        expanded_qualification.reference(),
    );
}

#[test]
fn wallet_admission_gate_rejects_before_the_single_certify_and_append_path() {
    let (executable, executable_bytes) = test_executable();
    let (contract, deployment, ownership) = test_executor_material();
    let product = qualify_product_components(executable, executable_bytes.as_bytes(), &contract)
        .expect("product");
    let transport = test_evm_transport();
    let (route, initial_nonce, signer) =
        test_wallet_qualification_inputs(&transport, &deployment, &ownership);
    let live = qualify_evm_live_support(
        &transport,
        &product,
        contract,
        deployment,
        ownership,
        route,
        initial_nonce,
        &signer,
    )
    .expect("live qualification");
    let qualification = live.wallet_request_qualification.as_ref();
    let selector = mfm_evm::EvmSubmitTransactionSelector::new(
        mfm_evm::EvmTransactionTarget::new("primary").expect("target"),
    );
    let tenant = qualification
        .executor_binding()
        .deployment()
        .tenant_scope_id();
    for alternate_convergence in [false, true] {
        let request = test_qualified_wallet_request(
            qualification,
            qualification.sender(),
            alternate_convergence,
        );
        let canonical = mfm_program::encode_boundary(&request).expect("request canonical");
        super::super::qualify_wallet_admission_request(
            qualification,
            tenant,
            &selector,
            canonical.as_bytes(),
        )
        .expect("qualified admission request");
    }

    let hostile = test_qualified_wallet_request(
        qualification,
        alloy_primitives::Address::from([0x33; 20]),
        false,
    );
    let hostile = mfm_program::encode_boundary(&hostile).expect("hostile request");
    assert!(super::super::qualify_wallet_admission_request(
        qualification,
        tenant,
        &selector,
        hostile.as_bytes(),
    )
    .is_err());
}

#[test]
fn support_scope_is_order_independent_and_binds_every_member_tuple() {
    let (executable, executable_bytes) = test_executable();
    let (executor_contract, executor_deployment, resource_ownership) = test_executor_material();
    let product =
        qualify_product_components(executable, executable_bytes.as_bytes(), &executor_contract)
            .expect("product qualification");
    let transport = test_evm_transport();
    let (route_generation_ref, initial_nonce, signer) =
        test_wallet_qualification_inputs(&transport, &executor_deployment, &resource_ownership);
    let live = qualify_evm_live_support(
        &transport,
        &product,
        executor_contract,
        executor_deployment,
        resource_ownership,
        route_generation_ref,
        initial_nonce,
        &signer,
    )
    .expect("live EVM qualification");
    let mut members = product.support_members.clone();
    members.extend(live.support_members);

    let expected = qualified_support_scope_id(&members).expect("support scope");
    members.reverse();
    assert_eq!(
        qualified_support_scope_id(&members).expect("reordered support scope"),
        expected
    );

    let changed_path = members[0].field_path().clone();
    let changed_contract = members[0].value_contract().clone();
    members[0] = QualifiedSupportMember::new(
        changed_path,
        PlainCanonicalJsonBytes::from_json_str("{}").expect("changed canonical"),
        changed_contract,
    );
    assert_ne!(
        qualified_support_scope_id(&members).expect("changed support scope"),
        expected
    );

    members.push(QualifiedSupportMember::new(
        members[0].field_path().clone(),
        members[0].canonical().clone(),
        members[0].value_contract().clone(),
    ));
    assert!(qualified_support_scope_id(&members).is_err());
}

#[test]
fn product_deployment_graph_has_exact_sixty_eight_plus_generation_closure() {
    let (executable, executable_bytes) = test_executable();
    let (executor_contract, executor_deployment, resource_ownership) = test_executor_material();
    let product =
        qualify_product_components(executable, executable_bytes.as_bytes(), &executor_contract)
            .expect("product qualification");
    let transport = test_evm_transport();
    let routing_manifest = test_routing_manifest(&transport);
    let (route_generation_ref, initial_nonce, signer) =
        test_wallet_qualification_inputs(&transport, &executor_deployment, &resource_ownership);
    let live = qualify_evm_live_support(
        &transport,
        &product,
        executor_contract,
        executor_deployment,
        resource_ownership,
        route_generation_ref,
        initial_nonce,
        &signer,
    )
    .expect("live qualification");
    let wallet_request_qualification = Arc::clone(&live.wallet_request_qualification);
    let deployment = assemble_qualified_product_deployment(product, live, &routing_manifest)
        .expect("product deployment");
    assert!(Arc::ptr_eq(
        &wallet_request_qualification,
        &deployment.wallet_request_qualification,
    ));

    assert_eq!(
        deployment.support_graph.members().len(),
        PRODUCT_DEPLOYMENT_SUPPORT_MEMBER_COUNT_WITHOUT_GENERATIONS + 2
    );
    assert_eq!(
        deployment.state_manifest.entries().len(),
        STATE_COMPONENT_COUNT
    );
    assert_eq!(
        deployment.capability_manifest.entries().len(),
        mfm_evm::EVM_READ_OPERATION_IDS.len() + 1
    );
    assert!(deployment
        .capability_manifest
        .entries()
        .iter()
        .filter(|entry| {
            entry.operation_id.as_str() != mfm_evm::EVM_SUBMIT_TRANSACTION_OPERATION_ID
        })
        .all(|entry| entry.binding_ref == deployment.read_capability_binding_ref));
    assert_eq!(
        deployment
            .capability_manifest
            .entries()
            .iter()
            .find(|entry| {
                entry.operation_id.as_str() == mfm_evm::EVM_SUBMIT_TRANSACTION_OPERATION_ID
            })
            .expect("wallet capability entry")
            .binding_ref,
        deployment
            .executor_binding
            .binding_ref()
            .as_content_ref()
            .clone()
    );
    assert_eq!(
        support_content_ref(
            deployment
                .support_graph
                .members()
                .get(
                    &FieldPath::new(STATE_IMPLEMENTATION_MANIFEST_PATH)
                        .expect("state manifest path")
                )
                .expect("state manifest member")
        )
        .expect("state manifest ref"),
        deployment.state_manifest_ref
    );
    assert_eq!(
        support_content_ref(
            deployment
                .support_graph
                .members()
                .get(
                    &FieldPath::new(CAPABILITY_BINDING_MANIFEST_PATH)
                        .expect("capability manifest path")
                )
                .expect("capability manifest member")
        )
        .expect("capability manifest ref"),
        deployment.capability_manifest_ref
    );
    let fact_objects = mfm_evm::evm_balance_fact_support_objects().expect("fact support objects");
    let complete_members = deployment
        .support_graph
        .members()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    validate_product_deployment_support_closure(
        &complete_members,
        2,
        &deployment.object_evidence_contract_ref,
        &deployment.state_manifest_ref,
        &deployment.capability_manifest_ref,
        &fact_objects,
        &deployment.executor_binding,
        deployment.wallet_request_qualification.as_ref(),
    )
    .expect("complete product closure");
    let mut removed = complete_members.clone();
    removed.retain(|member| member.field_path().as_str() != EVM_WALLET_SIGNER_BINDING_PATH);
    assert!(validate_product_deployment_support_closure(
        &removed,
        2,
        &deployment.object_evidence_contract_ref,
        &deployment.state_manifest_ref,
        &deployment.capability_manifest_ref,
        &fact_objects,
        &deployment.executor_binding,
        deployment.wallet_request_qualification.as_ref(),
    )
    .is_err());
    let mut substituted = complete_members;
    let member = substituted
        .iter_mut()
        .find(|member| member.field_path().as_str() == EVM_WALLET_INITIAL_NONCE_PATH)
        .expect("initial nonce member");
    *member = QualifiedSupportMember::new(
        member.field_path().clone(),
        PlainCanonicalJsonBytes::from_json_str("{}").expect("substitute canonical"),
        member.value_contract().clone(),
    );
    assert!(validate_product_deployment_support_closure(
        &substituted,
        2,
        &deployment.object_evidence_contract_ref,
        &deployment.state_manifest_ref,
        &deployment.capability_manifest_ref,
        &fact_objects,
        &deployment.executor_binding,
        deployment.wallet_request_qualification.as_ref(),
    )
    .is_err());
    assert!(deployment
        .support_graph
        .members()
        .values()
        .all(|member| member.value_contract().evidence_contract_ref()
            == &deployment.object_evidence_contract_ref));
}

fn test_executor_material() -> (
    ExecutorContractDescriptor,
    ExecutorDeployment,
    ResourceOwnership,
) {
    test_executor_material_with_safe_failure(
        mfm_evm::evm_safe_failure_contract_ref().expect("safe failure contract"),
    )
}

fn test_executor_material_with_safe_failure(
    safe_failure_contract_ref: ContentRef,
) -> (
    ExecutorContractDescriptor,
    ExecutorDeployment,
    ResourceOwnership,
) {
    let evidence_ref =
        mfm_values::component_object_evidence_contract_ref().expect("object evidence ref");
    let value_contracts = mfm_evm::evm_submit_transaction_value_contracts(evidence_ref.clone())
        .expect("wallet value contracts");
    let retained = mfm_executor::ExecutorRetainedClosureContract::new(
        test_executor_retained(
            "ensure-result",
            "mfm.executor-ensure-result.v1",
            evidence_ref.clone(),
        ),
        test_executor_retained(
            "delivery-audit",
            "mfm.executor-delivery-frontier.v1",
            evidence_ref.clone(),
        ),
        test_executor_retained(
            "executor-frontier",
            "mfm.executor-delivery-frontier.v1",
            evidence_ref.clone(),
        ),
        test_executor_retained(
            "terminal-evidence",
            "mfm.terminal-effect-evidence.v1",
            evidence_ref.clone(),
        ),
        test_executor_retained(
            "terminal-tombstone",
            "mfm.executor-terminal-tombstone.v1",
            evidence_ref.clone(),
        ),
        test_executor_retained(
            "terminal-proof",
            "mfm.executor-reference-terminal-proof.v1",
            evidence_ref.clone(),
        ),
        value_contracts.attempt_result().clone(),
    )
    .expect("executor retained closure");
    let resource_domain_ref = test_content_ref("wallet-domain");
    let target_surface =
        mfm_evm_live::evm_wallet_target_callback_surface_ref().expect("target surface");
    let contract = ExecutorContractDescriptor::new(
        test_content_ref("ensure-contract"),
        value_contracts.request().clone(),
        test_executor_retained("safe-failure", "mfm.safe-failure.v1", evidence_ref),
        retained,
        safe_failure_contract_ref,
        target_surface.clone(),
        mfm_executor::EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 32 * 1024, 2, 32 * 1024)
            .expect("evidence bounds"),
        Some(resource_domain_ref.clone()),
        vec![
            mfm_evm::evm_submit_transaction_leaf_expansion(target_surface).expect("leaf expansion"),
        ],
    )
    .expect("executor contract");
    let generation_ref = test_content_ref("wallet-generation");
    let ownership = ResourceOwnership::new(
        test_content_ref("wallet-coordination"),
        resource_domain_ref,
        generation_ref.clone(),
        Some(test_content_ref("wallet-generation-fence")),
    )
    .expect("resource ownership");
    let tenant_scope_id =
        mfm_ids::TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000061")
            .expect("tenant");
    let deployment = ExecutorDeployment::new(
        test_content_ref("executor-namespace"),
        generation_ref,
        tenant_scope_id,
        test_content_ref("evidence-authority"),
        Some(ownership.reference().expect("ownership ref")),
    )
    .expect("executor deployment");
    (contract, deployment, ownership)
}

fn test_executor_retained(
    label: &str,
    schema_contract: &str,
    evidence_ref: ContentRef,
) -> RetainedValueContract {
    let role = format!("mfm.test.evm-wallet.{label}");
    retained_contract(
        annex_schema_id(schema_contract).expect("annex schema"),
        label,
        &role,
        evidence_ref,
    )
    .expect("retained contract")
}

#[derive(Clone, Copy)]
enum TestExecutorContractMutation {
    SemanticRequestContract,
    SemanticRequestEvidence,
    SafeFailureEvidence,
    AttemptResultContract,
    AttemptResultEvidence,
    EnsureResultEvidence,
    DeliveryAuditEvidence,
    ExecutorFrontierEvidence,
    TerminalEvidenceEvidence,
    TerminalTombstoneEvidence,
    TerminalProofEvidence,
    SafeFailureReference,
    DownstreamCallback,
    MissingPlanExpansion,
    WrongPlanExpansion,
    ExtraPlanExpansion,
    ResourceDomain,
}

fn test_executor_contract_mutation(
    contract: &ExecutorContractDescriptor,
    mutation: TestExecutorContractMutation,
) -> ExecutorContractDescriptor {
    let wrong_evidence = test_content_ref("wrong-retained-evidence");
    let retained = contract.retained_closure_contract();
    let retained = mfm_executor::ExecutorRetainedClosureContract::new(
        test_retained_evidence_variant(
            retained.ensure_result_contract(),
            matches!(mutation, TestExecutorContractMutation::EnsureResultEvidence),
            &wrong_evidence,
        ),
        test_retained_evidence_variant(
            retained.delivery_audit_contract(),
            matches!(
                mutation,
                TestExecutorContractMutation::DeliveryAuditEvidence
            ),
            &wrong_evidence,
        ),
        test_retained_evidence_variant(
            retained.executor_frontier_contract(),
            matches!(
                mutation,
                TestExecutorContractMutation::ExecutorFrontierEvidence
            ),
            &wrong_evidence,
        ),
        test_retained_evidence_variant(
            retained.terminal_evidence_contract(),
            matches!(
                mutation,
                TestExecutorContractMutation::TerminalEvidenceEvidence
            ),
            &wrong_evidence,
        ),
        test_retained_evidence_variant(
            retained.terminal_tombstone_contract(),
            matches!(
                mutation,
                TestExecutorContractMutation::TerminalTombstoneEvidence
            ),
            &wrong_evidence,
        ),
        test_retained_evidence_variant(
            retained.terminal_proof_contract(),
            matches!(
                mutation,
                TestExecutorContractMutation::TerminalProofEvidence
            ),
            &wrong_evidence,
        ),
        if matches!(
            mutation,
            TestExecutorContractMutation::AttemptResultEvidence
        ) {
            test_retained_with_evidence(retained.domain_evidence_contract(), wrong_evidence.clone())
        } else if matches!(
            mutation,
            TestExecutorContractMutation::AttemptResultContract
        ) {
            test_retained_contract_substitution(retained.domain_evidence_contract())
        } else {
            retained.domain_evidence_contract().clone()
        },
    )
    .expect("mutated retained closure");
    ExecutorContractDescriptor::new(
        contract.ensure_contract_ref().clone(),
        if matches!(
            mutation,
            TestExecutorContractMutation::SemanticRequestContract
        ) {
            test_retained_contract_substitution(contract.semantic_request_contract())
        } else if matches!(
            mutation,
            TestExecutorContractMutation::SemanticRequestEvidence
        ) {
            test_retained_with_evidence(
                contract.semantic_request_contract(),
                wrong_evidence.clone(),
            )
        } else {
            contract.semantic_request_contract().clone()
        },
        if matches!(mutation, TestExecutorContractMutation::SafeFailureEvidence) {
            test_retained_with_evidence(contract.safe_failure_value_contract(), wrong_evidence)
        } else {
            contract.safe_failure_value_contract().clone()
        },
        retained,
        if matches!(mutation, TestExecutorContractMutation::SafeFailureReference) {
            test_content_ref("wrong-safe-failure-contract")
        } else {
            contract.safe_failure_contract_ref().clone()
        },
        if matches!(mutation, TestExecutorContractMutation::DownstreamCallback) {
            test_content_ref("wrong-downstream-callback")
        } else {
            contract.downstream_convergence_contract_ref().clone()
        },
        contract.evidence_bounds().clone(),
        if matches!(mutation, TestExecutorContractMutation::ResourceDomain) {
            Some(test_content_ref("wrong-resource-domain"))
        } else {
            contract.resource_domain_requirement().cloned()
        },
        test_plan_expansion_variant(contract, mutation),
    )
    .expect("mutated executor contract")
}

fn test_retained_evidence_variant(
    contract: &RetainedValueContract,
    mutate: bool,
    wrong_evidence: &ContentRef,
) -> RetainedValueContract {
    if mutate {
        test_retained_with_evidence(contract, wrong_evidence.clone())
    } else {
        contract.clone()
    }
}

fn test_retained_with_evidence(
    contract: &RetainedValueContract,
    evidence_contract_ref: ContentRef,
) -> RetainedValueContract {
    RetainedValueContract::new(
        contract.schema_id().clone(),
        contract.semantic_type_id().clone(),
        contract.role().clone(),
        contract.media_type(),
        evidence_contract_ref,
    )
    .expect("retained evidence variant")
}

fn test_retained_contract_substitution(contract: &RetainedValueContract) -> RetainedValueContract {
    RetainedValueContract::new(
        contract.schema_id().clone(),
        contract.semantic_type_id().clone(),
        StableId::new("mfm.test.substituted-retained-contract").expect("substituted retained role"),
        contract.media_type(),
        contract.evidence_contract_ref().clone(),
    )
    .expect("retained contract substitution")
}

fn test_plan_expansion_variant(
    contract: &ExecutorContractDescriptor,
    mutation: TestExecutorContractMutation,
) -> Vec<mfm_executor::RequiredPlanExpansion> {
    let expected = contract
        .required_plan_expansions()
        .first()
        .expect("wallet plan expansion");
    if matches!(mutation, TestExecutorContractMutation::MissingPlanExpansion) {
        return Vec::new();
    }
    if matches!(mutation, TestExecutorContractMutation::WrongPlanExpansion) {
        return vec![mfm_executor::RequiredPlanExpansion::new(
            expected.executor_operation_id(),
            expected.typed_boundary_contract_ref().clone(),
            test_content_ref("wrong-plan-expansion"),
        )
        .expect("wrong plan expansion")];
    }
    let mut expansions = contract.required_plan_expansions().to_vec();
    if matches!(mutation, TestExecutorContractMutation::ExtraPlanExpansion) {
        expansions.push(
            mfm_executor::RequiredPlanExpansion::new(
                "mfm.test.extra-wallet-operation",
                test_content_ref("extra-typed-boundary"),
                test_content_ref("extra-plan-expansion"),
            )
            .expect("extra plan expansion"),
        );
    }
    expansions
}

fn try_test_verified_executor_binding(
    contract: ExecutorContractDescriptor,
    deployment: ExecutorDeployment,
    ownership: ResourceOwnership,
) -> mfm_executor::Result<VerifiedExecutorBinding> {
    let binding = ExecutorBinding::new(
        contract.reference()?,
        test_content_ref("wallet-implementation"),
        deployment.reference()?,
    )?;
    let tenant = deployment.tenant_scope_id().clone();
    VerifiedExecutorBinding::verify(binding, contract, deployment, Some(ownership), &tenant)
}

fn test_verified_executor_binding(
    contract: ExecutorContractDescriptor,
    deployment: ExecutorDeployment,
    ownership: ResourceOwnership,
) -> VerifiedExecutorBinding {
    try_test_verified_executor_binding(contract, deployment, ownership)
        .expect("verified executor binding")
}

fn test_signer_variant(
    signer: &mfm_signing::VerifiedGenerationGuardedSignerBinding,
    profile: mfm_signing::SigningProfileId,
    account_id: &str,
    durable_generation_ref: ContentRef,
    fence_attestation_ref: ContentRef,
) -> mfm_signing::VerifiedGenerationGuardedSignerBinding {
    mfm_signing::VerifiedGenerationGuardedSignerBinding::verify(
        signer.signer_ref().clone(),
        signer.provider_implementation_id().as_str(),
        signer.algorithm().clone(),
        profile,
        mfm_signing::PublicSigningIdentity::new(
            signer.algorithm().clone(),
            signer.expected_public_identity().public_key().cloned(),
            Some(account_id.to_owned()),
        )
        .expect("signer identity"),
        durable_generation_ref,
        fence_attestation_ref,
        signer.direct_sign_exclusion_ref().clone(),
    )
    .expect("signer variant")
}

fn test_wallet_qualification_inputs(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
    deployment: &ExecutorDeployment,
    ownership: &ResourceOwnership,
) -> (
    mfm_evm::EvmRoutingGenerationRef,
    mfm_evm::EvmWalletInitialNonceDescriptor,
    mfm_signing::VerifiedGenerationGuardedSignerBinding,
) {
    let (route_generation_ref, route_generation) = transport
        .routing_generation_descriptors()
        .find(|(_, descriptor)| descriptor.chain_id() == 1)
        .expect("selected route");
    let sender = alloy_primitives::Address::from([0x11; 20]);
    let algorithm = mfm_signing::SigningAlgorithmId::new(
        mfm_signing::SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID,
    )
    .expect("algorithm");
    let signer = mfm_signing::VerifiedGenerationGuardedSignerBinding::verify(
        mfm_signing::SignerRef::new("wallet").expect("signer"),
        "mfm.test.wallet-signer",
        algorithm.clone(),
        mfm_signing::SigningProfileId::new(mfm_signing::SECP256K1_RFC6979_LOW_S_PROFILE_ID)
            .expect("profile"),
        mfm_signing::PublicSigningIdentity::new(algorithm, None, Some(format!("{sender:#x}")))
            .expect("identity"),
        deployment.durable_ledger_generation_ref().clone(),
        ownership
            .destination_fencing_authority_ref()
            .expect("generation fence")
            .clone(),
        test_content_ref("direct-sign-exclusion"),
    )
    .expect("signer binding");
    let initial_nonce = mfm_evm::EvmWalletInitialNonceDescriptor::new(
        7,
        mfm_evm::EvmWalletReference::from_content_ref(test_content_ref("nonce-source-attestation")),
        mfm_evm::EvmWalletReference::from_content_ref(
            ownership.external_resource_domain_ref().clone(),
        ),
        route_generation.chain_id(),
        sender,
        mfm_evm::EvmWalletReference::from_content_ref(
            deployment.durable_ledger_generation_ref().clone(),
        ),
    )
    .expect("initial nonce");
    (route_generation_ref.clone(), initial_nonce, signer)
}

fn test_qualified_wallet_request(
    qualification: &mfm_evm_live::EvmWalletRequestQualification,
    sender: alloy_primitives::Address,
    alternate_convergence: bool,
) -> mfm_evm::EvmSubmitTransactionRequest {
    let target = mfm_evm::EvmTransactionTarget::new("primary").expect("target");
    let template = mfm_evm::EvmWalletTransactionTemplate::new(
        target,
        mfm_evm::EvmWalletTransactionAction::call(alloy_primitives::Address::from([0x22; 20])),
        alloy_primitives::U256::from(5),
        [0xde, 0xad],
        Vec::new(),
        alloy_primitives::U256::from(75_000),
    )
    .expect("template");
    let replacement = if alternate_convergence {
        mfm_evm::EvmWalletReplacementPolicy::new(vec![mfm_evm::EvmWalletFeeCandidate::new(
            alloy_primitives::U256::from(25),
            alloy_primitives::U256::from(3),
        )
        .expect("fee")])
        .expect("replacement")
    } else {
        mfm_evm::EvmWalletReplacementPolicy::new(vec![
            mfm_evm::EvmWalletFeeCandidate::new(
                alloy_primitives::U256::from(20),
                alloy_primitives::U256::from(2),
            )
            .expect("fee"),
            mfm_evm::EvmWalletFeeCandidate::new(
                alloy_primitives::U256::from(30),
                alloy_primitives::U256::from(3),
            )
            .expect("fee"),
        ])
        .expect("replacement")
    };
    let convergence = if alternate_convergence {
        mfm_evm::EvmWalletConvergencePlan::new(1, 1, 1, 1, 1, 16 * 1024).expect("convergence")
    } else {
        mfm_evm::EvmWalletConvergencePlan::new(2, 2, 2, 2, 2, 16 * 1024).expect("convergence")
    };
    let policy = mfm_evm::EvmWalletPolicy::new(
        mfm_evm::EvmWalletReference::from_content_ref(qualification.wallet_domain_ref().clone()),
        qualification
            .executor_binding()
            .deployment()
            .tenant_scope_id()
            .clone(),
        mfm_evm::EvmWalletReference::from_content_ref(
            qualification
                .route_generation_ref()
                .to_content_ref()
                .expect("route generation ref"),
        ),
        qualification.chain_id(),
        sender,
        mfm_evm::EvmWalletReference::from_content_ref(
            qualification.signer_descriptor().reference().clone(),
        ),
        mfm_evm::EvmWalletReference::from_content_ref(
            qualification.resource_policy_binding().policy_ref().clone(),
        ),
        qualification
            .initial_nonce_descriptor()
            .initial_nonce()
            .expect("nonce"),
        mfm_evm::EvmWalletReference::from_content_ref(
            qualification
                .resource_policy_binding()
                .policy_configuration_ref()
                .clone(),
        ),
        replacement,
        qualification.already_known_classifier_ref().clone(),
        qualification.finality_policy_ref().clone(),
        qualification.assurance_policy_ref().clone(),
        convergence,
        qualification
            .executor_binding()
            .contract()
            .evidence_bounds()
            .clone(),
    )
    .expect("policy");
    let (_, template_ref) = mfm_program::encode_mfm_value(&template).expect("template ref");
    let (_, policy_ref) = mfm_program::encode_mfm_value(&policy).expect("policy ref");
    mfm_evm::EvmSubmitTransactionRequest::new(
        mfm_evm::EvmWalletReference::from_content_ref(template_ref),
        template,
        mfm_evm::EvmWalletReference::from_content_ref(policy_ref),
        policy,
    )
    .expect("request")
}

fn test_content_ref(label: &str) -> ContentRef {
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&serde_json::json!({"label": label}).to_string())
            .expect("canonical test content");
    exact_ref(
        descriptor_schema_id("mfm.test.evm-wallet-content").expect("test schema"),
        &canonical,
    )
    .expect("test content ref")
}

fn test_executable() -> (ContentRef, PlainCanonicalJsonBytes) {
    let canonical = annex_canonical_bytes(
        EXECUTABLE_DESCRIPTOR_CONTRACT,
        br#"{"contract":"mfm.executable-bytes.v1","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
    )
    .expect("test executable descriptor");
    let reference = exact_ref(
        annex_schema_id(EXECUTABLE_DESCRIPTOR_CONTRACT).expect("executable schema"),
        &canonical,
    )
    .expect("test executable ref");
    (reference, canonical)
}

fn test_evm_transport() -> mfm_evm_live::transport::EvmJsonRpcTransport {
    test_evm_transport_with_unrelated_route(false)
}

fn test_evm_transport_with_unrelated_route(
    include_unrelated_route: bool,
) -> mfm_evm_live::transport::EvmJsonRpcTransport {
    use mfm_evm_live::transport::{
        EvmJsonRpcTransport, EvmRoutingCatalogBuilder, EvmRpcAuthorization, EvmRpcEndpoint,
    };
    use zeroize::Zeroizing;

    let mut catalog = EvmRoutingCatalogBuilder::new();
    catalog
        .insert(
            "z-network",
            "secondary",
            2,
            StableId::new("generation-secondary").expect("secondary generation"),
            EvmRpcEndpoint::new("http://127.0.0.1:9").expect("secondary endpoint"),
            None,
        )
        .expect("secondary route");
    catalog
        .insert(
            "a-network",
            "primary",
            1,
            StableId::new("generation-primary").expect("primary generation"),
            EvmRpcEndpoint::new("http://127.0.0.1:9").expect("primary endpoint"),
            Some(
                EvmRpcAuthorization::new(Zeroizing::new("Bearer sentinel-secret".to_owned()))
                    .expect("primary authorization"),
            ),
        )
        .expect("primary route");
    if include_unrelated_route {
        catalog
            .insert(
                "m-network",
                "unrelated",
                3,
                StableId::new("generation-unrelated").expect("unrelated generation"),
                EvmRpcEndpoint::new("http://127.0.0.1:9").expect("unrelated endpoint"),
                None,
            )
            .expect("unrelated route");
    }
    EvmJsonRpcTransport::new(catalog.build().expect("catalog")).expect("transport")
}

fn test_wallet_qualification(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
) -> Arc<mfm_evm_live::EvmWalletRequestQualification> {
    let (executable, executable_bytes) = test_executable();
    let (contract, deployment, ownership) = test_executor_material();
    let product = qualify_product_components(executable, executable_bytes.as_bytes(), &contract)
        .expect("product");
    let (route, initial_nonce, signer) =
        test_wallet_qualification_inputs(transport, &deployment, &ownership);
    qualify_evm_live_support(
        transport,
        &product,
        contract,
        deployment,
        ownership,
        route,
        initial_nonce,
        &signer,
    )
    .expect("live qualification")
    .wallet_request_qualification
}

fn test_routing_manifest(
    transport: &mfm_evm_live::transport::EvmJsonRpcTransport,
) -> mfm_portfolio::PortfolioRoutingManifest {
    let bindings = transport
        .routing_generation_descriptors()
        .map(|(generation_ref, descriptor)| {
            mfm_portfolio::EvmRoutingBinding::new(descriptor.network_id(), generation_ref.clone())
                .expect("routing binding")
        })
        .collect();
    mfm_portfolio::PortfolioRoutingManifest::new(bindings).expect("routing manifest")
}
