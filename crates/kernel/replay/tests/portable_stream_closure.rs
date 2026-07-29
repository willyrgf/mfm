use std::collections::BTreeSet;

use mfm_canonical::{PlainCanonicalJsonBytes, RecoverabilityContractV2};
use mfm_ids::{ContentRef, RunId};
use mfm_replay::trace_export::{
    verify_portable_run_export_stream, write_portable_run_export_stream, ExportKind,
    OfflineFactCompleteness,
};
use mfm_replay::v1::{required_export_source_run_ids, ReplayErrorKind};
use mfm_store::v1::test_support::{LegalAdmissionFixture, PreparedLegalAdmission};
use mfm_store::{
    AppendOutcome, AsyncInMemoryRunStore, ExistingRunAppendMaterial, NewlyAppended,
    ObjectGraphProposal, ProducedObjectRoot, ProducedOutputSlot, RunAccessAuthorityIssuer,
    RunJournalStore, SettlementMaterial, TransitionMaterial, VerifiedRunView,
};

#[tokio::test]
async fn recursive_source_closure_is_canonical_complete_and_callback_free() {
    let source_a = LegalAdmissionFixture::new(0x50).expect("source A fixture");
    let (store, issuer) = AsyncInMemoryRunStore::new(source_a.store_identity().clone());
    source_a
        .provision_in_memory(&store)
        .expect("provision source A");
    let prepared_a = source_a
        .prepare_on(&store, &issuer)
        .await
        .expect("prepare source A");
    let view_a = settle_fixture(
        &store,
        &issuer,
        &source_a,
        prepared_a,
        r#"{"result":"source-a"}"#,
    )
    .await;

    let source_b = LegalAdmissionFixture::for_store_in_tenant(
        source_a.store_identity().clone(),
        source_a.tenant_scope_id().clone(),
        0x51,
    )
    .expect("source B fixture")
    .with_effective_output_source();
    source_b
        .provision_in_memory(&store)
        .expect("provision source B");
    let proposed_a = source_b
        .proposed_effective_output_sources(&view_a)
        .expect("propose A to B");
    let prepared_b = source_b
        .prepare_on_with_sources(&store, &issuer, proposed_a)
        .await
        .expect("prepare source B");
    let view_b = settle_fixture(
        &store,
        &issuer,
        &source_b,
        prepared_b,
        r#"{"result":"source-b"}"#,
    )
    .await;

    let root = LegalAdmissionFixture::for_store_in_tenant(
        source_a.store_identity().clone(),
        source_a.tenant_scope_id().clone(),
        0x52,
    )
    .expect("root fixture")
    .with_effective_output_source();
    root.provision_in_memory(&store).expect("provision root");
    let proposed_b = root
        .proposed_effective_output_sources(&view_b)
        .expect("propose B to root");
    let prepared_root = root
        .prepare_on_with_sources(&store, &issuer, proposed_b)
        .await
        .expect("prepare root");
    let root_view = settle_fixture(
        &store,
        &issuer,
        &root,
        prepared_root,
        r#"{"result":"root"}"#,
    )
    .await;

    let root_authority =
        issuer.authorize_export(root.tenant_scope_id().clone(), root_view.run_id().clone());
    assert_eq!(
        required_export_source_run_ids(&store, &root_authority)
            .await
            .expect("discover root source"),
        vec![view_b.run_id().clone()]
    );
    let authority_a =
        issuer.authorize_export(source_a.tenant_scope_id().clone(), view_a.run_id().clone());
    let authority_b =
        issuer.authorize_export(source_b.tenant_scope_id().clone(), view_b.run_id().clone());

    let mut omitted_output = Vec::new();
    let error = write_portable_run_export_stream(
        &store,
        &root_authority,
        &[issuer.authorize_export(source_b.tenant_scope_id().clone(), view_b.run_id().clone())],
        ExportKind::Semantic,
        &mut omitted_output,
    )
    .await
    .expect_err("recursive source A is required");
    assert_eq!(error.kind(), ReplayErrorKind::SourceRunExportDenied);
    assert!(omitted_output.is_empty());

    let mut duplicate_output = Vec::new();
    let error = write_portable_run_export_stream(
        &store,
        &root_authority,
        &[
            issuer.authorize_export(source_a.tenant_scope_id().clone(), view_a.run_id().clone()),
            issuer.authorize_export(source_b.tenant_scope_id().clone(), view_b.run_id().clone()),
            issuer.authorize_export(source_a.tenant_scope_id().clone(), view_a.run_id().clone()),
        ],
        ExportKind::Semantic,
        &mut duplicate_output,
    )
    .await
    .expect_err("duplicate dependency authority is extra");
    assert_eq!(error.kind(), ReplayErrorKind::InvalidExport);
    assert!(duplicate_output.is_empty());

    let mut first = Vec::new();
    let first_metadata = write_portable_run_export_stream(
        &store,
        &root_authority,
        &[authority_a, authority_b],
        ExportKind::Semantic,
        &mut first,
    )
    .await
    .expect("write recursive source closure");
    let mut second = Vec::new();
    let second_metadata = write_portable_run_export_stream(
        &store,
        &root_authority,
        &[
            issuer.authorize_export(source_b.tenant_scope_id().clone(), view_b.run_id().clone()),
            issuer.authorize_export(source_a.tenant_scope_id().clone(), view_a.run_id().clone()),
        ],
        ExportKind::Semantic,
        &mut second,
    )
    .await
    .expect("authority iteration order is immaterial");
    assert_eq!(first, second);
    assert_eq!(first_metadata, second_metadata);

    let verified =
        verify_portable_run_export_stream(first.as_slice(), first_metadata.content_ref())
            .await
            .expect("verify recursive source closure offline");
    assert_eq!(verified.run_id(), root_view.run_id());
    assert_eq!(
        verified.fact_completeness(),
        OfflineFactCompleteness::UnverifiedPortableStream
    );

    let records = records(&first);
    let frames = frames(&records);
    assert_eq!(join_frames(&frames), first);
    let run_ids = frames
        .iter()
        .filter_map(|frame| {
            (frame["kind"].as_str() == Some("run_begin"))
                .then(|| RunId::parse(frame["run_id"].as_str()?).ok())
                .flatten()
        })
        .collect::<Vec<_>>();
    let mut dependencies = vec![view_a.run_id().clone(), view_b.run_id().clone()];
    dependencies.sort();
    assert_eq!(run_ids.first(), Some(root_view.run_id()));
    assert_eq!(&run_ids[1..], dependencies);
    assert_global_payload_order_and_authorities(&records, &frames);

    let run_ranges = frame_group_ranges(&frames, "run_begin", "run_end");
    assert_eq!(run_ranges.len(), 3);
    let root_swap = swap_adjacent_groups(&records, &run_ranges[0], &run_ranges[1]);
    assert_invalid(&join_records(&root_swap), "root is position-bound").await;

    let dependency_reorder = swap_adjacent_groups(&records, &run_ranges[1], &run_ranges[2]);
    assert_invalid(&join_records(&dependency_reorder), "dependency RunId order").await;

    let mut repeated_root = records.clone();
    let root_group = records[run_ranges[0].clone()].to_vec();
    repeated_root.splice(run_ranges[0].end..run_ranges[0].end, root_group);
    assert_invalid(&join_records(&repeated_root), "root cannot reappear").await;

    let mut missing_dependency = records.clone();
    missing_dependency.drain(run_ranges[2].clone());
    assert_invalid(&join_records(&missing_dependency), "missing dependency run").await;

    let dependency_commit_starts = (run_ranges[1].start..run_ranges[1].end)
        .filter(|index| frames[*index]["kind"].as_str() == Some("commit_begin"))
        .collect::<Vec<_>>();
    assert!(dependency_commit_starts.len() >= 2);
    let mut non_closed_dependency = records.clone();
    non_closed_dependency.drain(dependency_commit_starts[1]..run_ranges[1].end.saturating_sub(1));
    assert_invalid(
        &join_records(&non_closed_dependency),
        "non-closed dependency run",
    )
    .await;

    let object_ranges = frame_group_ranges(&frames, "object_begin", "object_end");
    assert!(!object_ranges.is_empty());
    let mut missing_dependency_content = records.clone();
    missing_dependency_content.drain(object_ranges.last().expect("object range").clone());
    assert_invalid(
        &join_records(&missing_dependency_content),
        "missing closure-wide retained payload",
    )
    .await;

    let first_authority = frames
        .iter()
        .position(|frame| frame["kind"].as_str() == Some("object_authority"))
        .expect("object authority");
    let mut duplicate_authority = records.clone();
    duplicate_authority.insert(first_authority, records[first_authority].clone());
    assert_invalid(
        &join_records(&duplicate_authority),
        "duplicate object authority",
    )
    .await;

    let mut missing_authority = records.clone();
    missing_authority.remove(first_authority);
    assert_invalid(
        &join_records(&missing_authority),
        "missing object authority",
    )
    .await;

    let authority_pair = frames
        .windows(2)
        .position(|pair| {
            pair[0]["kind"].as_str() == Some("object_authority")
                && pair[1]["kind"].as_str() == Some("object_authority")
        })
        .expect("shared payload with multiple authorities");
    let mut reordered_authorities = records.clone();
    reordered_authorities.swap(authority_pair, authority_pair + 1);
    assert_invalid(
        &join_records(&reordered_authorities),
        "reordered object authorities",
    )
    .await;

    let mut evidence_tamper = frames.clone();
    let authority = evidence_tamper
        .iter_mut()
        .find(|frame| frame["kind"].as_str() == Some("object_authority"))
        .expect("object authority");
    authority["value_ref"]["role"] = serde_json::Value::String("mfm.tampered-role".to_owned());
    assert_invalid(
        &join_frames(&evidence_tamper),
        "tampered object evidence binding",
    )
    .await;

    for (kind, field, replacement, reason) in [
        (
            "commit_begin",
            "run_sequence",
            serde_json::Value::String("2".to_owned()),
            "commit sequence gap",
        ),
        (
            "commit_begin",
            "schema_id",
            serde_json::Value::String(first_metadata.schema_id().as_str().to_owned()),
            "commit schema mismatch",
        ),
        (
            "commit_begin",
            "byte_length",
            serde_json::Value::String("0".to_owned()),
            "commit length mismatch",
        ),
        (
            "commit_begin",
            "content_digest",
            serde_json::Value::String(format!("content:sha256-v1:{}", "0".repeat(64))),
            "commit content digest mismatch",
        ),
        (
            "record_begin",
            "schema_id",
            serde_json::Value::String(first_metadata.schema_id().as_str().to_owned()),
            "record schema mismatch",
        ),
        (
            "record_begin",
            "byte_length",
            serde_json::Value::String("0".to_owned()),
            "record length mismatch",
        ),
        (
            "record_begin",
            "content_digest",
            serde_json::Value::String(format!("content:sha256-v1:{}", "0".repeat(64))),
            "record content digest mismatch",
        ),
        (
            "record_begin",
            "ordinal",
            serde_json::Value::Number(1u64.into()),
            "record ordinal gap",
        ),
        (
            "object_begin",
            "schema_id",
            serde_json::Value::String(first_metadata.schema_id().as_str().to_owned()),
            "object schema mismatch",
        ),
        (
            "object_begin",
            "byte_length",
            serde_json::Value::String("0".to_owned()),
            "object length mismatch",
        ),
        (
            "object_begin",
            "content_digest",
            serde_json::Value::String(format!("content:sha256-v1:{}", "0".repeat(64))),
            "object content digest mismatch",
        ),
    ] {
        let mut tampered = frames.clone();
        let frame = tampered
            .iter_mut()
            .find(|frame| frame["kind"].as_str() == Some(kind))
            .expect("target frame");
        frame[field] = replacement;
        assert_invalid(&join_frames(&tampered), reason).await;
    }

    let mut tenant_tamper = frames.clone();
    tenant_tamper[0]["tenant_scope_id"] =
        serde_json::Value::String(format!("mfm.tenant_scope.v1:{}", "f".repeat(32)));
    assert_invalid(&join_frames(&tenant_tamper), "tenant mismatch").await;

    let mut scope_tamper = frames.clone();
    scope_tamper[0]["store_scope_id"] =
        serde_json::Value::String(format!("mfm.store_scope.v1:{}", "f".repeat(32)));
    assert_invalid(&join_frames(&scope_tamper), "store scope mismatch").await;

    let mut coordinate_tamper = frames.clone();
    let coordinate = &mut coordinate_tamper[0]["coordinate"];
    let digest = coordinate["containing_commit_digest"]
        .as_str()
        .expect("semantic closure digest")
        .to_owned();
    let replacement = if digest.ends_with('0') { '1' } else { '0' };
    let mut changed = digest;
    changed.pop();
    changed.push(replacement);
    coordinate["containing_commit_digest"] = serde_json::Value::String(changed);
    assert_invalid(&join_frames(&coordinate_tamper), "root coordinate mismatch").await;

    let mut kind_tamper = frames;
    kind_tamper[0]["export_kind"] = serde_json::Value::String("audit".to_owned());
    assert_invalid(&join_frames(&kind_tamper), "root kind mismatch").await;
}

