use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, FieldPath, SemanticTypeId, StableId};

use super::*;

fn retained_contract() -> RetainedValueContract {
    let contract = RecoverabilityContractV3::embedded().expect("embedded recoverability contract");
    let schema_id = contract
        .schema_id("mfm.primitive-stable_id.v1")
        .expect("fixture schema")
        .clone();
    let evidence_contract_ref = ContentRef::new(
        schema_id.clone(),
        contract.raw_content_digest(br#""shared-payload-evidence""#),
    )
    .expect("fixture evidence contract");
    RetainedValueContract::new(
        schema_id,
        SemanticTypeId::new(
            "mfm.test",
            "shared-retained-payload",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"shared-retained-payload"),
        )
        .expect("fixture semantic type"),
        StableId::new("shared-retained-payload").expect("fixture role"),
        "application/json",
        evidence_contract_ref,
    )
    .expect("fixture retained contract")
}

fn value_ref(contract: &RetainedValueContract, path: &str, bytes: &[u8]) -> ValueRef {
    let producer = ProducerBinding::this_record(&FieldPath::new(path).expect("fixture field path"))
        .expect("fixture producer");
    derive_value_ref(contract, &producer, bytes).expect("fixture value reference")
}

fn rewritten_value_ref(
    value_ref: &ValueRef,
    content_digest: Option<ContentDigest>,
    evidence_hash: Option<ObjectEvidenceDigest>,
    byte_length: Option<u64>,
) -> ValueRef {
    let fields = value_ref.fields().expect("fixture value fields");
    ValueRef::new(
        &fields.artifact_id,
        content_digest.as_ref().unwrap_or(&fields.content_digest),
        evidence_hash.as_ref().unwrap_or(&fields.evidence_hash),
        &fields.schema_id,
        &fields.semantic_type_id,
        &fields.role,
        byte_length.unwrap_or(fields.byte_length),
        &fields.media_type,
        &fields.evidence_contract_ref,
        &fields.producer_binding,
    )
    .expect("rewritten fixture value reference")
}

fn shared_objects(bytes: &[u8]) -> (Vec<ValueRef>, Vec<CommittedObject>) {
    let retained_contract = retained_contract();
    let mut value_refs = vec![
        value_ref(&retained_contract, "payload.left", bytes),
        value_ref(&retained_contract, "payload.right", bytes),
    ];
    value_refs.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let fields = value_refs[0].fields().expect("fixture payload fields");
    let payload = UntrustedObjectPayload::new(
        fields.schema_id,
        fields.content_digest,
        Arc::<[u8]>::from(bytes),
        value_refs.clone(),
    )
    .expect("shared untrusted payload");
    let objects = payload.into_committed().expect("shared committed objects");
    (value_refs, objects)
}

#[test]
fn multi_authority_payload_shares_one_verified_allocation() {
    let bytes = br#""shared-payload""#;
    let (value_refs, objects) = shared_objects(bytes);
    let independently_owned = value_refs
        .iter()
        .cloned()
        .map(|value_ref| {
            CommittedObject::from_persisted(value_ref, bytes.to_vec())
                .expect("independently committed object")
        })
        .collect::<Vec<_>>();

    assert_eq!(objects, independently_owned);
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].value_ref(), &value_refs[0]);
    assert_eq!(objects[1].value_ref(), &value_refs[1]);
    assert_eq!(objects[0].bytes(), bytes);
    assert_eq!(objects[1].bytes(), bytes);
    assert!(Arc::ptr_eq(&objects[0].bytes, &objects[1].bytes));
}

#[test]
fn committed_object_clone_shares_verified_payload() {
    let (_, objects) = shared_objects(br#""clone-payload""#);
    let cloned = objects[0].clone();

    assert_eq!(cloned, objects[0]);
    assert_eq!(cloned.bytes(), objects[0].bytes());
    assert!(Arc::ptr_eq(&cloned.bytes, &objects[0].bytes));
}

#[test]
fn shared_payload_rejects_mismatched_metadata() {
    let bytes = br#""validated-payload""#;
    let retained_contract = retained_contract();
    let valid_ref = value_ref(&retained_contract, "payload.valid", bytes);
    let fields = valid_ref.fields().expect("fixture value fields");
    let contract = RecoverabilityContractV3::embedded().expect("embedded recoverability contract");

    let wrong_length = rewritten_value_ref(
        &valid_ref,
        None,
        None,
        Some(fields.byte_length.checked_add(1).expect("fixture length")),
    );
    assert!(matches!(
        CommittedObject::from_persisted(wrong_length, bytes.to_vec()),
        Err(StoreError::ObjectContentMismatch { .. })
    ));

    let wrong_digest = rewritten_value_ref(
        &valid_ref,
        Some(contract.raw_content_digest(br#""different-payload""#)),
        None,
        None,
    );
    assert!(matches!(
        CommittedObject::from_persisted(wrong_digest, bytes.to_vec()),
        Err(StoreError::ObjectContentMismatch { .. })
    ));

    let wrong_evidence = ObjectEvidencePreimage::new(
        &fields.artifact_id,
        &fields.content_digest,
        &fields.schema_id,
        fields.byte_length.checked_add(1).expect("fixture length"),
        &fields.media_type,
        &fields.evidence_contract_ref,
    )
    .expect("mismatched evidence preimage")
    .evidence_hash()
    .expect("mismatched evidence digest");
    let wrong_evidence = rewritten_value_ref(&valid_ref, None, Some(wrong_evidence), None);
    assert!(matches!(
        CommittedObject::from_persisted(wrong_evidence, bytes.to_vec()),
        Err(StoreError::ObjectContentMismatch { .. })
    ));

    let other_bytes = br#""other-authority-payload""#;
    let other_ref = value_ref(&retained_contract, "payload.other", other_bytes);
    assert!(matches!(
        UntrustedObjectPayload::new(
            fields.schema_id,
            fields.content_digest,
            Arc::<[u8]>::from(bytes.as_slice()),
            vec![other_ref],
        ),
        Err(StoreError::InvalidObjectAuthority { .. })
    ));
}
