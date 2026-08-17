use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId};
use mfm_journal::{
    frame_head_digest, EncodedRunFrame, JournalError, JournalHistory, JournalRecord, OutcomeKind,
    StoredRunBytes, MAX_FRAME_BYTES, MAX_FRAME_NON_PAYLOAD_ENVELOPE, MAX_RUN_BYTES, MAX_RUN_FRAMES,
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

#[test]
fn admission_and_successor_heads_are_exact_byte_sha256_v1() {
    let program = br#"{"program":1}"#;
    let context = br#"{"context":2}"#;
    let run = run(1);
    let genesis = EncodedRunFrame::admission(
        &run,
        &object_ref("mfm.test.program", program),
        program,
        &object_ref("mfm.test.context", context),
        context,
    )
    .expect("genesis");
    assert_eq!(genesis.run_sequence(), 1);
    assert_eq!(genesis.previous_head_digest(), None);
    assert_eq!(
        genesis.head_digest(),
        &frame_head_digest(genesis.canonical_bytes())
    );
    assert_eq!(genesis.head_digest().algorithm(), DigestAlgorithm::Sha256V1);
    assert_eq!(
        genesis.head_digest().as_str(),
        "content:sha256-v1:52c650158d704d1867a255406e71b35122d6483102294e4932525802fb7d5f76"
    );

    let mut history = JournalHistory::from_genesis(genesis).expect("history");
    let output = br#"{"ok":true}"#;
    let successor = history
        .encode_pure_conclusion(
            OutcomeKind::Success,
            &object_ref("mfm.test.output", output),
            output,
        )
        .expect("successor");
    assert_eq!(successor.run_sequence(), 2);
    assert_eq!(
        successor.previous_head_digest(),
        Some(history.head_digest())
    );
    assert_eq!(
        successor.head_digest().as_str(),
        "content:sha256-v1:785ef527379ee35f029607112f43114c54c3c421838797ca8f9f838d3e23704c"
    );
    let record = history.extend_inserted(successor).expect("extend");
    assert!(matches!(
        record,
        JournalRecord::StateConcludedPure {
            kind: OutcomeKind::Success,
            ..
        }
    ));
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
fn every_record_and_outcome_has_the_exact_frozen_wire() {
    let run = run(11);
    let program = br#"{"program":1}"#;
    let context = br#"{"context":2}"#;
    let program_ref = object_ref("mfm.test.program", program);
    let context_ref = object_ref("mfm.test.context", context);
    let genesis = EncodedRunFrame::admission(&run, &program_ref, program, &context_ref, context)
        .expect("genesis");
    let expected = json!({
        "domain": "mfm.run.frame.v1",
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

    for kind in [OutcomeKind::Success, OutcomeKind::Failure] {
        let history = JournalHistory::from_genesis(
            EncodedRunFrame::admission(&run, &program_ref, program, &context_ref, context)
                .expect("genesis"),
        )
        .expect("history");
        let outcome = match kind {
            OutcomeKind::Success => br#"{"success":true}"#.as_slice(),
            OutcomeKind::Failure => br#"{"failure":true}"#.as_slice(),
        };
        let outcome_ref = object_ref("mfm.test.outcome", outcome);
        let pure = history
            .encode_pure_conclusion(kind, &outcome_ref, outcome)
            .expect("pure");
        let outcome_kind = match kind {
            OutcomeKind::Success => "success",
            OutcomeKind::Failure => "failure",
        };
        let expected = json!({
            "domain": "mfm.run.frame.v1",
            "objects": [object_wire(&outcome_ref, outcome)],
            "previous_head_digest": history.head_digest(),
            "record": {
                "kind": "state_concluded_pure",
                "outcome": {"kind": outcome_kind, "value": outcome_ref},
            },
            "run_id": run,
            "run_sequence": 2,
        });
        assert_eq!(pure.canonical_bytes(), canonical(&expected));

        let intent = br#"{"intent":1}"#;
        let evidence = br#"{"evidence":2}"#;
        let intent_ref = object_ref("mfm.test.intent", intent);
        let evidence_ref = object_ref("mfm.test.evidence", evidence);
        let read = history
            .encode_read_conclusion(
                &intent_ref,
                intent,
                &evidence_ref,
                evidence,
                kind,
                &outcome_ref,
                outcome,
            )
            .expect("read");
        let expected = json!({
            "domain": "mfm.run.frame.v1",
            "objects": sorted_objects(vec![
                object_wire(&intent_ref, intent),
                object_wire(&evidence_ref, evidence),
                object_wire(&outcome_ref, outcome),
            ]),
            "previous_head_digest": history.head_digest(),
            "record": {
                "evidence": evidence_ref,
                "intent": intent_ref,
                "kind": "state_concluded_read",
                "outcome": {"kind": outcome_kind, "value": outcome_ref},
            },
            "run_id": run,
            "run_sequence": 2,
        });
        assert_eq!(read.canonical_bytes(), canonical(&expected));
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
            "{\"domain\":\"mfm.run.frame.v1\",\"domain\":",
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
fn qualification_rejects_wrong_run_chain_and_hostile_closure() {
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
        JournalHistory::qualify(
            &run(4),
            StoredRunBytes::new(vec![bytes.clone()]).expect("stored")
        ),
        Err(JournalError::InvalidHistory)
    ));

    let mut wire: serde_json::Value = serde_json::from_slice(&bytes).expect("wire");
    wire["objects"].as_array_mut().expect("objects").pop();
    let hostile = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&wire).expect("json"),
    )
    .expect("canonical");
    assert!(matches!(
        JournalHistory::qualify(
            &run(3),
            StoredRunBytes::new(vec![hostile.as_bytes().to_vec()]).expect("stored")
        ),
        Err(JournalError::InvalidHistory)
    ));

    let mut damaged = bytes;
    damaged[0] = b'[';
    assert!(matches!(
        JournalHistory::qualify(&run(3), StoredRunBytes::new(vec![damaged]).expect("stored")),
        Err(JournalError::InvalidHistory)
    ));
}

#[test]
fn opaque_transfer_is_nonempty_and_bounded() {
    assert!(matches!(
        StoredRunBytes::new(Vec::new()),
        Err(JournalError::InvalidHistory)
    ));
    assert_eq!(
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([0; 32]))
            .algorithm(),
        DigestAlgorithm::Sha256V1
    );
}

#[test]
fn journal_capacity_theorem_is_exact() {
    assert_eq!(MAX_RUN_OBJECT_CANONICAL_BYTES, 8 * 1024 * 1024);
    assert_eq!(MAX_FRAME_NON_PAYLOAD_ENVELOPE, 65_536);
    assert_eq!(
        MAX_FRAME_BYTES,
        3 * MAX_RUN_OBJECT_CANONICAL_BYTES + MAX_FRAME_NON_PAYLOAD_ENVELOPE
    );
    assert_eq!(MAX_RUN_FRAMES, 65_536);
    assert_eq!(MAX_RUN_BYTES, 512 * 1024 * 1024);
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
