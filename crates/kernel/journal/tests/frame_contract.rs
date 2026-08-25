use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, EffectId, RunId, SchemaId};
use mfm_journal::{
    frame_head_digest, EncodedRunFrame, JournalError, JournalHistory, JournalRecord, OutcomeKind,
    StoredRunBytes,
};
use mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES;
use serde_json::{json, Value};

fn run(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn object_ref(schema_name: &str, bytes: &[u8]) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            schema_name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema"),
        raw_content_digest(bytes),
    )
    .expect("ref")
}

fn canonical(value: &Value) -> Vec<u8> {
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(value).expect("json"),
    )
    .expect("canonical")
    .to_vec()
}

fn qualify_one(run: &RunId, bytes: Vec<u8>) -> Result<JournalHistory, JournalError> {
    JournalHistory::qualify(run, StoredRunBytes::new(vec![bytes]).expect("stored"))
}

fn object_wire(reference: &ContentRef, bytes: &[u8]) -> Value {
    json!({
        "canonical": serde_json::from_slice::<Value>(bytes).expect("object json"),
        "content_ref": reference,
    })
}

fn sorted_objects(mut objects: Vec<Value>) -> Vec<Value> {
    objects.sort_by(|left, right| {
        left["content_ref"]["schema_id"]
            .as_str()
            .expect("left schema")
            .cmp(
                right["content_ref"]["schema_id"]
                    .as_str()
                    .expect("right schema"),
            )
            .then_with(|| {
                left["content_ref"]["content_digest"]
                    .as_str()
                    .expect("left digest")
                    .cmp(
                        right["content_ref"]["content_digest"]
                            .as_str()
                            .expect("right digest"),
                    )
            })
    });
    objects
}

fn retarget_frame(bytes: &[u8], run_sequence: u64, previous_head: &ContentDigest) -> Vec<u8> {
    let mut wire: Value = serde_json::from_slice(bytes).expect("frame wire");
    wire["run_sequence"] = json!(run_sequence);
    wire["previous_head_digest"] = json!(previous_head);
    canonical(&wire)
}

