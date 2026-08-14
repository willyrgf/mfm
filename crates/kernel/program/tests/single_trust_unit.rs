use super::*;
use mfm_ids::{DigestBytes, SchemaId};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct NonClone {
    text: String,
}

#[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct OtherValue {
    text: String,
}

struct CatalogRead;

impl AccessCapabilityContract for CatalogRead {
    type Mode = ReadMode;
    type Intent = NonClone;
    type Evidence = NonClone;
    type Facts = mfm_capabilities::NoPriorFacts;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.catalog-read")
            .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
    }

    fn total_attempt_bound() -> std::num::NonZeroU16 {
        std::num::NonZeroU16::new(1).expect("nonzero")
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.text == evidence.text)
            .then_some(())
            .ok_or(mfm_capabilities::CapabilityError::EvidenceBinding)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SameSchemaA {
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SameSchemaB {
    text: String,
}

macro_rules! same_schema_value {
    ($type:ty) => {
        impl MfmValue for $type {
            fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
                <NonClone as MfmValue>::schema_descriptor()
            }

            fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
                <NonClone as MfmValue>::semantic_id()
            }
        }
    };
}

same_schema_value!(SameSchemaA);
same_schema_value!(SameSchemaB);

static RETAINED_DECODES: AtomicUsize = AtomicUsize::new(0);
static RETAINED_SCHEMA_DESCRIPTORS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Serialize)]
struct DecodeCounted {
    text: String,
}

impl<'de> Deserialize<'de> for DecodeCounted {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            text: String,
        }

        RETAINED_DECODES.fetch_add(1, Ordering::SeqCst);
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self { text: wire.text })
    }
}

impl MfmValue for DecodeCounted {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        RETAINED_SCHEMA_DESCRIPTORS.fetch_add(1, Ordering::SeqCst);
        <NonClone as MfmValue>::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
        <NonClone as MfmValue>::semantic_id()
    }
}

#[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct OtherDescriptor {
    value: u64,
}

static CONFLICTING_DESCRIPTOR: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnstableDescriptor {
    text: String,
}

impl MfmValue for UnstableDescriptor {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        if CONFLICTING_DESCRIPTOR.load(Ordering::SeqCst) {
            <OtherDescriptor as MfmValue>::schema_descriptor()
        } else {
            <NonClone as MfmValue>::schema_descriptor()
        }
    }

    fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
        <NonClone as MfmValue>::semantic_id()
    }

    fn schema_id() -> mfm_values::Result<SchemaId> {
        <NonClone as MfmValue>::schema_id()
    }
}

fn reference(seed: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            "mfm.test.contract",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([seed; 32]),
        )
        .expect("schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            DigestBytes::from_array([seed.wrapping_add(1); 32]),
        ),
    )
    .expect("reference")
}

fn empty_document(contract_ref: ContentRef) -> ProgramDocument {
    ProgramDocument::new(
        StableId::new("mfm.test-entry-1").expect("entry"),
        contract_ref.clone(),
        contract_ref,
        Vec::new(),
    )
    .expect("document")
}

#[test]
fn program_document_has_a_dedicated_persisted_identity() {
    let document = empty_document(reference(9));
    let program_ref = document.program_ref().expect("program ref");
    assert_eq!(
        program_ref.schema_id(),
        &program_document_schema_id().expect("program schema")
    );
    assert_ne!(
        program_ref.schema_id(),
        document.root_contract_ref().schema_id()
    );

    let canonical = document.canonical_bytes().expect("canonical document");
    assert_eq!(
        ProgramDocument::decode_canonical(canonical.as_bytes()).expect("strict decode"),
        document
    );
    assert!(matches!(
        ProgramDocument::decode_canonical(b"{ \"entry_point_id\": \"invalid\" }"),
        Err(ProgramError::Canonical)
    ));
}

#[test]
fn catalog_erases_and_downcasts_owned_non_clone_values_once() {
    let mut builder = ProgramCatalog::builder();
    let contract_ref = builder.register_value::<NonClone>().expect("registration");
    let (catalog, _) = builder
        .finish(empty_document(contract_ref.clone()))
        .expect("catalog");
    let qualified = catalog
        .qualify(
            contract_ref.clone(),
            NonClone {
                text: "cumulative".to_owned(),
            },
        )
        .expect("qualified");
    let erased = qualified.erase();
    let typed: QualifiedTypedValue<NonClone> = erased
        .try_downcast(&catalog, &contract_ref)
        .expect("downcast");
    assert_eq!(typed.as_ref().text, "cumulative");
}

