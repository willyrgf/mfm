use std::collections::BTreeSet;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, ContentRef, SchemaId};
use mfm_store::test_support::{LegalAdmissionFixture, PreparedLegalAdmission};
use mfm_store::{
    open_in_memory, AppendOutcome, ExistingRunAppendMaterial, InMemoryRunJournalBackend,
    NewlyAppended, ObjectGraphProposal, ProducedObjectRoot, ProducedOutputSlot,
    RunAccessAuthorityIssuer, RunHistoryReader, RunHistoryWriter, SettlementMaterial,
    TransitionMaterial, VerifiedRunView,
};

use super::{
    coordinate_matches_verified_view, verify_portable_run_export_stream,
    write_portable_run_export_stream, ExportKind,
};

#[tokio::test]
async fn closure_wide_lookup_returns_dependency_only_retained_bytes() {
    let source_a = LegalAdmissionFixture::new(0x60).expect("source A fixture");
    let source_b = LegalAdmissionFixture::for_store_in_tenant(
        source_a.store_identity().clone(),
        source_a.tenant_scope_id().clone(),
        0x61,
    )
    .expect("source B fixture")
    .with_effective_output_source();
    let root = LegalAdmissionFixture::for_store_in_tenant(
        source_a.store_identity().clone(),
        source_a.tenant_scope_id().clone(),
        0x62,
    )
    .expect("root fixture")
    .with_effective_output_source();
    let (store, issuer) = open_in_memory(source_a.store_identity().clone());
    source_a
        .provision_in_memory(&store)
        .expect("provision source A");
    source_b
        .provision_in_memory(&store)
        .expect("provision source B");
    root.provision_in_memory(&store).expect("provision root");
    let support_a = source_a
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify source A");
    let support_b = source_b
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify source B");
    let support_root = root
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify root");
    let (writer, reader) = store.split();
    let prepared_a = source_a
        .prepare_on(&writer, &reader, &issuer, &support_a)
        .await
        .expect("prepare source A");
    let view_a = settle_fixture(
        &writer,
        &reader,
        &issuer,
        &source_a,
        prepared_a,
        r#"{"result":"source-a"}"#,
    )
    .await;

    let proposed_a = source_b
        .proposed_effective_output_sources(&view_a)
        .expect("propose A to B");
    let prepared_b = source_b
        .prepare_on_with_sources(&writer, &reader, &issuer, &support_b, proposed_a)
        .await
        .expect("prepare source B");
    let view_b = settle_fixture(
        &writer,
        &reader,
        &issuer,
        &source_b,
        prepared_b,
        r#"{"result":"source-b"}"#,
    )
    .await;

    let proposed_b = root
        .proposed_effective_output_sources(&view_b)
        .expect("propose B to root");
    let prepared_root = root
        .prepare_on_with_sources(&writer, &reader, &issuer, &support_root, proposed_b)
        .await
        .expect("prepare root");
    let root_view = settle_fixture(
        &writer,
        &reader,
        &issuer,
        &root,
        prepared_root,
        r#"{"result":"root"}"#,
    )
    .await;

    let root_content = content_identities(&root_view);
    let source_a_content = content_identities(&view_a);
    let (dependency_ref, expected_bytes) = view_b
        .journal()
        .objects()
        .find_map(|object| {
            let fields = object.value_ref().fields().ok()?;
            let identity = (fields.schema_id.clone(), fields.content_digest.clone());
            (!root_content.contains(&identity) && !source_a_content.contains(&identity)).then(
                || {
                    (
                        ContentRef::new(fields.schema_id, fields.content_digest)
                            .expect("dependency content ref"),
                        object.bytes().to_vec(),
                    )
                },
            )
        })
        .expect("B-only retained payload");
    assert!(
        root_view
            .journal()
            .objects()
            .all(|object| object.value_ref().fields().is_ok_and(|fields| {
                fields.schema_id != *dependency_ref.schema_id()
                    || fields.content_digest != *dependency_ref.content_digest()
            })),
        "root journal must not contain the selected dependency payload"
    );

    let root_authority =
        issuer.authorize_export(root.tenant_scope_id().clone(), root_view.run_id().clone());
    let first_dependency_run_id = [view_a.run_id(), view_b.run_id()]
        .into_iter()
        .min()
        .expect("dependency run");
    assert!(
        root_view.run_id() > first_dependency_run_id,
        "fixture must regress a root/dependency lexical inversion"
    );
    let dependency_authorities = [
        issuer.authorize_export(source_a.tenant_scope_id().clone(), view_a.run_id().clone()),
        issuer.authorize_export(source_b.tenant_scope_id().clone(), view_b.run_id().clone()),
    ];
    let mut bytes = Vec::new();
    let metadata = write_portable_run_export_stream(
        &reader,
        &root_authority,
        &dependency_authorities,
        ExportKind::Semantic,
        &mut bytes,
    )
    .await
    .expect("write recursive source closure");
    let offline = verify_portable_run_export_stream(bytes.as_slice(), metadata.content_ref())
        .await
        .expect("verify recursive stream offline");
    assert_eq!(
        offline.store_scope_id,
        *root_view.store_identity().store_scope_id()
    );
    assert_eq!(offline.tenant_scope_id, *root_view.tenant_scope_id());
    assert_eq!(offline.run_id, *root_view.run_id());
    assert_eq!(offline.export_kind, ExportKind::Semantic);
    assert!(
        coordinate_matches_verified_view(&offline.coordinate, &root_view)
            .expect("compare stream coordinate")
    );

    let replay_authority =
        issuer.authorize_replay(root.tenant_scope_id().clone(), root_view.run_id().clone());
    let history = crate::verify_recorded_history(&reader, &replay_authority)
        .await
        .expect("verify root history");
    let stream = history
        .verify_export_stream(bytes.as_slice(), metadata.content_ref())
        .await
        .expect("bind verified recursive stream");
    assert_eq!(
        stream
            .retained_content(&dependency_ref)
            .expect("lookup B-only retained content"),
        expected_bytes
    );
}

