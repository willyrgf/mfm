use std::collections::BTreeSet;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use mfm_canonical::{CanonicalBytes, PlainCanonicalJsonBytes, RecoverabilityContract};
use mfm_ids::{AppendRequestId, RunId, StableId, StoreEpoch, StoreScopeId};
use mfm_program::{QualifiedCandidateIdentity, QualifiedProgramRegistry};
use mfm_qualified_run_test_support::{
    CandidateCallbackCounts, PreparedQualifiedRun, QualifiedRunFixture,
};
use mfm_replay::trace_export::{
    verify_portable_run_export_stream, write_portable_run_export_stream, ExportKind,
    PortableRunExportMetadata, VerifiedExportStream,
};
use mfm_replay::{
    compare_current, required_export_source_run_ids, verify_recorded_history, ReplayErrorKind,
};
use mfm_runtime::{AuthorizedAdmissionPlan, DriveOutcome, Runtime};
use mfm_store::{
    open_in_memory, AdmissionMaterial, AppendOutcome, ExistingRunAppendMaterial,
    InMemoryRunJournalBackend, NewlyAppended, ObjectGraphProposal, ProducedObjectRoot,
    ProducedOutputSlot, QualifiedRunStore, RunAccessAuthorityIssuer, RunHistoryReader,
    SettlementMaterial, StoreIdentity, TransitionMaterial,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct ClosedQualifiedRun {
    reader: RunHistoryReader<InMemoryRunJournalBackend>,
    issuer: RunAccessAuthorityIssuer,
    fixture: QualifiedRunFixture,
    registry: Arc<QualifiedProgramRegistry>,
    run_id: RunId,
}

impl ClosedQualifiedRun {
    async fn portable_stream(&self) -> (Vec<u8>, PortableRunExportMetadata) {
        let export_authority = self
            .issuer
            .authorize_export(self.fixture.tenant_scope_id().clone(), self.run_id.clone());
        assert!(
            required_export_source_run_ids(&self.reader, &export_authority)
                .await
                .expect("discover source-free export closure")
                .is_empty()
        );
        let mut bytes = Vec::new();
        let metadata = write_portable_run_export_stream(
            &self.reader,
            &export_authority,
            &[],
            ExportKind::Semantic,
            &mut bytes,
        )
        .await
        .expect("export genuine semantic history");
        (bytes, metadata)
    }

    async fn portable_history(&self) -> VerifiedExportStream {
        let (bytes, metadata) = self.portable_stream().await;
        bind_portable_history(self, bytes, metadata).await
    }
}

fn store_identity(discriminator: u8) -> StoreIdentity {
    StoreIdentity::new(
        StoreScopeId::new(format!(
            "{}{}",
            StoreScopeId::PREFIX,
            format!("{discriminator:02x}").repeat(16)
        ))
        .expect("valid store scope"),
        StoreEpoch::new(1),
    )
}

fn provision(store: &QualifiedRunStore<InMemoryRunJournalBackend>, fixture: &QualifiedRunFixture) {
    store
        .provision_configured_value(
            fixture.configured_binding().clone(),
            fixture.configured_bytes().to_vec(),
        )
        .expect("provision genuine configured value");
}

async fn prepare(
    reader: &RunHistoryReader<InMemoryRunJournalBackend>,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &QualifiedRunFixture,
    registry: Arc<QualifiedProgramRegistry>,
) -> PreparedQualifiedRun {
    fixture
        .prepare_on(reader, issuer, registry)
        .await
        .expect("prepare genuine qualified run")
}

fn admission_plan(
    fixture: &QualifiedRunFixture,
    prepared: PreparedQualifiedRun,
) -> (Arc<QualifiedProgramRegistry>, AuthorizedAdmissionPlan) {
    let (registry, authority, append_request_id, artifacts, input, configured, sources) =
        prepared.into_parts();
    let plan = AuthorizedAdmissionPlan::new(
        authority,
        append_request_id,
        fixture.entry_point_id().clone(),
        fixture.entry_point_operation_id().clone(),
        fixture.invocation_identity().clone(),
        artifacts,
        input,
        configured,
        sources,
    );
    (registry, plan)
}

async fn close_with_runtime(
    identity_discriminator: u8,
    fixture_discriminator: u8,
) -> ClosedQualifiedRun {
    let identity = store_identity(identity_discriminator);
    let fixture =
        QualifiedRunFixture::for_store(identity, fixture_discriminator).expect("qualified fixture");
    close_fixture_with_runtime(fixture, None).await.0
}

async fn close_with_candidate(
    fixture: QualifiedRunFixture,
    candidate: &QualifiedRunFixture,
) -> (ClosedQualifiedRun, Arc<QualifiedProgramRegistry>) {
    let (run, registry) = close_fixture_with_runtime(fixture, Some(candidate)).await;
    (
        run,
        registry.expect("candidate registry was requested before the history split"),
    )
}

async fn close_fixture_with_runtime(
    fixture: QualifiedRunFixture,
    candidate: Option<&QualifiedRunFixture>,
) -> (ClosedQualifiedRun, Option<Arc<QualifiedProgramRegistry>>) {
    let (store, issuer) = open_in_memory(fixture.store_identity().clone());
    provision(&store, &fixture);
    if let Some(candidate) = candidate {
        provision(&store, candidate);
    }
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify genuine run");
    let candidate_registry = match candidate {
        Some(candidate) => Some(
            candidate
                .qualify_on(&store, &issuer)
                .await
                .expect("qualify candidate registry"),
        ),
        None => None,
    };
    let (writer, reader) = store.split();
    let prepared = prepare(&reader, &issuer, &fixture, Arc::clone(&registry)).await;
    let (_, plan) = admission_plan(&fixture, prepared);
    let runtime = Runtime::new(writer, Arc::clone(&registry));
    let run_id = runtime
        .admit(plan)
        .await
        .expect("admit genuine qualified run")
        .run_id()
        .clone();

    for _ in 0..8 {
        let outcome = runtime
            .drive_once(issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone()))
            .await
            .expect("drive genuine qualified run");
        match outcome {
            DriveOutcome::Advanced { .. } => {}
            DriveOutcome::Closed { .. } => {
                return (
                    ClosedQualifiedRun {
                        reader,
                        issuer,
                        fixture,
                        registry,
                        run_id,
                    },
                    candidate_registry,
                );
            }
            DriveOutcome::Waiting { reason, .. } => {
                panic!("source-free genuine run unexpectedly waited: {reason:?}");
            }
        }
    }
    panic!("source-free genuine run did not close within its certified node bound");
}

