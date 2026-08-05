//! Minimal verified fixtures shared by cross-crate contract tests.
//!
//! This module is enabled only by the `test-support` feature.  It deliberately
//! returns the same opaque export evidence that production readers return; it
//! does not expose a second fold or a way to construct production authority.

use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, InvocationIdentity, RunId,
    SchemaId, SchemaVersion, SemanticTypeId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    HistoryObject, PriorRunFactSourceManifest, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_runtime::history::StructuredAdmissionMaterial;
use mfm_spec::structured::{
    BlockTail, CertifiedComponentObject, CertifiedProgramComponents, CertifiedProgramDocument,
    CertifiedProgramRoot, CertifiedStructuralBounds, ExpandedBlock, ExpandedStructuredProgram,
    FailureScope, FailureScopeBinding, LexicalProducer, LexicalSlot, StructuralPath,
    StructuralPathSegment, StructuredFailureContract, StructuredPublicContractRefs,
};
use mfm_spec::CanonicalJsonValue;
use mfm_values::{RetainedValueContract, SchemaIdentity, SchemaKind, SchemaShape};

use super::backend::StructuredRunStore;
use super::fold::{ProgramVerifier, StructuredStoreError, VerifiedProgramData};
use super::memory::StructuredMemoryBackend;
use super::purpose::{ExportRunEvidence, ExportRunReader, RecordedRunEvidence, ReplayRunReader};
use super::qualification::{
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PublicPhysicalBindingVerifier,
};
use super::{
    PhysicalTargetIdentity as StorePhysicalTargetIdentity, StructuredAdmissionRequest,
    StructuredStoreIdentity,
};

/// A deterministic program verifier retained by a portable replay fixture.
pub struct FixtureProgramVerifier {
    entry_point: StableId,
    document: CertifiedProgramDocument,
    expanded: ExpandedStructuredProgram,
    value_schemas: BTreeMap<ContentRef, SchemaIdentity>,
}

impl mfm_authority_seal::ProgramVerifierSeal for FixtureProgramVerifier {}

impl ProgramVerifier for FixtureProgramVerifier {
    fn verify(
        &self,
        entry_point_id: &StableId,
        root: &CertifiedProgramRoot,
        _authored: &CanonicalJsonValue,
    ) -> Result<Arc<VerifiedProgramData>, StructuredStoreError> {
        if entry_point_id != &self.entry_point || root != &self.document.root {
            return Err(StructuredStoreError::Certification);
        }
        Ok(Arc::new(VerifiedProgramData::new(
            self.document.clone(),
            self.expanded.clone(),
            self.value_schemas.clone(),
        )))
    }
}

/// Physical-binding verifier for the fixture's target-fixed retained history.
struct AcceptPhysicalBindings;

impl mfm_authority_seal::PhysicalBindingVerifierSeal for AcceptPhysicalBindings {}

impl PublicPhysicalBindingVerifier for AcceptPhysicalBindings {
    fn verify_authorization(
        &self,
        _context: &PhysicalBindingAuthorization<'_>,
        _certificate: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        Ok(())
    }

    fn verify_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> Result<(), StructuredStoreError> {
        Ok(())
    }
}

/// One real admitted run plus the exact verifiers needed to fold its export offline.
pub struct OfflineExportFixture {
    /// Opaque export evidence loaded through the production purpose reader.
    pub export: ExportRunEvidence,
    /// The same run loaded through the recorded-replay purpose reader.
    pub recorded: RecordedRunEvidence,
    /// Program trust used by the store fold and the isolated replay fold.
    pub program_verifier: FixtureProgramVerifier,
    /// Physical-binding trust used by the store fold and isolated replay fold.
    physical_binding_verifier: AcceptPhysicalBindings,
}

impl OfflineExportFixture {
    /// Returns the fixture's opaque physical-binding trust for replay tests.
    pub fn physical_binding_verifier(&self) -> &dyn PublicPhysicalBindingVerifier {
        &self.physical_binding_verifier
    }
}

