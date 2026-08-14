use super::*;
use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId, StableId};
use mfm_journal::single_trust::{
    ConfigurationHeadProjection, ImmutableObject, PreparationMode, RunAdmitted,
    SequentialControlAddress, StateOutcome, ValueRef,
};
use mfm_program::BindingDescriptor;

fn content(seed: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            DigestBytes::from_array([seed; 32]),
        ),
    )
    .expect("content")
}

fn configuration(seed: u8) -> ConfigurationHeadProjection {
    ConfigurationHeadProjection::new(1, content(seed)).expect("configuration")
}

fn ids() -> (StoreScopeId, TenantScopeId, RunId) {
    (
        StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef").expect("scope"),
        TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef").expect("tenant"),
        RunId::parse(
            "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run"),
    )
}

fn context_value_ref() -> ContentRef {
    ContentRef::new(content(2).schema_id().clone(), raw_content_digest(b"null"))
        .expect("context value")
}

fn value_ref(contract: &ContentRef, json: &str) -> ValueRef {
    ValueRef::new(
        contract.clone(),
        ContentRef::new(
            contract.schema_id().clone(),
            raw_content_digest(json.as_bytes()),
        )
        .expect("value ref"),
    )
}

fn value_object(value: &ValueRef, json: &str) -> ImmutableObject {
    ImmutableObject::new(
        StableId::new("mfm.value").expect("object type"),
        value.value_ref().clone(),
        json.to_owned(),
    )
    .expect("value object")
}

fn program_object(document: &ProgramDocument) -> ImmutableObject {
    ImmutableObject::new(
        StableId::new(PROGRAM_OBJECT_TYPE).expect("program object type"),
        document.program_ref().expect("program ref"),
        document
            .canonical_bytes()
            .expect("program bytes")
            .as_str()
            .to_owned(),
    )
    .expect("program object")
}

fn pure_state(
    ordinal: u32,
    input: ContentRef,
    output: ContentRef,
    terminal: bool,
    next: Option<SequentialControlAddress>,
) -> Declaration {
    let address = SequentialControlAddress::new(ordinal, Vec::new()).expect("address");
    let implementation = content(ordinal as u8 + 20);
    let state = match next {
        Some(next) => StateDeclaration::with_next(
            address,
            implementation,
            input,
            output,
            None,
            ExecutionMode::Pure,
            next,
        )
        .expect("state"),
        None => StateDeclaration::new(
            address,
            implementation,
            input,
            output,
            None,
            ExecutionMode::Pure,
            terminal,
        )
        .expect("state"),
    };
    Declaration::State(Box::new(state))
}