#[test]
fn effect_sequence_has_exact_v2_wires_and_recursive_sha256_v1_heads() {
    let program = br#"{"program":1}"#;
    let context = br#"{"context":2}"#;
    let run = run(1);
    let program_ref = object_ref("mfm.test.program", program);
    let context_ref = object_ref("mfm.test.context", context);
    let genesis = EncodedRunFrame::admission(&run, &program_ref, program, &context_ref, context)
        .expect("genesis");
    let expected = json!({
        "domain": "mfm.run.frame.v2",
        "objects": sorted_objects(vec![
            object_wire(&program_ref, program),
            object_wire(&context_ref, context),
        ]),
        "previous_head_digest": null,
        "record": {
            "admitted_context": context_ref,
            "kind": "run_admitted",
            "program_ref": program_ref,
        },
        "run_id": run,
        "run_sequence": 1,
    });
    assert_eq!(genesis.canonical_bytes(), canonical(&expected));
    assert_eq!(genesis.run_sequence(), 1);
    assert_eq!(genesis.previous_head_digest(), None);
    assert_eq!(
        genesis.head_digest(),
        &frame_head_digest(genesis.canonical_bytes())
    );
    assert_eq!(genesis.head_digest().algorithm(), DigestAlgorithm::Sha256V1);
    assert_eq!(
        genesis.head_digest().as_str(),
        "content:sha256-v1:a9da9f2cb35cb5142cfdb2457c317235da9a36d009d89b6d70eb7f8a85f541af"
    );

    let genesis_bytes = genesis.canonical_bytes().to_vec();
    let mut history = JournalHistory::from_genesis(genesis).expect("history");
    let command = br#"{"command":3}"#;
    let command_ref = object_ref("mfm.test.command", command);
    let effect_id = EffectId::from_digest(DigestBytes::from_array([9; 32]));
    let prepared = history
        .encode_effect_prepare(&effect_id, &command_ref, command)
        .expect("prepare");
    let expected = json!({
        "domain": "mfm.run.frame.v2",
        "objects": [object_wire(&command_ref, command)],
        "previous_head_digest": history.head_digest(),
        "record": {
            "command": command_ref,
            "effect_id": effect_id,
            "kind": "state_effect_prepared",
        },
        "run_id": run,
        "run_sequence": 2,
    });
    assert_eq!(prepared.canonical_bytes(), canonical(&expected));
    assert_eq!(prepared.run_sequence(), 2);
    assert_eq!(prepared.previous_head_digest(), Some(history.head_digest()));
    assert_eq!(
        prepared.head_digest().as_str(),
        "content:sha256-v1:a1cd763092a16c87247a05dd58bae14a84abcf93e91682ded4e34bb372ca3c6c"
    );
    let prepared_bytes = prepared.canonical_bytes().to_vec();
    let record = history.extend_inserted(prepared).expect("extend prepare");
    assert!(matches!(
        record,
        JournalRecord::StateEffectPrepared {
            effect_id: retained,
            ..
        } if retained == &effect_id
    ));

    let evidence = br#"{"evidence":2}"#;
    let outcome = br#"{"success":true}"#;
    let evidence_ref = object_ref("mfm.test.evidence", evidence);
    let outcome_ref = object_ref("mfm.test.outcome", outcome);
    let concluded = history
        .encode_effect_conclusion(
            &evidence_ref,
            evidence,
            OutcomeKind::Success,
            &outcome_ref,
            outcome,
        )
        .expect("conclusion");
    let expected = json!({
        "domain": "mfm.run.frame.v2",
        "objects": sorted_objects(vec![
            object_wire(&evidence_ref, evidence),
            object_wire(&outcome_ref, outcome),
        ]),
        "previous_head_digest": history.head_digest(),
        "record": {
            "evidence": evidence_ref,
            "kind": "state_effect_concluded",
            "outcome": {"kind": "success", "value": outcome_ref},
        },
        "run_id": run,
        "run_sequence": 3,
    });
    assert_eq!(concluded.canonical_bytes(), canonical(&expected));
    assert_eq!(
        concluded.head_digest().as_str(),
        "content:sha256-v1:3865f66a9df1316ffad4aeb94aa2d02d5a0c6b3cb588cc343404b70b07231910"
    );
    let concluded_bytes = concluded.canonical_bytes().to_vec();
    let record = history
        .extend_inserted(concluded)
        .expect("extend conclusion");
    assert!(matches!(
        record,
        JournalRecord::StateEffectConcluded {
            kind: OutcomeKind::Success,
            ..
        }
    ));

    let cold = JournalHistory::qualify(
        &run,
        StoredRunBytes::new(vec![genesis_bytes, prepared_bytes, concluded_bytes]).expect("stored"),
    )
    .expect("cold sequence");
    assert_eq!(cold.head_sequence(), 3);
}