/// Builds one bounded, zero-state run through the store's real admission and export readers.
pub async fn zero_state_export(discriminator: u8) -> super::Result<OfflineExportFixture> {
    let fixture = zero_state_fixture(discriminator);
    let program_verifier = verifier(&fixture);
    let store = StructuredRunStore::new(
        StructuredMemoryBackend::new(store_identity(discriminator)),
        Arc::new(verifier(&fixture)),
        Arc::new(AcceptPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let run_id = run_id(discriminator);
    writer
        .admit_run(admission(&fixture, run_id.clone(), discriminator))
        .await?;
    let export_reader = ExportRunReader::new(reader.clone());
    let export = export_reader.load_for_export(&run_id).await?;
    let recorded = ReplayRunReader::new(reader)
        .load_for_recorded_verify(&run_id)
        .await?;
    Ok(OfflineExportFixture {
        export,
        recorded,
        program_verifier,
        physical_binding_verifier: AcceptPhysicalBindings,
    })
}

struct Fixture {
    entry_point: StableId,
    document: CertifiedProgramDocument,
    expanded: ExpandedStructuredProgram,
    input: LexicalSlot,
    value_schema: SchemaIdentity,
}

fn verifier(fixture: &Fixture) -> FixtureProgramVerifier {
    let mut value_schemas = BTreeMap::new();
    value_schemas.insert(
        fixture.input.contract_ref.clone(),
        fixture.value_schema.clone(),
    );
    FixtureProgramVerifier {
        entry_point: fixture.entry_point.clone(),
        document: fixture.document.clone(),
        expanded: fixture.expanded.clone(),
        value_schemas,
    }
}

fn admission(fixture: &Fixture, run_id: RunId, discriminator: u8) -> StructuredAdmissionRequest {
    StructuredAdmissionRequest::new(
        run_id,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
            .expect("fixture tenant"),
        InvocationIdentity::new("00000000-0000-4000-8000-000000000001")
            .expect("fixture invocation"),
        fixture.entry_point.clone(),
        fixture.document.clone(),
        admission_material(discriminator),
        vec![super::ProposedCanonicalValue::from_json("7").expect("fixture input")],
        AppendRequestId::new(format!("portable-fixture-admit-{discriminator}"))
            .expect("fixture append id"),
    )
}

fn admission_material(discriminator: u8) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "fixture.admission-configuration",
            discriminator,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "fixture.admission-context",
            discriminator,
        ),
        PriorRunFactSourceManifest::new(Vec::new())
            .expect("empty source manifest")
            .to_history_object()
            .expect("source manifest object"),
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "fixture.admission-routing",
            discriminator,
        ),
        Vec::new(),
    )
    .expect("admission material")
}

fn admission_object(object_type: &str, schema_name: &str, discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        StableId::new(object_type).expect("object type"),
        SchemaId::new(
            schema_name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, schema_name.as_bytes()[0]]),
        )
        .expect("admission schema"),
        "{\"entries\":[]}",
    )
    .expect("admission object")
}

fn zero_state_fixture(discriminator: u8) -> Fixture {
    let entry_point = StableId::new(format!("fixture.zero.{discriminator}")).expect("entry point");
    let path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: entry_point.clone(),
    }])
    .expect("root path");
    let value_schema = value_schema(discriminator);
    let contract = value_contract(&value_schema, discriminator);
    let contract_ref = retained_value_contract_ref(&contract);
    let input = LexicalSlot {
        lexical_path: path.clone(),
        contract_ref: contract_ref.clone(),
        producer: LexicalProducer::AdmissionRoot {
            root_id: StableId::new("input").expect("input id"),
        },
    };
    let expanded = ExpandedStructuredProgram {
        operation_id: entry_point.clone(),
        input_roots: vec![input.clone()],
        output_contract_ref: contract_ref.clone(),
        failure_contract: StructuredFailureContract::Never,
        root: ExpandedBlock {
            path,
            failure_scope: FailureScopeBinding::Owns {
                scope: FailureScope {
                    scope_id: StableId::new(format!("fixture.scope.{discriminator}"))
                        .expect("scope id"),
                    failure_contract: StructuredFailureContract::Never,
                    default_mappers: Vec::new(),
                },
            },
            declarations: Vec::new(),
            failure_exits: Vec::new(),
            tail: BlockTail::Normal(input.clone()),
        },
    };
    let document = document(&expanded, &contract, discriminator);
    Fixture {
        entry_point,
        document,
        expanded,
        input,
        value_schema,
    }
}