#[test]
fn catalog_rejects_unregistered_access_material_and_invalid_typed_bytes() {
    let mut builder = ProgramCatalog::builder();
    let capability = builder
        .register_capability::<CatalogRead>()
        .expect("capability");
    let other = builder.register_value::<OtherValue>().expect("other value");
    let contract = nominal_contract_ref::<NonClone>().expect("contract");
    let (catalog, _) = builder
        .finish(empty_document(contract.clone()))
        .expect("catalog");
    let intent = catalog
        .qualify(
            contract.clone(),
            NonClone {
                text: "bound".to_owned(),
            },
        )
        .expect("intent")
        .erase();
    let unregistered = catalog
        .qualify(
            other,
            OtherValue {
                text: "bound".to_owned(),
            },
        )
        .expect("other")
        .erase();
    assert!(catalog
        .validate_access_intent(&capability, &unregistered)
        .is_err());
    assert!(catalog
        .validate_access_evidence(&capability, &intent, &unregistered)
        .is_err());

    let mismatched = catalog
        .qualify(
            contract,
            NonClone {
                text: "fabricated".to_owned(),
            },
        )
        .expect("evidence")
        .erase();
    assert!(catalog
        .validate_access_evidence(&capability, &intent, &mismatched)
        .is_err());
    assert!(catalog
        .qualify_retained_erased(
            nominal_contract_ref::<NonClone>().expect("contract"),
            br#"{"text":1}"#,
        )
        .is_err());
}

#[test]
fn catalog_requires_exact_registered_contract_descriptor_and_rust_type() {
    let mut builder = ProgramCatalog::builder();
    let contract_ref = builder
        .register_value::<SameSchemaA>()
        .expect("registration");
    assert_eq!(
        builder.register_value::<SameSchemaA>(),
        Ok(contract_ref.clone())
    );
    assert_eq!(
        builder.register_value::<SameSchemaB>(),
        Err(ProgramError::InvalidCatalog)
    );
    let (catalog, _) = builder
        .finish(empty_document(contract_ref.clone()))
        .expect("catalog");
    assert!(catalog.contains_value::<SameSchemaA>(&contract_ref));
    assert!(!catalog.contains_value::<SameSchemaB>(&contract_ref));
    assert!(catalog
        .qualify(
            contract_ref.clone(),
            SameSchemaA {
                text: "accepted".to_owned(),
            },
        )
        .is_ok());
    let same_schema_unregistered = ContentRef::new(
        contract_ref.schema_id().clone(),
        raw_content_digest(b"unregistered nominal contract"),
    )
    .expect("foreign contract");
    assert!(matches!(
        catalog.qualify(
            same_schema_unregistered,
            SameSchemaA {
                text: "schema-only".to_owned(),
            },
        ),
        Err(ProgramError::InvalidValue)
    ));
    assert!(matches!(
        catalog.qualify(
            contract_ref,
            SameSchemaB {
                text: "transposed".to_owned(),
            },
        ),
        Err(ProgramError::InvalidValue)
    ));
}

#[test]
fn catalog_rejects_missing_and_conflicting_descriptor_associations() {
    let missing = nominal_contract_ref::<NonClone>().expect("contract");
    assert!(matches!(
        ProgramCatalog::builder().finish(empty_document(missing)),
        Err(ProgramError::InvalidCatalog)
    ));

    CONFLICTING_DESCRIPTOR.store(false, Ordering::SeqCst);
    let mut builder = ProgramCatalog::builder();
    let contract_ref = builder
        .register_value::<UnstableDescriptor>()
        .expect("first descriptor");
    CONFLICTING_DESCRIPTOR.store(true, Ordering::SeqCst);
    assert_eq!(
        builder.register_value::<UnstableDescriptor>(),
        Err(ProgramError::InvalidCatalog)
    );
    CONFLICTING_DESCRIPTOR.store(false, Ordering::SeqCst);
    assert!(builder.finish(empty_document(contract_ref)).is_ok());

    let mut builder = ProgramCatalog::builder();
    let registered = builder.register_value::<NonClone>().expect("registered");
    let (catalog, _) = builder.finish(empty_document(registered)).expect("catalog");
    let unregistered = nominal_contract_ref::<OtherDescriptor>().expect("unregistered");
    assert!(matches!(
        catalog.program(empty_document(unregistered)),
        Err(ProgramError::InvalidCatalog)
    ));
}

#[test]
fn content_equal_catalogs_do_not_share_qualification_brand() {
    let mut left = ProgramCatalog::builder();
    let left_contract = left
        .register_value::<NonClone>()
        .expect("left registration");
    let (left, _) = left
        .finish(empty_document(left_contract.clone()))
        .expect("left catalog");
    let mut right = ProgramCatalog::builder();
    let right_contract = right
        .register_value::<NonClone>()
        .expect("right registration");
    let (right, _) = right
        .finish(empty_document(right_contract))
        .expect("right catalog");
    assert!(!left.same_catalog(&right));
    let qualified = left
        .qualify(
            left_contract.clone(),
            NonClone {
                text: "left".to_owned(),
            },
        )
        .expect("left value");
    assert!(qualified.belongs_to_catalog(&left));
    assert!(!qualified.belongs_to_catalog(&right));
    assert!(matches!(
        qualified
            .erase()
            .try_downcast::<NonClone>(&right, &left_contract),
        Err(ProgramError::InvalidCatalog)
    ));
}