async fn settle_fixture(
    store: &AsyncInMemoryRunStore,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &LegalAdmissionFixture,
    prepared: PreparedLegalAdmission,
    output: &str,
) -> VerifiedRunView {
    let (admit, append) = prepared.into_parts();
    let outcome = store
        .append_admission(&admit, append)
        .await
        .expect("append fixture admission");
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
        panic!("fixture admission must be new");
    };
    let drive =
        issuer.authorize_drive(fixture.tenant_scope_id().clone(), admitted.run_id().clone());
    let open = store
        .load_committed_journal(&drive)
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
    let frame = store
        .prepare_frame(&drive, &open, node.node_id())
        .await
        .expect("prepare fixture frame");
    let output = PlainCanonicalJsonBytes::from_json_str(output).expect("canonical fixture output");
    let append = store
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
        store
            .append(&drive, append)
            .await
            .expect("append settlement"),
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
    ));
    store
        .load_committed_journal(&drive)
        .await
        .expect("load closed fixture")
        .verify_recorded_history()
        .expect("verify closed fixture")
}

fn records(bytes: &[u8]) -> Vec<Vec<u8>> {
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .map(<[u8]>::to_vec)
        .collect()
}

fn frames(records: &[Vec<u8>]) -> Vec<serde_json::Value> {
    records
        .iter()
        .map(|record| {
            assert_eq!(record.first(), Some(&0x1e));
            assert_eq!(record.last(), Some(&b'\n'));
            serde_json::from_slice(&record[1..record.len() - 1]).expect("stream frame")
        })
        .collect()
}

