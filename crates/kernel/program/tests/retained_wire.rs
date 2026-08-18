use mfm_program::{Program, ProgramError};
use mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES;

const PURE_PROGRAM: &[u8] = include_bytes!("fixtures/pure-program-v2.json");
const READ_MATCH_PROGRAM: &[u8] = include_bytes!("fixtures/read-match-program-v2.json");
const PROGRAM_SCHEMA: &str = "schema:mfm-program-document:2:sha256-jcs-v1:fc3c33ba3470e25a166df52597c4b45a723dd6d0823d1318938c29225323076c";

#[test]
fn retained_program_v2_wires_are_exact() {
    for (bytes, digest) in [
        (
            PURE_PROGRAM,
            "content:sha256-v1:e7faf0b0db6284824c9d75baf9cd237a43fdf3627ac71f0ea5be6d958d4d035f",
        ),
        (
            READ_MATCH_PROGRAM,
            "content:sha256-v1:82aa393ce2010c625726fd23faba701844f7164c4c3798425cdc8b6bd3ef2940",
        ),
    ] {
        let program = Program::decode_canonical(bytes).expect("retained Program");
        assert_eq!(program.canonical_bytes(), bytes);
        assert_eq!(program.content_ref().schema_id().as_str(), PROGRAM_SCHEMA);
        assert_eq!(program.content_ref().content_digest().as_str(), digest);
    }
}

#[test]
fn public_decoder_rejects_hostile_wire() {
    assert_decode_error(b"{", ProgramError::Canonical);

    let mut noncanonical = PURE_PROGRAM.to_vec();
    noncanonical.push(b' ');
    assert_decode_error(&noncanonical, ProgramError::Canonical);
    assert_decode_error(
        &vec![b' '; MAX_RUN_OBJECT_CANONICAL_BYTES + 1],
        ProgramError::Capacity,
    );

    let raw: serde_json::Value = serde_json::from_slice(PURE_PROGRAM).expect("Program JSON");
    reject_mutation(&raw, |unknown| {
        unknown["version"] = serde_json::json!(1);
    });

    for field in ["next_index", "failure_next_index"] {
        reject_mutation(&raw, |omitted| {
            omitted["declarations"][0]["value"]
                .as_object_mut()
                .expect("State object")
                .remove(field);
        });
    }
    reject_mutation(&raw, |overflow| {
        overflow["declarations"][0]["value"]["next_index"] = serde_json::json!(65_536_u64);
    });
    reject_mutation(&raw, |nested_unknown| {
        nested_unknown["declarations"][0]["value"]["execution"]["legacy"] = serde_json::json!(true);
    });
    for tag in ["unknown", "state_prepared", "state_concluded_access"] {
        reject_mutation(&raw, |hostile| {
            hostile["declarations"][0]["kind"] = serde_json::json!(tag);
        });
    }
    for field in ["occurrence", "preparation"] {
        reject_mutation(&raw, |retired| {
            retired["declarations"][0]["value"][field] = serde_json::json!({});
        });
    }
    for field in [
        "execution_binding",
        "physical_target_ref",
        "adapter_implementation_ref",
    ] {
        reject_mutation(&raw, |retired| {
            retired["declarations"][0]["value"]["execution"][field] = serde_json::json!({});
        });
    }

    let read_match: serde_json::Value =
        serde_json::from_slice(READ_MATCH_PROGRAM).expect("Read/Match Program JSON");
    let mut hostile = read_match.clone();
    hostile["declarations"][1]["value"]["variants"]
        .as_array_mut()
        .expect("Match variants")
        .swap(0, 1);
    assert_decode_error(&canonical_json(&hostile), ProgramError::InvalidContract);
}

fn assert_decode_error(bytes: &[u8], expected: ProgramError) {
    assert_eq!(Program::decode_canonical(bytes), Err(expected));
}

fn reject_mutation(fixture: &serde_json::Value, mutate: impl FnOnce(&mut serde_json::Value)) {
    let mut hostile = fixture.clone();
    mutate(&mut hostile);
    assert_decode_error(&canonical_json(&hostile), ProgramError::Canonical);
}

fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&value.to_string())
        .expect("canonical JSON")
        .to_vec()
}