fn document(
    expanded: &ExpandedStructuredProgram,
    contract: &RetainedValueContract,
    discriminator: u8,
) -> CertifiedProgramDocument {
    let contract_ref = retained_value_contract_ref(contract);
    let canonical = contract.canonical_json().expect("contract canonical");
    let component = CertifiedComponentObject {
        object_type: StableId::new("structured.data_contract").expect("component type"),
        content_ref: contract_ref.clone(),
        value: CanonicalJsonValue::from_canonical_json(canonical.as_bytes())
            .expect("component value"),
    };
    let authored_value = CanonicalJsonValue::new(serde_json::json!({
        "fixture": discriminator,
    }))
    .expect("authored value");
    let authored_canonical = authored_value.canonical_json().expect("authored canonical");
    let authored_ref = ContentRef::new(
        SchemaId::new(
            "mfm.authored-structured-program",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.authored-structured-program.v1"),
        )
        .expect("authored schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(authored_canonical.as_bytes()),
        ),
    )
    .expect("authored reference");
    let authored_component = CertifiedComponentObject {
        object_type: StableId::new("structured.authored_program").expect("authored type"),
        content_ref: authored_ref.clone(),
        value: authored_value,
    };
    let placeholder = content_ref("fixture.placeholder", discriminator);
    CertifiedProgramDocument {
        root: CertifiedProgramRoot {
            components: CertifiedProgramComponents {
                certified_program_contract_ref: placeholder.clone(),
                entry_point_contract_ref: placeholder.clone(),
                qualified_entry_point_admission_policy_ref: placeholder.clone(),
                authored_program_ref: authored_ref,
                expanded_program_ref: expanded.content_ref().expect("expanded ref"),
                expansion_profile_ref: placeholder.clone(),
                expansion_proof_ref: placeholder.clone(),
                policy_coverage_proof_ref: placeholder.clone(),
                public_input_output_failure_contract_refs: StructuredPublicContractRefs {
                    input_contract_refs: expanded
                        .input_roots
                        .iter()
                        .map(|slot| slot.contract_ref.clone())
                        .collect(),
                    output_contract_ref: expanded.output_contract_ref.clone(),
                    failure_contract_ref: expanded
                        .failure_contract
                        .contract_ref()
                        .expect("failure ref"),
                },
                certified_structural_bounds: CertifiedStructuralBounds {
                    max_occurrences: 8,
                    max_declarations: 8,
                    max_lanes: 8,
                    max_fan_out_depth: 2,
                    max_branch_depth: 8,
                },
                state_capability_adapter_signer_resource_manifest_closure_ref: placeholder.clone(),
                secret_free_implementation_manifest_closure_ref: placeholder.clone(),
                certification_predicate_set_ref: placeholder,
            },
            canonical_component_closure_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(&[discriminator, 9]),
            ),
        },
        component_closure: vec![authored_component, component],
    }
}

fn retained_value_contract_ref(contract: &RetainedValueContract) -> ContentRef {
    mfm_spec::structured::retained_value_contract_ref(contract).expect("contract ref")
}

fn value_contract(schema: &SchemaIdentity, discriminator: u8) -> RetainedValueContract {
    RetainedValueContract::new(
        schema.schema_id().expect("schema"),
        fixture_semantic_type(discriminator),
        StableId::new("fixture.integer").expect("role"),
        "application/json",
        content_ref("fixture.evidence", discriminator),
    )
    .expect("value contract")
}

fn value_schema(discriminator: u8) -> SchemaIdentity {
    SchemaIdentity::new(
        SchemaKind::Value,
        Some(fixture_semantic_type(discriminator)),
        "fixture.integer",
        SchemaVersion::new("1").expect("schema version"),
        SchemaShape::UnsignedInteger { bits: 64 },
    )
    .expect("value schema")
}

fn fixture_semantic_type(discriminator: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.fixture",
        "integer",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[discriminator, 2]),
    )
    .expect("semantic type")
}

fn content_ref(name: &str, discriminator: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, 3]),
        )
        .expect("content schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(&[discriminator, 4]),
        ),
    )
    .expect("content ref")
}

fn store_identity(discriminator: u8) -> StructuredStoreIdentity {
    StructuredStoreIdentity {
        store_scope_id: StoreScopeId::new(format!(
            "{}{:032x}",
            StoreScopeId::PREFIX,
            discriminator
        ))
        .expect("store scope"),
        store_epoch: StoreEpoch::new(1),
        physical_target: Some(StorePhysicalTargetIdentity {
            target_key: format!("fixture-target-{discriminator}"),
            database_oid: u32::from(discriminator),
            fence_generation: 1,
            release_epoch: 1,
            current_incarnation_ref: ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(&[discriminator, 6]),
            ),
        }),
    }
}

fn run_id(discriminator: u8) -> RunId {
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[discriminator, 5]),
    )
}