async fn close_panicking_history_manually(
    identity_discriminator: u8,
    fixture_discriminator: u8,
) -> ClosedQualifiedRun {
    let identity = store_identity(identity_discriminator);
    let (store, issuer) = open_in_memory(identity.clone());
    let fixture = QualifiedRunFixture::for_store(identity, fixture_discriminator)
        .expect("qualified fixture")
        .with_panicking_state_callback()
        .expect("panicking fixture");
    provision(&store, &fixture);
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify panicking fixture");
    let (writer, reader) = store.split();
    let prepared = prepare(&reader, &issuer, &fixture, Arc::clone(&registry)).await;
    let (registry, authority, append_request_id, artifacts, input, configured, sources) =
        prepared.into_parts();
    let append = writer
        .prepare_admission(
            &authority,
            append_request_id,
            AdmissionMaterial::new(
                artifacts,
                input,
                &configured,
                registry.admitted_support(),
                &sources,
            ),
        )
        .expect("prepare genuine admission");
    let outcome = writer
        .append_admission(&authority, append)
        .await
        .expect("append genuine qualified admission");
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
        panic!("genuine admission was not newly committed");
    };
    let run_id = admitted.run_id().clone();

    let drive_authority = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = writer
        .load_for_drive(&drive_authority)
        .await
        .expect("load genuine admitted history")
        .verify_recorded_history()
        .expect("verify genuine admitted history");
    let [node] = view.certified_spec().nodes() else {
        panic!("panicking fixture must certify exactly one node");
    };
    let [output_slot] = node.settlement_contract().output_slots() else {
        panic!("panicking fixture must certify exactly one output");
    };
    let frame = writer
        .prepare_frame(&drive_authority, &view, node.node_id())
        .await
        .expect("prepare verified historical frame");
    let configured: serde_json::Value =
        serde_json::from_slice(fixture.configured_bytes()).expect("fixture configuration JSON");
    let output = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::json!({
            "value": configured
                .get("value")
                .and_then(serde_json::Value::as_u64)
                .expect("fixture numeric value")
        })
        .to_string(),
    )
    .expect("canonical historical output");
    let append = writer
        .prepare_append(
            &drive_authority,
            &view,
            AppendRequestId::new(format!(
                "replay-fixture/manual-settlement/{fixture_discriminator:02x}"
            ))
            .expect("manual settlement append identity"),
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
        .expect("prepare genuine recorded settlement");
    assert!(matches!(
        writer
            .append(&drive_authority, append)
            .await
            .expect("append genuine recorded settlement"),
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
    ));

    ClosedQualifiedRun {
        reader,
        issuer,
        fixture,
        registry,
        run_id,
    }
}