#[test]
fn fused_read_closure_coalesces_identical_objects() {
    let program = b"{}";
    let context = b"[]";
    let genesis = EncodedRunFrame::admission(
        &run(2),
        &object_ref("mfm.test.program", program),
        program,
        &object_ref("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    let genesis_bytes = genesis.canonical_bytes().to_vec();
    let history = JournalHistory::from_genesis(genesis).expect("history");
    let value = br#"{"same":1}"#;
    let reference = object_ref("mfm.test.same", value);
    let frame = history
        .encode_read_conclusion(
            &reference,
            value,
            &reference,
            value,
            OutcomeKind::Failure,
            &reference,
            value,
        )
        .expect("read frame");
    let bytes = frame.canonical_bytes();
    assert_eq!(
        bytes
            .windows(reference.content_digest().as_str().len())
            .filter(|window| *window == reference.content_digest().as_str().as_bytes())
            .count(),
        4
    );
    JournalHistory::qualify(
        &run(2),
        StoredRunBytes::new(vec![genesis_bytes, bytes.to_vec()]).expect("stored read"),
    )
    .expect("cold failed Read");
}

#[test]
fn repeated_cross_frame_object_is_valid_hot_and_cold() {
    let run = run(15);
    let shared = br#"{"shared":true}"#;
    let context = b"[]";
    let shared_ref = object_ref("mfm.test.shared", shared);
    let genesis = EncodedRunFrame::admission(
        &run,
        &shared_ref,
        shared,
        &object_ref("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    let genesis_bytes = genesis.canonical_bytes().to_vec();
    let mut history = JournalHistory::from_genesis(genesis).expect("history");
    let successor = history
        .encode_pure_conclusion(OutcomeKind::Success, &shared_ref, shared)
        .expect("repeated object successor");
    let successor_bytes = successor.canonical_bytes().to_vec();

    let JournalRecord::StateConcludedPure { outcome, .. } =
        history.extend_inserted(successor).expect("hot extension")
    else {
        panic!("successor record");
    };
    assert_eq!(outcome.content_ref(), &shared_ref);
    assert_eq!(outcome.canonical_bytes(), shared);

    let cold = JournalHistory::qualify(
        &run,
        StoredRunBytes::new(vec![genesis_bytes, successor_bytes]).expect("stored prefix"),
    )
    .expect("cold qualification");
    let JournalRecord::StateConcludedPure { outcome, .. } =
        cold.records().nth(1).expect("cold successor")
    else {
        panic!("cold successor record");
    };
    assert_eq!(outcome.content_ref(), &shared_ref);
    assert_eq!(outcome.canonical_bytes(), shared);
}

#[test]
fn effect_records_require_one_adjacent_prepare_conclusion_pair() {
    let run = run(21);
    let program = b"{}";
    let context = b"[]";
    let genesis = EncodedRunFrame::admission(
        &run,
        &object_ref("mfm.test.program", program),
        program,
        &object_ref("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    let mut history = JournalHistory::from_genesis(genesis).expect("history");
    let evidence = b"true";
    let evidence_ref = object_ref("mfm.test.evidence", evidence);
    assert_eq!(
        history
            .encode_effect_conclusion(
                &evidence_ref,
                evidence,
                OutcomeKind::Success,
                &evidence_ref,
                evidence,
            )
            .expect_err("conclusion without prepare"),
        JournalError::InvalidFrame
    );

    let command = br#"{"command":1}"#;
    let command_ref = object_ref("mfm.test.command", command);
    let prepared = history
        .encode_effect_prepare(
            &EffectId::from_digest(DigestBytes::from_array([4; 32])),
            &command_ref,
            command,
        )
        .expect("prepare");
    history.extend_inserted(prepared).expect("pending");
    let invalid_successors = [
        (
            "pure conclusion",
            history.encode_pure_conclusion(OutcomeKind::Success, &evidence_ref, evidence),
        ),
        (
            "second prepare",
            history.encode_effect_prepare(
                &EffectId::from_digest(DigestBytes::from_array([5; 32])),
                &command_ref,
                command,
            ),
        ),
    ];
    for (case, candidate) in invalid_successors {
        assert_eq!(
            candidate.expect_err(case),
            JournalError::InvalidFrame,
            "{case}"
        );
    }
}

#[test]
fn cold_effect_prefixes_enforce_exact_pairing_and_checked_identity() {
    let run = run(22);
    let program = b"{}";
    let context = b"[]";
    let program_ref = object_ref("mfm.test.program", program);
    let context_ref = object_ref("mfm.test.context", context);
    let genesis = EncodedRunFrame::admission(&run, &program_ref, program, &context_ref, context)
        .expect("genesis");
    let genesis_bytes = genesis.canonical_bytes().to_vec();
    let genesis_head = genesis.head_digest().clone();
    let mut pending = JournalHistory::from_genesis(genesis).expect("history");
    let command = br#"{"command":1}"#;
    let evidence = br#"{"accepted":true}"#;
    let outcome = br#"{"value":1}"#;
    let command_ref = object_ref("mfm.test.command", command);
    let evidence_ref = object_ref("mfm.test.evidence", evidence);
    let outcome_ref = object_ref("mfm.test.outcome", outcome);
    let effect_id = EffectId::from_digest(DigestBytes::from_array([6; 32]));
    let pure_bytes = pending
        .encode_pure_conclusion(OutcomeKind::Success, &outcome_ref, outcome)
        .expect("pure")
        .canonical_bytes()
        .to_vec();
    let read_bytes = pending
        .encode_read_conclusion(
            &command_ref,
            command,
            &evidence_ref,
            evidence,
            OutcomeKind::Success,
            &outcome_ref,
            outcome,
        )
        .expect("read")
        .canonical_bytes()
        .to_vec();
    let prepared = pending
        .encode_effect_prepare(&effect_id, &command_ref, command)
        .expect("prepare");
    let prepared_bytes = prepared.canonical_bytes().to_vec();
    let prepared_head = prepared.head_digest().clone();
    pending.extend_inserted(prepared).expect("pending");

    JournalHistory::qualify(
        &run,
        StoredRunBytes::new(vec![genesis_bytes.clone(), prepared_bytes.clone()]).expect("stored"),
    )
    .expect("valid pending suffix");

    let concluded = pending
        .encode_effect_conclusion(
            &evidence_ref,
            evidence,
            OutcomeKind::Success,
            &outcome_ref,
            outcome,
        )
        .expect("conclusion");
    let concluded_bytes = concluded.canonical_bytes().to_vec();
    let concluded_head = concluded.head_digest().clone();
    JournalHistory::qualify(
        &run,
        StoredRunBytes::new(vec![
            genesis_bytes.clone(),
            prepared_bytes.clone(),
            concluded_bytes.clone(),
        ])
        .expect("stored"),
    )
    .expect("valid concluded pair");

    let mut malformed_id: Value = serde_json::from_slice(&prepared_bytes).expect("prepared wire");
    malformed_id["record"]["effect_id"] = json!(effect_id.as_str().replace("effect:", "run:"));
    let invalid_histories = vec![
        (
            "malformed effect identity",
            vec![genesis_bytes.clone(), canonical(&malformed_id)],
        ),
        (
            "orphan conclusion",
            vec![
                genesis_bytes.clone(),
                retarget_frame(&concluded_bytes, 2, &genesis_head),
            ],
        ),
        (
            "duplicate prepare",
            vec![
                genesis_bytes.clone(),
                prepared_bytes.clone(),
                retarget_frame(&prepared_bytes, 3, &prepared_head),
            ],
        ),
        (
            "intervening pure",
            vec![
                genesis_bytes.clone(),
                prepared_bytes.clone(),
                retarget_frame(&pure_bytes, 3, &prepared_head),
            ],
        ),
        (
            "intervening read",
            vec![
                genesis_bytes.clone(),
                prepared_bytes.clone(),
                retarget_frame(&read_bytes, 3, &prepared_head),
            ],
        ),
        (
            "duplicate conclusion",
            vec![
                genesis_bytes,
                prepared_bytes,
                concluded_bytes.clone(),
                retarget_frame(&concluded_bytes, 4, &concluded_head),
            ],
        ),
    ];
    for (case, frames) in invalid_histories {
        assert!(
            matches!(
                JournalHistory::qualify(
                    &run,
                    StoredRunBytes::new(frames).expect("stored invalid prefix"),
                ),
                Err(JournalError::InvalidHistory)
            ),
            "{case}"
        );
    }
}

#[test]
fn strict_wire_and_frame_local_closure_reject_hostile_inputs() {
    let run = run(12);
    let program = b"{}";
    let context = b"[]";
    let genesis = EncodedRunFrame::admission(
        &run,
        &object_ref("mfm.test.program", program),
        program,
        &object_ref("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    let original: Value = serde_json::from_slice(genesis.canonical_bytes()).expect("wire");

    let mut mutations = Vec::new();
    let mut v1_domain = original.clone();
    v1_domain["domain"] = json!("mfm.run.frame.v1");
    mutations.push(canonical(&v1_domain));
    let mut wrong_domain = original.clone();
    wrong_domain["domain"] = json!("mfm.run.frame.v0");
    mutations.push(canonical(&wrong_domain));
    let mut unknown = original.clone();
    unknown["unknown"] = json!(true);
    mutations.push(canonical(&unknown));
    let mut wrong_tag = original.clone();
    wrong_tag["record"]["kind"] = json!("admitted");
    mutations.push(canonical(&wrong_tag));
    let mut wrong_run_algorithm = original.clone();
    wrong_run_algorithm["run_id"] = json!(original["run_id"]
        .as_str()
        .expect("run id")
        .replace("sha256-jcs-v1", "sha256-v1"));
    mutations.push(canonical(&wrong_run_algorithm));
    for retired_tag in ["state_prepared", "state_concluded_access"] {
        let mut retired = original.clone();
        retired["record"]["kind"] = json!(retired_tag);
        mutations.push(canonical(&retired));
    }
    for retired_field in ["occurrence", "preparation_sequence", "entry_point"] {
        let mut retired = original.clone();
        retired["record"][retired_field] = json!({});
        mutations.push(canonical(&retired));
    }
    let mut missing = original.clone();
    missing["objects"].as_array_mut().expect("objects").pop();
    mutations.push(canonical(&missing));
    let mut extra = original.clone();
    let objects = extra["objects"].as_array_mut().expect("objects");
    objects.push(object_wire(&object_ref("mfm.test.extra", b"null"), b"null"));
    *objects = sorted_objects(std::mem::take(objects));
    mutations.push(canonical(&extra));
    let mut duplicate = original.clone();
    let first = duplicate["objects"][0].clone();
    duplicate["objects"]
        .as_array_mut()
        .expect("objects")
        .insert(1, first);
    mutations.push(canonical(&duplicate));
    let mut reversed = original.clone();
    reversed["objects"]
        .as_array_mut()
        .expect("objects")
        .reverse();
    mutations.push(canonical(&reversed));
    let mut wrong_digest = original.clone();
    wrong_digest["objects"][0]["content_ref"]["content_digest"] =
        json!("content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000");
    mutations.push(canonical(&wrong_digest));
    let mut wrong_schema = original.clone();
    let schema = wrong_schema["objects"][0]["content_ref"]["schema_id"]
        .as_str()
        .expect("schema")
        .replace("sha256-jcs-v1", "sha256-v1");
    wrong_schema["objects"][0]["content_ref"]["schema_id"] = json!(schema);
    mutations.push(canonical(&wrong_schema));
    let mut float_object = original.clone();
    float_object["objects"][0]["canonical"] = json!(1.5);
    mutations.push(serde_json::to_vec(&float_object).expect("float json"));

    let text = std::str::from_utf8(genesis.canonical_bytes()).expect("utf8");
    mutations.push(
        text.replacen(
            "{\"domain\":",
            "{\"domain\":\"mfm.run.frame.v2\",\"domain\":",
            1,
        )
        .into_bytes(),
    );
    let mut noncanonical = genesis.canonical_bytes().to_vec();
    noncanonical.push(b' ');
    mutations.push(noncanonical);
    mutations.push(vec![0xff]);

    for hostile in mutations {
        assert!(matches!(
            qualify_one(&run, hostile),
            Err(JournalError::InvalidHistory)
        ));
    }
}

#[test]
fn history_requires_exact_sequence_predecessor_and_recursive_head() {
    let run = run(13);
    let program = b"{}";
    let context = b"[]";
    let genesis = EncodedRunFrame::admission(
        &run,
        &object_ref("mfm.test.program", program),
        program,
        &object_ref("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    let genesis_bytes = genesis.canonical_bytes().to_vec();
    let history = JournalHistory::from_genesis(genesis).expect("history");
    let outcome = b"true";
    let second = history
        .encode_pure_conclusion(
            OutcomeKind::Success,
            &object_ref("mfm.test.outcome", outcome),
            outcome,
        )
        .expect("second");
    let second_wire: Value = serde_json::from_slice(second.canonical_bytes()).expect("wire");
    let mut wrong_genesis_kind = second_wire.clone();
    wrong_genesis_kind["run_sequence"] = json!(1);
    wrong_genesis_kind["previous_head_digest"] = Value::Null;
    assert!(matches!(
        qualify_one(&run, canonical(&wrong_genesis_kind)),
        Err(JournalError::InvalidHistory)
    ));
    for field in ["run_sequence", "previous_head_digest"] {
        let mut hostile = second_wire.clone();
        hostile[field] = if field == "run_sequence" {
            json!(3)
        } else {
            json!("content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000")
        };
        assert!(matches!(
            JournalHistory::qualify(
                &run,
                StoredRunBytes::new(vec![genesis_bytes.clone(), canonical(&hostile)])
                    .expect("stored"),
            ),
            Err(JournalError::InvalidHistory)
        ));
    }
    let mut wrong_algorithm = second_wire.clone();
    let predecessor = wrong_algorithm["previous_head_digest"]
        .as_str()
        .expect("predecessor")
        .replace("sha256-v1", "sha256-jcs-v1");
    wrong_algorithm["previous_head_digest"] = json!(predecessor);
    assert!(matches!(
        JournalHistory::qualify(
            &run,
            StoredRunBytes::new(vec![genesis_bytes.clone(), canonical(&wrong_algorithm)])
                .expect("stored"),
        ),
        Err(JournalError::InvalidHistory)
    ));
    assert!(matches!(
        JournalHistory::qualify(
            &run,
            StoredRunBytes::new(vec![genesis_bytes.clone(), genesis_bytes]).expect("stored"),
        ),
        Err(JournalError::InvalidHistory)
    ));
}

#[test]
fn qualification_rejects_a_history_belonging_to_another_run() {
    let program = b"{}";
    let context = b"[]";
    let genesis = EncodedRunFrame::admission(
        &run(3),
        &object_ref("mfm.test.program", program),
        program,
        &object_ref("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    let bytes = genesis.canonical_bytes().to_vec();
    assert!(matches!(
        JournalHistory::qualify(&run(4), StoredRunBytes::new(vec![bytes]).expect("stored")),
        Err(JournalError::InvalidHistory)
    ));
}

#[test]
fn opaque_transfer_must_contain_a_complete_prefix() {
    assert!(matches!(
        StoredRunBytes::new(Vec::new()),
        Err(JournalError::InvalidHistory)
    ));
}

#[test]
fn object_one_byte_over_the_exact_limit_is_capacity() {
    let run = run(14);
    let genesis = EncodedRunFrame::admission(
        &run,
        &object_ref("mfm.test.program", b"{}"),
        b"{}",
        &object_ref("mfm.test.context", b"[]"),
        b"[]",
    )
    .expect("genesis");
    let history = JournalHistory::from_genesis(genesis).expect("history");
    let oversized = format!("\"{}\"", "a".repeat(MAX_RUN_OBJECT_CANONICAL_BYTES - 1));
    assert_eq!(oversized.len(), MAX_RUN_OBJECT_CANONICAL_BYTES + 1);
    assert!(matches!(
        history.encode_pure_conclusion(
            OutcomeKind::Success,
            &object_ref("mfm.test.oversized", oversized.as_bytes()),
            oversized.as_bytes(),
        ),
        Err(JournalError::Capacity)
    ));
}