fn admission(scope: StoreScopeId, tenant: TenantScopeId, run: RunId) -> RunFrame {
    let context_ref = context_value_ref();
    let entry = StableId::new("mfm.test-entry-1").expect("entry");
    let document =
        ProgramDocument::new(entry.clone(), content(2), content(2), Vec::new()).expect("document");
    let context = ValueRef::new(content(2), context_ref.clone());
    RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("append-request-0123456789abcdef").expect("append"),
        RunRecord::RunAdmitted(
            RunAdmitted::new(
                scope,
                StoreEpoch::new(1),
                run,
                tenant,
                entry,
                document.program_ref().expect("program ref"),
                context.clone(),
                configuration(4),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![value_object(&context, "null"), program_object(&document)],
    )
    .expect("frame")
}

#[test]
fn fact_stream_identity_binds_scope_epoch_and_tenant() {
    let (scope, tenant, _) = ids();
    let base = fact_stream_ref(&scope, StoreEpoch::new(1), &tenant).expect("base stream");
    assert_ne!(
        base,
        fact_stream_ref(&scope, StoreEpoch::new(2), &tenant).expect("epoch stream")
    );
    assert_ne!(
        base,
        fact_stream_ref(
            &StoreScopeId::new("mfm.store_scope.v1:1123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            &tenant,
        )
        .expect("scope stream")
    );
    assert_ne!(
        base,
        fact_stream_ref(
            &scope,
            StoreEpoch::new(1),
            &TenantScopeId::new("mfm.tenant_scope.v1:1123456789abcdef0123456789abcdef")
                .expect("tenant"),
        )
        .expect("tenant stream")
    );
}

#[test]
fn duplicate_admission_is_found_and_distinct_head_is_stale() {
    let (scope, tenant, run) = ids();
    let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
    let first = admission(scope.clone(), tenant.clone(), run.clone());
    assert_eq!(
        store.append_admission_fixture(first.clone()),
        Ok(AppendDisposition::NewlyCommitted { sequence: 1 })
    );
    assert_eq!(
        store.append_admission_fixture(first),
        Ok(AppendDisposition::Found { sequence: 1 })
    );
    let stale = {
        let admitted = admission(scope, tenant, run);
        RunFrame::new(
            admitted.run_id().clone(),
            admitted.store_scope_id().clone(),
            admitted.store_epoch(),
            3,
            AppendRequestId::new("append-request-2-0123456789abcdef").expect("append"),
            RunRecord::StateConcluded(StateConcluded::Pure {
                occurrence: SequentialControlAddress::new(0, Vec::new()).expect("occurrence"),
                outcome: StateOutcome::Success(ValueRef::new(content(5), content(6))),
                fact_proposals: None,
                fact_publication: None,
            }),
            Vec::new(),
        )
        .expect("candidate")
    };
    assert_eq!(
        store.append(stale),
        Ok(AppendDisposition::StaleHead { actual_sequence: 1 })
    );
}

#[test]
fn admission_requires_one_exact_canonical_program_object() {
    let (scope, tenant, run) = ids();
    let valid = admission(scope.clone(), tenant.clone(), run.clone());
    assert!(QualifiedRun::validate_prefix(
        scope.clone(),
        StoreEpoch::new(1),
        tenant.clone(),
        vec![valid.clone()],
    )
    .is_ok());

    let record = valid.record().clone();
    let missing = RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("missing-program-admission-012345").expect("request"),
        record.clone(),
        vec![valid.objects()[0].clone()],
    )
    .expect("missing program frame");
    assert_eq!(
        QualifiedRun::validate_prefix(
            scope.clone(),
            StoreEpoch::new(1),
            tenant.clone(),
            vec![missing],
        ),
        Err(StoreError::InvalidHistory)
    );

    let duplicate = RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("duplicate-program-admission-012345").expect("request"),
        record.clone(),
        vec![
            valid.objects()[0].clone(),
            valid.objects()[1].clone(),
            valid.objects()[1].clone(),
        ],
    )
    .expect("duplicate program frame");
    assert_eq!(
        QualifiedRun::validate_prefix(
            scope.clone(),
            StoreEpoch::new(1),
            tenant.clone(),
            vec![duplicate],
        ),
        Err(StoreError::InvalidHistory)
    );

    let substituted_document = ProgramDocument::new(
        StableId::new("mfm.test-entry-1").expect("entry"),
        content(3),
        content(3),
        Vec::new(),
    )
    .expect("substituted document");
    let substituted = RunFrame::new(
        run,
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("substituted-program-admission-012345").expect("request"),
        record,
        vec![
            valid.objects()[0].clone(),
            program_object(&substituted_document),
        ],
    )
    .expect("substituted program frame");
    assert_eq!(
        QualifiedRun::validate_prefix(scope, StoreEpoch::new(1), tenant, vec![substituted]),
        Err(StoreError::InvalidHistory)
    );
}