async fn bind_portable_history(
    run: &ClosedQualifiedRun,
    bytes: Vec<u8>,
    metadata: PortableRunExportMetadata,
) -> VerifiedExportStream {
    assert_stream_framing(&bytes);
    let contract = RecoverabilityContract::embedded().expect("recoverability contract");
    assert_eq!(
        metadata.content_digest(),
        &contract.raw_content_digest(&bytes)
    );
    assert_eq!(
        metadata.schema_id(),
        contract
            .schema_id("mfm.portable-run-export-stream.v1")
            .expect("portable stream schema")
    );
    let offline = verify_portable_run_export_stream(bytes.as_slice(), metadata.content_ref())
        .await
        .expect("independently verify generated portable export");
    assert_eq!(offline.run_id(), &run.run_id);
    assert_eq!(offline.export_kind(), ExportKind::Semantic);
    let replay_authority = run
        .issuer
        .authorize_replay(run.fixture.tenant_scope_id().clone(), run.run_id.clone());
    let verified = verify_recorded_history(&run.reader, &replay_authority)
        .await
        .expect("verify callback-free recorded history");
    verified
        .verify_export_stream(bytes.as_slice(), metadata.content_ref())
        .await
        .expect("bind semantic export to verified history")
}

fn assert_stream_framing(bytes: &[u8]) {
    assert_eq!(bytes.first(), Some(&0x1e));
    let mut kinds = Vec::new();
    for record in bytes.split_inclusive(|byte| *byte == b'\n') {
        assert_eq!(record.first(), Some(&0x1e));
        assert_eq!(record.last(), Some(&b'\n'));
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(&record[1..record.len() - 1])
                .expect("canonical stream frame");
        let frame: serde_json::Value =
            serde_json::from_slice(canonical.as_bytes()).expect("stream frame JSON");
        let kind = frame
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .expect("frame kind");
        if kind == "chunk" {
            let decoded = CanonicalBytes::from_base64url_no_pad(
                frame
                    .get("bytes")
                    .and_then(serde_json::Value::as_str)
                    .expect("chunk bytes"),
            )
            .expect("canonical chunk");
            assert!(!decoded.as_bytes().is_empty());
            assert!(decoded.as_bytes().len() <= 65_536);
        }
        if matches!(kind, "header" | "end") {
            assert!(
                frame
                    .as_object()
                    .expect("frame object")
                    .keys()
                    .all(|key| !key.contains("digest")),
                "stream must not carry its own digest"
            );
        }
        kinds.push(kind.to_owned());
    }
    assert_eq!(kinds.first().map(String::as_str), Some("header"));
    assert_eq!(kinds.last().map(String::as_str), Some("end"));
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| kind.as_str() == "header")
            .count(),
        1
    );
    assert_eq!(
        kinds.iter().filter(|kind| kind.as_str() == "end").count(),
        1
    );
    assert_eq!(
        kinds.into_iter().collect::<BTreeSet<_>>(),
        [
            "chunk",
            "commit_begin",
            "end",
            "header",
            "object_authority",
            "object_begin",
            "object_end",
            "record_begin",
            "run_begin",
            "run_end",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
}

fn stream_ref(bytes: &[u8]) -> mfm_ids::ContentRef {
    let contract = RecoverabilityContract::embedded().expect("recoverability contract");
    mfm_ids::ContentRef::new(
        contract
            .schema_id("mfm.portable-run-export-stream.v1")
            .expect("portable stream schema")
            .clone(),
        contract.raw_content_digest(bytes),
    )
    .expect("portable stream reference")
}

fn parse_result(result: &mfm_replay::CanonicalReplayResult) -> serde_json::Value {
    serde_json::from_slice(result.as_bytes()).expect("canonical candidate result JSON")
}

fn assert_content_ref(value: &serde_json::Value, expected: &mfm_ids::ContentRef) {
    assert_eq!(
        value.get("schema_id").and_then(serde_json::Value::as_str),
        Some(expected.schema_id().as_str())
    );
    assert_eq!(
        value
            .get("content_digest")
            .and_then(serde_json::Value::as_str),
        Some(expected.content_digest().as_str())
    );
}

fn assert_candidate_identities(
    result: &serde_json::Value,
    admitted: &QualifiedProgramRegistry,
    candidate: &QualifiedCandidateIdentity,
) {
    let report = result.get("report").expect("candidate report");
    assert_content_ref(
        report
            .get("admitted_executable_identity_ref")
            .expect("admitted executable ref"),
        admitted.executable_identity_ref(),
    );
    for (field, expected) in [
        (
            "candidate_executable_identity_ref",
            candidate.candidate_executable_identity_ref(),
        ),
        (
            "candidate_planning_profile_ref",
            candidate.planning_profile_ref(),
        ),
        (
            "candidate_planner_contract_ref",
            candidate.planner_contract_ref(),
        ),
        (
            "candidate_planner_implementation_ref",
            candidate.planner_implementation_ref(),
        ),
        (
            "candidate_state_implementation_manifest_ref",
            candidate.state_implementation_manifest_ref(),
        ),
        (
            "candidate_capability_binding_manifest_ref",
            candidate.capability_binding_manifest_ref(),
        ),
    ] {
        assert_content_ref(
            report.get(field).expect("candidate identity field"),
            expected,
        );
    }
    assert_content_ref(
        result
            .get("candidate_executable_identity_ref")
            .expect("top-level candidate executable ref"),
        candidate.candidate_executable_identity_ref(),
    );
}

#[tokio::test]
async fn portable_stream_is_deterministic_and_rejects_structural_tampering() {
    let run = close_with_runtime(0x40, 0x41).await;
    let (first, first_metadata) = run.portable_stream().await;
    let (second, second_metadata) = run.portable_stream().await;
    assert_eq!(first, second);
    assert_eq!(first_metadata, second_metadata);
    assert_stream_framing(&first);

    let error = verify_portable_run_export_stream(
        FailAtEofReader::new(first.clone()),
        first_metadata.content_ref(),
    )
    .await
    .expect_err("EOF probe I/O failure after a valid end frame");
    assert_eq!(error.kind(), ReplayErrorKind::ExportStreamIo);

    let mut wrong_offset = first.clone();
    let needle = br#""offset":"0""#;
    let index = wrong_offset
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("stream chunk offset");
    wrong_offset[index + needle.len() - 2] = b'1';
    let error =
        verify_portable_run_export_stream(wrong_offset.as_slice(), &stream_ref(&wrong_offset))
            .await
            .expect_err("noncontiguous chunk offset");
    assert_eq!(error.kind(), ReplayErrorKind::InvalidExport);

    let mut wrong_prefix = first.clone();
    wrong_prefix[0] = b' ';
    let error =
        verify_portable_run_export_stream(wrong_prefix.as_slice(), &stream_ref(&wrong_prefix))
            .await
            .expect_err("wrong record prefix");
    assert_eq!(error.kind(), ReplayErrorKind::InvalidExport);

    let truncated = &first[..first.len() - 1];
    let error = verify_portable_run_export_stream(truncated, &stream_ref(truncated))
        .await
        .expect_err("premature clean EOF");
    assert_eq!(error.kind(), ReplayErrorKind::InvalidExport);

    let error = verify_portable_run_export_stream(first.as_slice(), &stream_ref(b"different"))
        .await
        .expect_err("external digest mismatch");
    assert_eq!(error.kind(), ReplayErrorKind::InvalidExport);

    let mut short_writer = ShortWriter::new(3);
    let export_authority = run
        .issuer
        .authorize_export(run.fixture.tenant_scope_id().clone(), run.run_id.clone());
    let short_metadata = write_portable_run_export_stream(
        &run.reader,
        &export_authority,
        &[],
        ExportKind::Semantic,
        &mut short_writer,
    )
    .await
    .expect("write through deterministic short writes");
    assert!(short_writer.flushed);
    assert!(short_writer.shutdown);
    assert_eq!(short_writer.bytes, first);
    assert_eq!(short_metadata, first_metadata);

    let first_record_end = first
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("header line feed")
        + 1;
    let last_record_start = first
        .iter()
        .rposition(|byte| *byte == 0x1e)
        .expect("terminal record separator");
    let terminal = &first[last_record_start..];
    let duplicate_field_record =
        raw_record(br#"{"kind":"end","kind":"end","version":"mfm.portable-run-export-frame.v1"}"#);
    let unknown_field_record = raw_record(
        br#"{"kind":"end","unknown":true,"version":"mfm.portable-run-export-frame.v1"}"#,
    );
    let non_jcs_record =
        raw_record(br#"{"version":"mfm.portable-run-export-frame.v1","kind":"end"}"#);
    let malformed = [
        (
            "byte-order-mark",
            [b"\xef\xbb\xbf".as_slice(), first.as_slice()].concat(),
        ),
        (
            "crlf",
            [
                &first[..first_record_end - 1],
                b"\r\n".as_slice(),
                &first[first_record_end..],
            ]
            .concat(),
        ),
        (
            "blank-record",
            [b"\n".as_slice(), first.as_slice()].concat(),
        ),
        (
            "json-whitespace",
            [&first[..1], b" ".as_slice(), &first[1..]].concat(),
        ),
        (
            "duplicate-field",
            [
                &first[..last_record_start],
                duplicate_field_record.as_slice(),
            ]
            .concat(),
        ),
        (
            "unknown-field",
            [&first[..last_record_start], unknown_field_record.as_slice()].concat(),
        ),
        (
            "non-jcs-order",
            [&first[..last_record_start], non_jcs_record.as_slice()].concat(),
        ),
        (
            "duplicate-header",
            [&first[..first_record_end], first.as_slice()].concat(),
        ),
        ("frame-after-end", [first.as_slice(), terminal].concat()),
        (
            "bytes-after-end",
            [first.as_slice(), b"x".as_slice()].concat(),
        ),
    ];
    for (name, malformed) in malformed {
        let error =
            verify_portable_run_export_stream(malformed.as_slice(), &stream_ref(&malformed))
                .await
                .unwrap_err();
        assert_eq!(
            error.kind(),
            ReplayErrorKind::InvalidExport,
            "{name} must be rejected"
        );
    }
}

fn raw_record(json: &[u8]) -> Vec<u8> {
    [b"\x1e".as_slice(), json, b"\n".as_slice()].concat()
}

struct ShortWriter {
    bytes: Vec<u8>,
    max_write: usize,
    flushed: bool,
    shutdown: bool,
}

struct FailAtEofReader {
    bytes: Vec<u8>,
    position: usize,
}

impl FailAtEofReader {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, position: 0 }
    }
}

impl AsyncRead for FailAtEofReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.position == self.bytes.len() {
            return Poll::Ready(Err(io::Error::other("sentinel EOF probe failure")));
        }
        let count = buffer
            .remaining()
            .min(self.bytes.len().saturating_sub(self.position));
        let end = self.position + count;
        buffer.put_slice(&self.bytes[self.position..end]);
        self.position = end;
        Poll::Ready(Ok(()))
    }
}