fn content_identities(view: &VerifiedRunView) -> BTreeSet<(SchemaId, ContentDigest)> {
    view.journal()
        .objects()
        .map(|object| {
            let fields = object.value_ref().fields().expect("object authority");
            (fields.schema_id, fields.content_digest)
        })
        .collect()
}

async fn settle_fixture(
    writer: &RunHistoryWriter<InMemoryRunJournalBackend>,
    reader: &RunHistoryReader<InMemoryRunJournalBackend>,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &LegalAdmissionFixture,
    prepared: PreparedLegalAdmission,
    output: &str,
) -> VerifiedRunView {
    let (admit, append) = prepared.into_parts();
    let outcome = writer
        .append_admission(&admit, append)
        .await
        .expect("append fixture admission");
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
        panic!("fixture admission must be new");
    };
    let drive =
        issuer.authorize_drive(fixture.tenant_scope_id().clone(), admitted.run_id().clone());
    let open = writer
        .load_for_drive(&drive)
        .await
        .expect("load open fixture")
        .verify_recorded_history()
        .expect("verify open fixture");
    let [node] = open.certified_spec().nodes() else {
        panic!("fixture must certify exactly one node");
    };
    let [output_slot] = node.settlement_contract().output_slots() else {
        panic!("fixture must certify exactly one output");
    };
    let frame = writer
        .prepare_frame(&drive, &open, node.node_id())
        .await
        .expect("prepare fixture frame");
    let output = PlainCanonicalJsonBytes::from_json_str(output).expect("canonical fixture output");
    let append = writer
        .prepare_append(
            &drive,
            &open,
            fixture.successor_append_request_id().clone(),
            ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                prepared_frame: Box::new(frame),
                settlement: SettlementMaterial::Succeeded {
                    output_roots: vec![ProducedOutputSlot::new(
                        output_slot.output_ordinal(),
                        output_slot.field_path().clone(),
                        ProducedObjectRoot::new(output_slot.value_contract().clone(), output),
                    )],
                    fact_roots: Vec::new(),
                },
                object_graph: ObjectGraphProposal::empty(),
            })),
        )
        .expect("prepare fixture settlement");
    assert!(matches!(
        writer
            .append(&drive, append)
            .await
            .expect("append settlement"),
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
    ));
    reader
        .load_for_replay(
            &issuer.authorize_replay(fixture.tenant_scope_id().clone(), admitted.run_id().clone()),
        )
        .await
        .expect("load closed fixture")
        .verify_recorded_history()
        .expect("verify closed fixture")
}