fn frame_group_ranges(
    frames: &[serde_json::Value],
    begin: &str,
    end: &str,
) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, frame) in frames.iter().enumerate() {
        match frame["kind"].as_str() {
            Some(kind) if kind == begin => {
                assert!(start.replace(index).is_none());
            }
            Some(kind) if kind == end => {
                let start = start.take().expect("group begin");
                ranges.push(start..index + 1);
            }
            _ => {}
        }
    }
    assert!(start.is_none());
    ranges
}

fn assert_global_payload_order_and_authorities(records: &[Vec<u8>], frames: &[serde_json::Value]) {
    let mut identities = Vec::new();
    let mut authorities = Vec::<Vec<u8>>::new();
    let mut in_object = false;
    for (record, frame) in records.iter().zip(frames) {
        match frame["kind"].as_str() {
            Some("object_begin") => {
                in_object = true;
                identities.push((
                    frame["schema_id"].as_str().expect("object schema"),
                    frame["content_digest"].as_str().expect("object digest"),
                ));
                authorities.clear();
            }
            Some("object_authority") => {
                assert!(in_object);
                let bytes = &record[1..record.len() - 1];
                if let Some(prior) = authorities.last() {
                    assert!(prior.as_slice() < bytes);
                }
                authorities.push(bytes.to_vec());
            }
            Some("object_end") => {
                assert!(in_object);
                assert!(!authorities.is_empty());
                in_object = false;
            }
            _ => {}
        }
    }
    assert!(!in_object);
    assert!(identities.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        identities.len(),
        identities.iter().copied().collect::<BTreeSet<_>>().len()
    );
}