impl ShortWriter {
    fn new(max_write: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_write,
            flushed: false,
            shutdown: false,
        }
    }
}

impl AsyncWrite for ShortWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.shutdown {
            return Poll::Ready(Err(io::Error::other("write after shutdown")));
        }
        let count = buffer.len().min(self.max_write);
        self.bytes.extend_from_slice(&buffer[..count]);
        Poll::Ready(Ok(count))
    }

    fn poll_flush(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.flushed = true;
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.flushed {
            return Poll::Ready(Err(io::Error::other("shutdown before flush")));
        }
        self.shutdown = true;
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn exact_current_candidate_agrees_and_binds_every_identity() {
    let run = close_with_runtime(0x31, 0x41).await;
    let historical = run.portable_history().await;
    let candidate = run
        .registry
        .select_current_candidate(run.fixture.entry_point_operation_id())
        .expect("select exact current candidate");

    let result = compare_current(&historical, &run.registry).expect("compare exact candidate");
    let result = parse_result(&result);
    assert_eq!(
        result.get("kind").and_then(serde_json::Value::as_str),
        Some("candidate_comparison")
    );
    assert_candidate_identities(&result, &run.registry, &candidate);
    let report = result.get("report").expect("candidate report");
    assert_eq!(
        report.get("plan").and_then(serde_json::Value::as_str),
        Some("agrees")
    );
    let transitions = report
        .get("transitions")
        .and_then(serde_json::Value::as_array)
        .expect("candidate transition results");
    assert_eq!(transitions.len(), 1);
    assert_eq!(
        transitions[0]
            .get("result")
            .and_then(serde_json::Value::as_str),
        Some("agrees")
    );
}

#[tokio::test]
async fn absent_current_operation_is_candidate_unavailable() {
    let identity = store_identity(0x32);
    let fixture =
        QualifiedRunFixture::for_store(identity.clone(), 0x42).expect("historical fixture");
    let unavailable = QualifiedRunFixture::for_store_with_operation(
        identity,
        0x42,
        StableId::new("mfm.fixture/unavailable-replay-operation").expect("operation"),
    )
    .expect("unavailable candidate fixture");
    let (run, registry) = close_with_candidate(fixture, &unavailable).await;
    let historical = run.portable_history().await;

    let error =
        compare_current(&historical, &registry).expect_err("candidate operation must be absent");
    assert_eq!(error.kind(), ReplayErrorKind::CandidateUnavailable);
    assert_eq!(error.code(), "MFM_REPLAY_CANDIDATE_UNAVAILABLE");
}

#[tokio::test]
async fn panicking_candidate_is_redaction_safe_execution_failure() {
    let run = close_panicking_history_manually(0x33, 0x43).await;
    let historical = run.portable_history().await;

    let error =
        compare_current(&historical, &run.registry).expect_err("candidate callback must panic");
    assert_eq!(error.kind(), ReplayErrorKind::CandidateExecutionFailed);
    assert_eq!(error.code(), "MFM_REPLAY_CANDIDATE_EXECUTION_FAILED");
    assert!(!error
        .to_string()
        .contains("qualified fixture state callback"));
    assert!(!format!("{error:?}").contains("qualified fixture state callback"));
}

#[tokio::test]
async fn candidate_frame_decode_integrity_is_comparison_integrity_failure() {
    let identity = store_identity(0x34);
    let fixture =
        QualifiedRunFixture::for_store(identity.clone(), 0x44).expect("historical fixture");
    let integrity_failing = QualifiedRunFixture::for_store(identity, 0x44)
        .expect("candidate fixture")
        .with_integrity_failing_state_callback();
    let (run, registry) = close_with_candidate(fixture, &integrity_failing).await;
    let historical = run.portable_history().await;

    let error = compare_current(&historical, &registry)
        .expect_err("candidate typed frame decode must fail integrity");
    assert_eq!(error.kind(), ReplayErrorKind::ComparisonIntegrityFailed);
    assert_eq!(error.code(), "MFM_REPLAY_COMPARISON_INTEGRITY_FAILED");
}

#[tokio::test]
async fn global_not_comparable_skips_every_candidate_callback() {
    let identity = store_identity(0x35);
    let fixture = QualifiedRunFixture::for_store(identity.clone(), 0x45)
        .expect("historical fixture")
        .with_two_node_chain();
    let incompatible = QualifiedRunFixture::for_store(identity, 0x45)
        .expect("candidate fixture")
        .with_two_node_divergent_candidate()
        .with_incompatible_planning_profile();
    incompatible.reset_candidate_callback_counts();
    let (run, registry) = close_with_candidate(fixture, &incompatible).await;
    let historical = run.portable_history().await;

    let result =
        compare_current(&historical, &registry).expect("incompatible candidate must not execute");
    let result = parse_result(&result);
    let report = result.get("report").expect("candidate report");
    assert_eq!(
        report.get("plan").and_then(serde_json::Value::as_str),
        Some("not_comparable")
    );
    let transitions = report
        .get("transitions")
        .and_then(serde_json::Value::as_array)
        .expect("candidate transition results");
    assert_eq!(transitions.len(), 2);
    assert!(transitions.iter().all(|transition| {
        transition.get("result").and_then(serde_json::Value::as_str) == Some("not_comparable")
    }));
    assert_eq!(
        incompatible.candidate_callback_counts(),
        CandidateCallbackCounts {
            first_state: 0,
            second_state: 0,
        }
    );
}

#[tokio::test]
async fn transition_comparisons_use_recorded_frames_not_prior_candidate_outputs() {
    let identity = store_identity(0x36);
    let fixture = QualifiedRunFixture::for_store(identity.clone(), 0x46)
        .expect("historical fixture")
        .with_two_node_chain();
    let divergent = QualifiedRunFixture::for_store(identity, 0x46)
        .expect("candidate fixture")
        .with_two_node_divergent_candidate();
    divergent.reset_candidate_callback_counts();
    let (run, registry) = close_with_candidate(fixture, &divergent).await;
    let historical = run.portable_history().await;

    let result = compare_current(&historical, &registry).expect("compare divergent candidate");
    let result = parse_result(&result);
    let report = result.get("report").expect("candidate report");
    assert_eq!(
        report.get("plan").and_then(serde_json::Value::as_str),
        Some("agrees")
    );
    let transition_results = report
        .get("transitions")
        .and_then(serde_json::Value::as_array)
        .expect("candidate transition results")
        .iter()
        .map(|transition| {
            transition
                .get("result")
                .and_then(serde_json::Value::as_str)
                .expect("transition verdict")
        })
        .collect::<Vec<_>>();
    assert_eq!(transition_results, ["differs", "agrees"]);
    assert_eq!(
        divergent.candidate_callback_counts(),
        CandidateCallbackCounts {
            first_state: 1,
            second_state: 1,
        }
    );
}