#[test]
fn retained_bytes_decode_once_under_the_exact_association() {
    RETAINED_DECODES.store(0, Ordering::SeqCst);
    RETAINED_SCHEMA_DESCRIPTORS.store(0, Ordering::SeqCst);
    let mut builder = ProgramCatalog::builder();
    let contract_ref = builder
        .register_value::<DecodeCounted>()
        .expect("registration");
    let (catalog, _) = builder
        .finish(empty_document(contract_ref.clone()))
        .expect("catalog");
    let descriptors_after_registration = RETAINED_SCHEMA_DESCRIPTORS.load(Ordering::SeqCst);
    for _ in 0..3 {
        let erased = catalog
            .qualify_retained_erased(contract_ref.clone(), br#"{"text":"retained"}"#)
            .expect("retained value");
        assert_eq!(erased.canonical_bytes(), br#"{"text":"retained"}"#);
    }
    assert_eq!(RETAINED_DECODES.load(Ordering::SeqCst), 3);
    assert_eq!(
        RETAINED_SCHEMA_DESCRIPTORS.load(Ordering::SeqCst),
        descriptors_after_registration
    );
}

#[test]
fn document_rejects_non_identity_zero_state_root() {
    assert!(ProgramDocument::new(
        StableId::new("mfm.test-entry-1").expect("entry"),
        reference(1),
        reference(2),
        Vec::new(),
    )
    .is_err());
}

#[test]
fn document_deserialization_reenters_normalization() {
    let value = serde_json::json!({
        "entry_point_id": "mfm.test-entry-1",
        "root_contract_ref": reference(1),
        "admitted_context_contract_ref": reference(2),
        "declarations": []
    });
    assert!(serde_json::from_value::<ProgramDocument>(value).is_err());
}

#[test]
fn match_arms_must_converge() {
    assert!(MatchDeclaration::new(
        SequentialControlAddress::new(1, Vec::new()).expect("address"),
        reference(1),
        vec![
            MatchVariant::new(
                StableId::new("native").expect("tag"),
                reference(2),
                reference(3),
                SequentialControlAddress::new(2, vec![0]).expect("entry"),
            ),
            MatchVariant::new(
                StableId::new("token").expect("tag"),
                reference(4),
                reference(5),
                SequentialControlAddress::new(3, vec![1]).expect("entry"),
            ),
        ],
    )
    .is_err());
}

#[test]
fn attempt_and_conclusion_capacity_bounds_accept_exact_and_reject_plus_one() {
    let read = ExecutionMode::Read {
        capability_contract_ref: reference(1),
        total_attempt_bound: 3,
        fact_selection_required: false,
    };
    assert_eq!(read.validate(), Ok(()));
    assert!(ExecutionMode::Read {
        capability_contract_ref: reference(1),
        total_attempt_bound: 4,
        fact_selection_required: false,
    }
    .validate()
    .is_err());

    let effect = ExecutionMode::Effect {
        capability_contract_ref: reference(1),
        effect_domain: StableId::new("mfm.test.effect").expect("effect domain"),
        fact_selection_required: false,
    };
    assert_eq!(effect.validate(), Ok(()));
    assert_eq!(effect.total_attempt_bound(), Some(1));

    let current = serde_json::to_value(&effect).expect("current Effect shape");
    assert_eq!(
        serde_json::from_value::<ExecutionMode>(current.clone()).expect("current Effect"),
        effect
    );
    for (field, value) in [
        ("absorbing", serde_json::json!(false)),
        ("total_attempt_bound", serde_json::json!(1)),
    ] {
        let mut old = current.clone();
        old.as_object_mut()
            .expect("Effect object")
            .insert(field.to_owned(), value);
        assert!(serde_json::from_value::<ExecutionMode>(old).is_err());
    }

    let state = StateDeclaration::new(
        SequentialControlAddress::new(0, Vec::new()).expect("address"),
        reference(2),
        reference(1),
        reference(1),
        None,
        ExecutionMode::Pure,
        true,
    )
    .expect("state");
    assert!(state
        .clone()
        .with_maximum_conclusion_bytes(MAX_STATE_CONCLUSION_BYTES)
        .is_ok());
    assert!(state
        .with_maximum_conclusion_bytes(MAX_STATE_CONCLUSION_BYTES + 1)
        .is_err());
}