#[test]
fn pure_conclusion_has_no_preparation_path() {
    let (scope, tenant, run) = ids();
    let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
    let occurrence = SequentialControlAddress::new(0, Vec::new()).expect("occurrence");
    let contract = content(2);
    let document = ProgramDocument::new(
        StableId::new("mfm.test-entry-1").expect("entry"),
        contract.clone(),
        contract.clone(),
        vec![pure_state(
            0,
            contract.clone(),
            contract.clone(),
            true,
            None,
        )],
    )
    .expect("document");
    let context_ref = context_value_ref();
    let admitted = RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("append-pure-admission").expect("append"),
        RunRecord::RunAdmitted(
            RunAdmitted::new(
                scope.clone(),
                StoreEpoch::new(1),
                run.clone(),
                tenant.clone(),
                StableId::new("mfm.test-entry-1").expect("entry"),
                document.program_ref().expect("program"),
                ValueRef::new(content(2), context_ref.clone()),
                configuration(4),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![
            value_object(&ValueRef::new(content(2), context_ref.clone()), "null"),
            program_object(&document),
        ],
    )
    .expect("frame");
    store.append_admission_fixture(admitted).expect("admit");
    let conclusion = StateConcluded::Pure {
        occurrence,
        outcome: StateOutcome::Success(ValueRef::new(contract, context_value_ref())),
        fact_proposals: None,
        fact_publication: None,
    };
    let owner = store
        .prepare_conclusion_fixture(
            &run,
            &document,
            1,
            AppendRequestId::new("append-conclusion-0123456789ab").expect("append"),
            conclusion,
            Vec::new(),
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("prepare");
    assert_eq!(
        owner.commit(&store),
        Ok(AppendDisposition::NewlyCommitted { sequence: 2 })
    );
}

#[test]
fn direct_new_preparation_matches_qualified_record_ordinal() {
    let (scope, tenant, run) = ids();
    let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
    let input_contract = content(70);
    let output_contract = content(71);
    let capability_contract = content(72);
    let state_implementation = content(73);
    let adapter_implementation = content(74);
    let physical_target = content(75);
    let binding = BindingDescriptor::new(
        state_implementation.clone(),
        Some(capability_contract.clone()),
        Some(adapter_implementation),
        physical_target,
        None,
        None,
    )
    .expect("binding");
    let execution_binding_ref = binding.content_ref().expect("binding ref");
    let state = StateDeclaration::new(
        SequentialControlAddress::new(0, Vec::new()).expect("address"),
        state_implementation,
        input_contract.clone(),
        output_contract,
        None,
        ExecutionMode::Read {
            capability_contract_ref: capability_contract,
            total_attempt_bound: 1,
            fact_selection_required: false,
        },
        true,
    )
    .expect("state")
    .with_execution_binding(binding.clone())
    .expect("execution binding");
    let document = ProgramDocument::new(
        StableId::new("mfm.test-access").expect("entry"),
        state.output_contract_ref().clone(),
        input_contract.clone(),
        vec![Declaration::State(Box::new(state))],
    )
    .expect("document");
    let input = value_ref(&input_contract, "null");
    let intent_contract = content(76);
    let intent = value_ref(&intent_contract, "null");
    let admission = RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("append-access-admission").expect("append"),
        RunRecord::RunAdmitted(
            RunAdmitted::new(
                scope.clone(),
                StoreEpoch::new(1),
                run.clone(),
                tenant.clone(),
                StableId::new("mfm.test-access").expect("entry"),
                document.program_ref().expect("program"),
                input.clone(),
                configuration(77),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![value_object(&input, "null"), program_object(&document)],
    )
    .expect("admission frame");
    store.append_admission_fixture(admission).expect("admit");

    let prepared = StatePrepared::new(
        SequentialControlAddress::new(0, Vec::new()).expect("address"),
        0,
        input,
        intent,
        None,
        None,
        PreparationMode::Read {
            total_attempt_bound: 1,
        },
        execution_binding_ref,
        None,
        mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
    )
    .expect("prepared");
    let append = store
        .prepare_access_fixture(
            &run,
            &document,
            1,
            AppendRequestId::new("append-access-preparation").expect("append"),
            prepared,
            vec![value_object(&value_ref(&intent_contract, "null"), "null")],
        )
        .expect("prepare");
    let assigned = append.preparation().cloned().expect("assigned preparation");
    let retained = store.load(&run).expect("retained");
    let (_, selected) = retained
        .selected_preparation(&SequentialControlAddress::new(0, Vec::new()).expect("address"))
        .expect("selected preparation");
    assert_eq!(assigned, selected);
    assert_eq!(assigned.record_ordinal(), 1);
}

#[test]
fn unresolved_effect_rejects_a_second_preparation_without_changing_reservation() {
    let (scope, tenant, run) = ids();
    let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
    let input_contract = content(80);
    let output_contract = content(81);
    let capability_contract = content(82);
    let state_implementation = content(83);
    let adapter_implementation = content(84);
    let physical_target = content(85);
    let effect_domain = StableId::new("mfm.test.effect").expect("effect domain");
    let binding = BindingDescriptor::new(
        state_implementation.clone(),
        Some(capability_contract.clone()),
        Some(adapter_implementation),
        physical_target,
        Some(effect_domain.clone()),
        None,
    )
    .expect("binding");
    let execution_binding_ref = binding.content_ref().expect("binding ref");
    let state = StateDeclaration::new(
        SequentialControlAddress::new(0, Vec::new()).expect("address"),
        state_implementation,
        input_contract.clone(),
        output_contract,
        None,
        ExecutionMode::Effect {
            capability_contract_ref: capability_contract,
            effect_domain,
            fact_selection_required: false,
        },
        true,
    )
    .expect("state")
    .with_execution_binding(binding.clone())
    .expect("execution binding")
    .with_maximum_conclusion_bytes(4096)
    .expect("conclusion bound");
    let document = ProgramDocument::new(
        StableId::new("mfm.test-effect").expect("entry"),
        state.output_contract_ref().clone(),
        input_contract.clone(),
        vec![Declaration::State(Box::new(state))],
    )
    .expect("document");
    let input = value_ref(&input_contract, "null");
    let intent_contract = content(86);
    let intent = value_ref(&intent_contract, "null");
    let admission = RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("append-effect-admission").expect("append"),
        RunRecord::RunAdmitted(
            RunAdmitted::new(
                scope,
                StoreEpoch::new(1),
                run.clone(),
                tenant,
                StableId::new("mfm.test-effect").expect("entry"),
                document.program_ref().expect("program"),
                input.clone(),
                configuration(87),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![value_object(&input, "null"), program_object(&document)],
    )
    .expect("admission frame");
    store.append_admission_fixture(admission).expect("admit");

    let maximum_conclusion_bytes = 4096;
    let first = StatePrepared::new(
        SequentialControlAddress::new(0, Vec::new()).expect("address"),
        0,
        input.clone(),
        intent.clone(),
        None,
        None,
        PreparationMode::Effect,
        execution_binding_ref.clone(),
        None,
        maximum_conclusion_bytes,
    )
    .expect("first preparation");
    let first = store
        .prepare_access_fixture(
            &run,
            &document,
            1,
            AppendRequestId::new("append-effect-preparation-1").expect("append"),
            first,
            vec![value_object(&intent, "null")],
        )
        .expect("first append");
    let first_ref = first.preparation().cloned().expect("preparation ref");
    let prepared_run = store.load(&run).expect("prepared run");
    assert_eq!(
        prepared_run
            .reserved_conclusion_bytes()
            .expect("reserved conclusion"),
        maximum_conclusion_bytes
    );

    let second = StatePrepared::new(
        SequentialControlAddress::new(0, Vec::new()).expect("address"),
        1,
        input,
        intent.clone(),
        None,
        None,
        PreparationMode::Effect,
        execution_binding_ref,
        Some(first_ref),
        maximum_conclusion_bytes,
    )
    .expect("wire-valid second preparation");
    assert!(matches!(
        store.prepare_access_fixture(
            &run,
            &document,
            2,
            AppendRequestId::new("append-effect-preparation-2").expect("append"),
            second,
            vec![value_object(&intent, "null")],
        ),
        Err(StoreError::NotActionable)
    ));
    let unchanged = store.load(&run).expect("unchanged run");
    assert_eq!(unchanged.head_sequence(), 2);
    assert_eq!(
        unchanged
            .reserved_conclusion_bytes()
            .expect("reserved conclusion"),
        maximum_conclusion_bytes
    );
}

#[test]
fn reducer_advances_the_exact_direct_successor_context() {
    let (scope, tenant, run) = ids();
    let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
    let input_contract = content(30);
    let middle_contract = content(31);
    let root_value = value_ref(&input_contract, "{\"step\":0}");
    let middle_value = value_ref(&middle_contract, "{\"step\":1}");
    let final_value = value_ref(&middle_contract, "{\"step\":2}");
    let second_address = SequentialControlAddress::new(2, Vec::new()).expect("address");
    let document = ProgramDocument::new(
        StableId::new("mfm.test-sequential").expect("entry"),
        middle_contract.clone(),
        input_contract.clone(),
        vec![
            pure_state(
                1,
                input_contract.clone(),
                middle_contract.clone(),
                false,
                Some(second_address.clone()),
            ),
            pure_state(
                2,
                middle_contract.clone(),
                middle_contract.clone(),
                true,
                None,
            ),
        ],
    )
    .expect("document");
    let admission = RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("append-sequential-admission").expect("append"),
        RunRecord::RunAdmitted(
            RunAdmitted::new(
                scope.clone(),
                StoreEpoch::new(1),
                run.clone(),
                tenant.clone(),
                StableId::new("mfm.test-sequential").expect("entry"),
                document.program_ref().expect("program"),
                root_value.clone(),
                configuration(41),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![
            value_object(&root_value, "{\"step\":0}"),
            program_object(&document),
        ],
    )
    .expect("admission frame");
    store.append_admission_fixture(admission).expect("admit");
    let ready = store.reduce_fixture(&run, document.clone()).expect("ready");
    assert!(matches!(ready.action, RunAction::ReadyPure { .. }));

    let first = RunFrame::new(
        run.clone(),
        scope.clone(),
        StoreEpoch::new(1),
        2,
        AppendRequestId::new("append-sequential-first").expect("append"),
        RunRecord::StateConcluded(StateConcluded::Pure {
            occurrence: SequentialControlAddress::new(1, Vec::new()).expect("address"),
            outcome: StateOutcome::Success(middle_value.clone()),
            fact_proposals: None,
            fact_publication: None,
        }),
        vec![value_object(&middle_value, "{\"step\":1}")],
    )
    .expect("first frame");
    assert_eq!(
        store.append(first.clone()),
        Ok(AppendDisposition::NewlyCommitted { sequence: 2 })
    );
    assert_eq!(
        store.append(first),
        Ok(AppendDisposition::Found { sequence: 2 })
    );
    let next = store.reduce_fixture(&run, document.clone()).expect("next");
    assert_eq!(next.latest_context, middle_value);
    assert!(matches!(next.action, RunAction::ReadyPure { .. }));

    let second = RunFrame::new(
        run.clone(),
        scope,
        StoreEpoch::new(1),
        3,
        AppendRequestId::new("append-sequential-second").expect("append"),
        RunRecord::StateConcluded(StateConcluded::Pure {
            occurrence: second_address,
            outcome: StateOutcome::Success(final_value.clone()),
            fact_proposals: None,
            fact_publication: None,
        }),
        vec![value_object(&final_value, "{\"step\":2}")],
    )
    .expect("second frame");
    store.append(second).expect("second conclusion");
    let terminal = store.reduce_fixture(&run, document).expect("terminal");
    assert_eq!(terminal.latest_context, final_value);
    assert!(matches!(terminal.action, RunAction::Terminal { .. }));
}

#[test]
fn reducer_materializes_one_selected_match_payload() {
    let (scope, tenant, run) = ids();
    let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
    let selector_contract = content(50);
    let payload_contract = content(51);
    let result_contract = content(52);
    let selector_value = value_ref(
        &selector_contract,
        "{\"kind\":\"left\",\"value\":{\"n\":1}}",
    );
    let payload_value = value_ref(&payload_contract, "{\"n\":1}");
    let result_value = value_ref(&result_contract, "{\"result\":2}");
    let state_address = SequentialControlAddress::new(2, Vec::new()).expect("address");
    let selector = mfm_program::MatchDeclaration::new(
        SequentialControlAddress::new(1, Vec::new()).expect("address"),
        selector_contract.clone(),
        vec![mfm_program::MatchVariant::new(
            StableId::new("left").expect("tag"),
            payload_contract.clone(),
            result_contract.clone(),
            state_address.clone(),
        )],
    )
    .expect("selector");
    let document = ProgramDocument::new(
        StableId::new("mfm.test-match").expect("entry"),
        result_contract.clone(),
        selector_contract.clone(),
        vec![
            Declaration::Match(selector),
            pure_state(2, payload_contract.clone(), result_contract, true, None),
        ],
    )
    .expect("document");
    let admission = RunFrame::new(
        run.clone(),
        scope,
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("append-match-admission").expect("append"),
        RunRecord::RunAdmitted(
            RunAdmitted::new(
                store.scope().clone(),
                StoreEpoch::new(1),
                run.clone(),
                tenant,
                StableId::new("mfm.test-match").expect("entry"),
                document.program_ref().expect("program"),
                selector_value.clone(),
                configuration(53),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![
            ImmutableObject::new(
                StableId::new("mfm.value").expect("object type"),
                selector_value.value_ref().clone(),
                "{\"kind\":\"left\",\"value\":{\"n\":1}}".to_owned(),
            )
            .expect("selector object"),
            program_object(&document),
        ],
    )
    .expect("admission frame");
    store.append_admission_fixture(admission).expect("admit");
    let selected = store
        .reduce_fixture(&run, document.clone())
        .expect("selected arm");
    assert_eq!(selected.latest_context, payload_value);
    assert_eq!(selected.latest_context_object.canonical_json(), "{\"n\":1}");
    assert!(matches!(selected.action, RunAction::ReadyPure { .. }));
    let owner = store
        .prepare_conclusion_fixture(
            &run,
            &document,
            1,
            AppendRequestId::new("append-match-conclusion").expect("append"),
            StateConcluded::Pure {
                occurrence: state_address,
                outcome: StateOutcome::Success(result_value.clone()),
                fact_proposals: None,
                fact_publication: None,
            },
            vec![value_object(&result_value, "{\"result\":2}")],
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("match conclusion");
    assert_eq!(owner.frame().objects().len(), 1);
    assert_eq!(
        owner.frame().objects()[0].canonical_json(),
        "{\"result\":2}"
    );
    owner.commit(&store).expect("commit match conclusion");
    let terminal = store.reduce_fixture(&run, document).expect("terminal arm");
    assert!(matches!(terminal.action, RunAction::Terminal { .. }));
}

#[test]
fn reducer_reuses_a_retained_match_payload_object() {
    let (scope, tenant, run) = ids();
    let store = SemanticStore::memory(scope.clone(), StoreEpoch::new(1), tenant.clone());
    let payload_contract = content(60);
    let selector_contract = content(61);
    let result_contract = content(62);
    let payload_value = value_ref(&payload_contract, "{\"n\":1}");
    let selector_value = value_ref(
        &selector_contract,
        "{\"kind\":\"left\",\"value\":{\"n\":1}}",
    );
    let result_value = value_ref(&result_contract, "{\"result\":2}");
    let selector_address = SequentialControlAddress::new(1, Vec::new()).expect("address");
    let result_address = SequentialControlAddress::new(2, Vec::new()).expect("address");
    let selector = mfm_program::MatchDeclaration::new(
        selector_address.clone(),
        selector_contract.clone(),
        vec![mfm_program::MatchVariant::new(
            StableId::new("left").expect("tag"),
            payload_contract.clone(),
            result_contract.clone(),
            result_address.clone(),
        )],
    )
    .expect("selector");
    let document = ProgramDocument::new(
        StableId::new("mfm.test-retained-match").expect("entry"),
        result_contract.clone(),
        payload_contract.clone(),
        vec![
            pure_state(
                0,
                payload_contract.clone(),
                selector_contract.clone(),
                false,
                Some(selector_address),
            ),
            Declaration::Match(selector),
            pure_state(2, payload_contract.clone(), result_contract, true, None),
        ],
    )
    .expect("document");
    let admission = RunFrame::new(
        run.clone(),
        scope,
        StoreEpoch::new(1),
        1,
        AppendRequestId::new("append-retained-match-admission").expect("append"),
        RunRecord::RunAdmitted(
            RunAdmitted::new(
                store.scope().clone(),
                StoreEpoch::new(1),
                run.clone(),
                tenant,
                StableId::new("mfm.test-retained-match").expect("entry"),
                document.program_ref().expect("program"),
                payload_value.clone(),
                configuration(63),
                Vec::new(),
            )
            .expect("admission"),
        ),
        vec![
            value_object(&payload_value, "{\"n\":1}"),
            program_object(&document),
        ],
    )
    .expect("admission frame");
    store.append_admission_fixture(admission).expect("admit");
    let selector_owner = store
        .prepare_conclusion_fixture(
            &run,
            &document,
            1,
            AppendRequestId::new("append-retained-match-selector").expect("append"),
            StateConcluded::Pure {
                occurrence: SequentialControlAddress::new(0, Vec::new()).expect("address"),
                outcome: StateOutcome::Success(selector_value.clone()),
                fact_proposals: None,
                fact_publication: None,
            },
            vec![value_object(
                &selector_value,
                "{\"kind\":\"left\",\"value\":{\"n\":1}}",
            )],
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("selector conclusion");
    selector_owner.commit(&store).expect("commit selector");
    let selected = store
        .reduce_fixture(&run, document.clone())
        .expect("selected retained payload");
    assert_eq!(selected.latest_context, payload_value);
    assert_eq!(
        selected.latest_context_object.object_type().as_str(),
        "mfm.value"
    );

    let result_owner = store
        .prepare_conclusion_fixture(
            &run,
            &document,
            2,
            AppendRequestId::new("append-retained-match-result").expect("append"),
            StateConcluded::Pure {
                occurrence: result_address,
                outcome: StateOutcome::Success(result_value.clone()),
                fact_proposals: None,
                fact_publication: None,
            },
            vec![value_object(&result_value, "{\"result\":2}")],
            mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
        )
        .expect("result conclusion");
    result_owner.commit(&store).expect("commit result");
    assert!(matches!(
        store
            .reduce_fixture(&run, document)
            .expect("terminal")
            .action,
        RunAction::Terminal { .. }
    ));
}