fn join_records(records: &[Vec<u8>]) -> Vec<u8> {
    records.concat()
}

fn swap_adjacent_groups(
    records: &[Vec<u8>],
    first: &std::ops::Range<usize>,
    second: &std::ops::Range<usize>,
) -> Vec<Vec<u8>> {
    assert_eq!(first.end, second.start);
    let mut swapped = Vec::with_capacity(records.len());
    swapped.extend_from_slice(&records[..first.start]);
    swapped.extend_from_slice(&records[second.clone()]);
    swapped.extend_from_slice(&records[first.clone()]);
    swapped.extend_from_slice(&records[second.end..]);
    swapped
}

fn join_frames(frames: &[serde_json::Value]) -> Vec<u8> {
    frames
        .iter()
        .flat_map(|frame| {
            let mut record = Vec::new();
            record.push(0x1e);
            record.extend(serde_json::to_vec(frame).expect("serialize frame"));
            record.push(b'\n');
            record
        })
        .collect()
}

async fn assert_invalid(bytes: &[u8], reason: &str) {
    let error = verify_portable_run_export_stream(bytes, &stream_ref(bytes))
        .await
        .expect_err(reason);
    assert_eq!(error.kind(), ReplayErrorKind::InvalidExport, "{reason}");
}

fn stream_ref(bytes: &[u8]) -> ContentRef {
    let contract = RecoverabilityContractV2::embedded().expect("recoverability contract");
    ContentRef::new(
        contract
            .schema_id("mfm.portable-run-export-stream.v1")
            .expect("stream schema")
            .clone(),
        contract.raw_content_digest(bytes),
    )
    .expect("stream content ref")
}
