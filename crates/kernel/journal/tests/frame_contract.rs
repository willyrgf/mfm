use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, RunId};
use mfm_journal::{
    decode_frame, frame_head_digest, seal_frame, JournalError, MAX_FRAME_BYTES, MAX_RUN_FRAMES,
};
use serde_json::{json, Value};

fn run() -> RunId {
    RunId::from_digest(DigestBytes::from_array([1; 32]))
}
fn canonical(value: &Value) -> PlainCanonicalJsonBytes {
    PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(value).unwrap()).unwrap()
}

#[test]
fn opaque_v6_envelope_round_trips_exact_bytes_and_recursive_head_identity() {
    let run = run();
    let payload = canonical(&json!({"caller_owned": [1, null, {"input": true}]}));
    let first = seal_frame(&run, 1, None, &payload).unwrap();
    let expected = canonical(&json!({
        "domain": "mfm.run.frame.v6", "run_id": run, "run_sequence": 1,
        "previous_head_digest": null, "payload": {"caller_owned": [1, null, {"input": true}]},
    }));
    assert_eq!(first.canonical_bytes(), expected.as_bytes());
    assert_eq!(first.head_digest(), &frame_head_digest(expected.as_bytes()));
    assert_eq!(first.head_digest().algorithm(), DigestAlgorithm::Sha256V1);
    let second = seal_frame(&run, 2, Some(first.head_digest()), &canonical(&json!(null))).unwrap();
    for frame in [&first, &second] {
        let decoded = decode_frame(frame.canonical_bytes()).unwrap();
        assert_eq!(decoded.run_id(), frame.run_id());
        assert_eq!(decoded.run_sequence(), frame.run_sequence());
        assert_eq!(decoded.previous_head_digest(), frame.previous_head_digest());
        assert_eq!(decoded.head_digest(), frame.head_digest());
        assert_eq!(decoded.payload(), frame.payload());
        assert_eq!(decoded.canonical_bytes(), frame.canonical_bytes());
    }
    assert_eq!(first.canonical_bytes(), expected.as_bytes());
}

#[test]
fn envelope_rejects_noncanonical_unknown_duplicate_and_obsolete_fields() {
    let first = seal_frame(&run(), 1, None, &canonical(&json!({}))).unwrap();
    let base: Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
    for mutation in 0..5 {
        let mut value = base.clone();
        match mutation {
            0 => value["domain"] = "mfm.run.frame.v5".into(),
            1 => value["objects"] = json!([]),
            2 => value["record"] = json!({"kind": "run_admitted"}),
            3 => value["run_sequence"] = 0.into(),
            4 => value["run_sequence"] = 2.into(),
            _ => unreachable!(),
        }
        assert!(
            decode_frame(canonical(&value).as_bytes()).is_err(),
            "mutation {mutation}"
        );
    }
    let mut trailing = first.canonical_bytes().to_vec();
    trailing.push(b' ');
    assert!(decode_frame(&trailing).is_err());
    let duplicate = String::from_utf8(first.canonical_bytes().to_vec())
        .unwrap()
        .replacen("{", "{\"run_sequence\":1,", 1);
    assert!(decode_frame(duplicate.as_bytes()).is_err());
}

#[test]
fn local_header_checks_do_not_reconstruct_predecessor_history() {
    let payload = canonical(&json!({"arbitrary_payload": true}));
    let previous =
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([9; 32]));
    assert!(seal_frame(&run(), 0, None, &payload).is_err());
    assert!(seal_frame(&run(), 1, Some(&previous), &payload).is_err());
    assert!(seal_frame(&run(), 2, None, &payload).is_err());
    let wrong_algorithm = payload.content_digest();
    assert!(seal_frame(&run(), 2, Some(&wrong_algorithm), &payload).is_err());
    let last = seal_frame(&run(), MAX_RUN_FRAMES, Some(&previous), &payload).unwrap();
    assert_eq!(
        decode_frame(last.canonical_bytes()).unwrap().run_sequence(),
        MAX_RUN_FRAMES
    );
    assert!(matches!(
        seal_frame(&run(), MAX_RUN_FRAMES + 1, Some(&previous), &payload),
        Err(JournalError::FrameCount(_))
    ));
}

#[test]
fn decoding_rejects_actual_frame_overflow_before_parsing_and_retains_parser_causes() {
    let oversized = vec![b' '; MAX_FRAME_BYTES + 1];
    let Err(JournalError::FrameSize(size)) = decode_frame(&oversized) else {
        panic!("frame size")
    };
    assert_eq!(size.actual(), MAX_FRAME_BYTES as u64 + 1);
    assert_eq!(size.limit(), MAX_FRAME_BYTES as u64);
    let error = decode_frame(b"{").unwrap_err();
    assert!(std::error::Error::source(&error).is_some());
    let projected = serde_json::to_value(error).unwrap();
    assert_eq!(projected["canonical"]["operation"], "decode");
}
