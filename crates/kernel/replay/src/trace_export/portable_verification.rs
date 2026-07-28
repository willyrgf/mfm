use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::{RecordId, StoreEpoch};
use mfm_journal::v1::{
    CandidateRecordEnvelope, CommitEnvelope, JournalPredecessorFields, RecordHashPreimage,
    RecordIdPreimage, RunPhase, ValueRef,
};
use mfm_store::v1::{
    verify_offline_recorded_material, CommittedJournalCommit, CommittedJournalRecord,
    StoreIdentity, UntrustedObjectPayload, VerifiedRunView,
};

use super::*;

#[derive(Default)]
struct NativeRunMembers<'a> {
    commits: BTreeMap<u64, &'a ParsedMember>,
    records: BTreeMap<(u64, u32), &'a ParsedMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PayloadIdentity {
    schema_id: SchemaId,
    content_digest: ContentDigest,
}

struct DecodedRun {
    store_epoch: StoreEpoch,
    commits: Vec<CommittedJournalCommit>,
    object_refs: BTreeMap<Vec<u8>, ValueRef>,
}

pub(super) struct VerifiedPortableRun {
    pub(super) view: VerifiedRunView,
}

pub(super) struct VerifiedPortableGraph {
    pub(super) runs: Vec<VerifiedPortableRun>,
}

pub(super) fn verify_member_graph(
    members: &[ParsedMember],
    expected_runs: &[RunId],
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    coordinate: &ExportCoordinate,
) -> Result<VerifiedPortableGraph> {
    if expected_runs.is_empty() || expected_runs.len() > MAX_RUN_INDEX + 1 {
        return Err(ReplayError::InvalidExport);
    }

    let mut runs = (0..expected_runs.len())
        .map(|_| NativeRunMembers::default())
        .collect::<Vec<_>>();
    let mut objects_by_path = (0..expected_runs.len())
        .map(|_| BTreeMap::new())
        .collect::<Vec<BTreeMap<usize, &ParsedMember>>>();
    let mut objects_by_payload = BTreeMap::<PayloadIdentity, &ParsedMember>::new();
    for member in members {
        let run_index = match member.parsed_path {
            ParsedMemberPath::Commit { run_index, .. }
            | ParsedMemberPath::Record { run_index, .. }
            | ParsedMemberPath::Object { run_index, .. } => run_index,
        };
        let run = runs.get_mut(run_index).ok_or(ReplayError::InvalidExport)?;
        match member.parsed_path {
            ParsedMemberPath::Commit { run_sequence, .. } => {
                if run.commits.insert(run_sequence, member).is_some() {
                    return Err(ReplayError::InvalidExport);
                }
            }
            ParsedMemberPath::Record {
                run_sequence,
                ordinal,
                ..
            } => {
                if run
                    .records
                    .insert((run_sequence, ordinal), member)
                    .is_some()
                {
                    return Err(ReplayError::InvalidExport);
                }
            }
            ParsedMemberPath::Object { object_index, .. } => {
                if objects_by_path[run_index]
                    .insert(object_index, member)
                    .is_some()
                {
                    return Err(ReplayError::InvalidExport);
                }
                let identity = PayloadIdentity {
                    schema_id: member.schema_id.clone(),
                    content_digest: member.content_digest.clone(),
                };
                if objects_by_payload.insert(identity, member).is_some() {
                    return Err(ReplayError::InvalidExport);
                }
            }
        }
    }
    if objects_by_path
        .iter()
        .any(|objects| !objects.keys().copied().eq(0..objects.len()))
    {
        return Err(ReplayError::InvalidExport);
    }

    let mut decoded_runs = Vec::with_capacity(runs.len());
    let mut observed_store_epoch = None;
    for (run, expected_run_id) in runs.iter().zip(expected_runs) {
        let decoded = decode_run_members(run, expected_run_id, store_scope_id)?;
        match observed_store_epoch {
            None => observed_store_epoch = Some(decoded.store_epoch),
            Some(expected) if expected == decoded.store_epoch => {}
            Some(_) => return Err(ReplayError::InvalidExport),
        }
        decoded_runs.push(decoded);
    }

    let mut expected_ref_runs = BTreeMap::<Vec<u8>, BTreeSet<usize>>::new();
    for (run_index, run) in decoded_runs.iter().enumerate() {
        for value_ref in run.object_refs.values() {
            expected_ref_runs
                .entry(value_ref.as_bytes().to_vec())
                .or_default()
                .insert(run_index);
        }
    }
    validate_object_members(&objects_by_payload, &objects_by_path, &expected_ref_runs)?;

    let store_epoch = observed_store_epoch.ok_or(ReplayError::InvalidExport)?;
    let mut verified = Vec::with_capacity(decoded_runs.len());
    for (run_index, (decoded, run_id)) in decoded_runs.into_iter().zip(expected_runs).enumerate() {
        let payloads = payloads_for_run(&objects_by_payload, &expected_ref_runs, run_index)?;
        let view = verify_offline_recorded_material(
            StoreIdentity::new(store_scope_id.clone(), store_epoch),
            tenant_scope_id.clone(),
            run_id.clone(),
            decoded.commits,
            payloads,
        )
        .map_err(|error| store_error(&error))?;
        if run_index != 0
            && (view.run_phase() != RunPhase::Closed
                || view.journal_head() != view.semantic_head()
                || view.semantic_closure().is_none())
        {
            return Err(ReplayError::InvalidExport);
        }
        verified.push(VerifiedPortableRun { view });
    }

    validate_verified_source_closure(&verified)?;
    validate_export_coordinate(
        coordinate,
        expected_runs.first().ok_or(ReplayError::InvalidExport)?,
        verified.first().ok_or(ReplayError::InvalidExport)?,
    )?;
    Ok(VerifiedPortableGraph { runs: verified })
}

fn decode_run_members(
    run: &NativeRunMembers<'_>,
    expected_run_id: &RunId,
    expected_store_scope_id: &StoreScopeId,
) -> Result<DecodedRun> {
    if run.commits.is_empty()
        || !run
            .commits
            .keys()
            .copied()
            .eq(1..=u64::try_from(run.commits.len()).map_err(|_| ReplayError::InvalidExport)?)
        || run
            .records
            .keys()
            .any(|(sequence, _)| !run.commits.contains_key(sequence))
    {
        return Err(ReplayError::InvalidExport);
    }

    let mut store_epoch = None;
    let mut commits = Vec::with_capacity(run.commits.len());
    let mut object_refs = BTreeMap::new();
    for (run_sequence, member) in &run.commits {
        let envelope = CommitEnvelope::strict_decode(&member.bytes)?;
        if member.schema_id != *envelope.schema_id() {
            return Err(ReplayError::InvalidExport);
        }
        let envelope_fields = envelope.fields()?;
        if envelope_fields.core.run_sequence != *run_sequence
            || envelope_fields.core.run_id != *expected_run_id
            || envelope_fields.core.store_scope_id != *expected_store_scope_id
        {
            return Err(ReplayError::InvalidExport);
        }
        if *run_sequence == 1 {
            let JournalPredecessorFields::Genesis {
                store_scope_id,
                store_epoch: genesis_store_epoch,
                run_id,
                ..
            } = envelope_fields.core.predecessor.fields()?
            else {
                return Err(ReplayError::InvalidExport);
            };
            if store_scope_id != *expected_store_scope_id || run_id != *expected_run_id {
                return Err(ReplayError::InvalidExport);
            }
            store_epoch = Some(genesis_store_epoch);
        }

        // Admission intents define the complete reachable authority set; path bindings are only
        // record-field projections and may omit retained values used through other relationships.
        for intent in &envelope_fields.core.artifact_admission_intents {
            let value_ref = intent.fields()?.value_ref;
            object_refs
                .entry(value_ref.as_bytes().to_vec())
                .or_insert(value_ref);
        }

        let record_members = run
            .records
            .range((*run_sequence, 0)..=(*run_sequence, u32::MAX))
            .collect::<Vec<_>>();
        if record_members.is_empty()
            || record_members
                .iter()
                .enumerate()
                .any(|(ordinal, ((_, actual), _))| usize::try_from(*actual).ok() != Some(ordinal))
        {
            return Err(ReplayError::InvalidExport);
        }
        let mut records = Vec::with_capacity(record_members.len());
        for ((_, ordinal), record_member) in record_members {
            let candidate = CandidateRecordEnvelope::strict_decode(&record_member.bytes)?;
            if record_member.schema_id != *candidate.schema_id()
                || candidate.fields()?.ordinal != *ordinal
            {
                return Err(ReplayError::InvalidExport);
            }
            let record_hash = RecordHashPreimage::from_candidate(&candidate)?.record_hash()?;
            let record_id: RecordId = RecordIdPreimage::new(
                expected_store_scope_id,
                expected_run_id,
                *run_sequence,
                *ordinal,
                &record_hash,
            )?
            .record_id()?;
            records.push(CommittedJournalRecord::from_persisted(
                record_id,
                record_hash,
                candidate,
            ));
        }
        commits.push(CommittedJournalCommit::from_persisted(envelope, records));
    }

    Ok(DecodedRun {
        store_epoch: store_epoch.ok_or(ReplayError::InvalidExport)?,
        commits,
        object_refs,
    })
}

fn validate_object_members(
    objects_by_payload: &BTreeMap<PayloadIdentity, &ParsedMember>,
    objects_by_path: &[BTreeMap<usize, &ParsedMember>],
    expected_ref_runs: &BTreeMap<Vec<u8>, BTreeSet<usize>>,
) -> Result<()> {
    let mut supplied_refs = BTreeSet::new();
    let mut expected_order = BTreeMap::<usize, Vec<(Vec<u8>, usize)>>::new();
    for member in objects_by_payload.values() {
        let value_refs = member
            .value_refs
            .as_ref()
            .ok_or(ReplayError::InvalidExport)?;
        UntrustedObjectPayload::new(
            member.schema_id.clone(),
            member.content_digest.clone(),
            member.bytes.clone(),
            value_refs.clone(),
        )
        .map_err(|error| store_error(&error))?;

        let mut owner = None;
        for value_ref in value_refs {
            let key = value_ref.as_bytes().to_vec();
            let runs = expected_ref_runs
                .get(&key)
                .ok_or(ReplayError::InvalidExport)?;
            supplied_refs.insert(key);
            let candidate = runs
                .iter()
                .next()
                .copied()
                .ok_or(ReplayError::InvalidExport)?;
            owner = Some(owner.map_or(candidate, |prior: usize| prior.min(candidate)));
        }
        let owner = owner.ok_or(ReplayError::InvalidExport)?;
        let ParsedMemberPath::Object {
            run_index,
            object_index,
        } = member.parsed_path
        else {
            return Err(ReplayError::InvalidExport);
        };
        if run_index != owner {
            return Err(ReplayError::InvalidExport);
        }
        let sort_key = value_refs
            .iter()
            .filter(|value_ref| {
                expected_ref_runs
                    .get(value_ref.as_bytes())
                    .is_some_and(|runs| runs.contains(&owner))
            })
            .map(|value_ref| value_ref.as_bytes().to_vec())
            .min()
            .ok_or(ReplayError::InvalidExport)?;
        expected_order
            .entry(owner)
            .or_default()
            .push((sort_key, object_index));
    }
    if supplied_refs != expected_ref_runs.keys().cloned().collect() {
        return Err(ReplayError::InvalidExport);
    }
    for entries in expected_order.values_mut() {
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        if entries
            .iter()
            .enumerate()
            .any(|(expected, (_, actual))| expected != *actual)
            || entries.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(ReplayError::InvalidExport);
        }
    }
    for (run_index, objects) in objects_by_path.iter().enumerate() {
        if objects.len() != expected_order.get(&run_index).map_or(0, Vec::len) {
            return Err(ReplayError::InvalidExport);
        }
    }
    Ok(())
}

fn payloads_for_run(
    objects_by_payload: &BTreeMap<PayloadIdentity, &ParsedMember>,
    expected_ref_runs: &BTreeMap<Vec<u8>, BTreeSet<usize>>,
    run_index: usize,
) -> Result<Vec<UntrustedObjectPayload>> {
    objects_by_payload
        .values()
        .filter_map(|member| {
            let value_refs = member
                .value_refs
                .as_ref()?
                .iter()
                .filter(|value_ref| {
                    expected_ref_runs
                        .get(value_ref.as_bytes())
                        .is_some_and(|runs| runs.contains(&run_index))
                })
                .cloned()
                .collect::<Vec<_>>();
            (!value_refs.is_empty()).then(|| {
                UntrustedObjectPayload::new(
                    member.schema_id.clone(),
                    member.content_digest.clone(),
                    member.bytes.clone(),
                    value_refs,
                )
                .map_err(|error| store_error(&error))
            })
        })
        .collect()
}

fn validate_verified_source_closure(runs: &[VerifiedPortableRun]) -> Result<()> {
    let views = runs
        .iter()
        .map(|run| (run.view.run_id().clone(), &run.view))
        .collect::<BTreeMap<_, _>>();
    if views.len() != runs.len() {
        return Err(ReplayError::InvalidExport);
    }
    let mut graph = BTreeMap::<RunId, BTreeSet<RunId>>::new();
    for run in runs {
        let edges = graph.entry(run.view.run_id().clone()).or_default();
        for requirement in run.view.admission_source_requirements().cross_run_sources() {
            edges.insert(requirement.source_run_id().clone());
        }
    }

    let discovered = graph
        .values()
        .flat_map(BTreeSet::iter)
        .cloned()
        .collect::<BTreeSet<_>>();
    let supplied = runs
        .iter()
        .skip(1)
        .map(|run| run.view.run_id().clone())
        .collect::<BTreeSet<_>>();
    if discovered != supplied {
        return Err(ReplayError::InvalidExport);
    }
    reject_source_cycles(&graph, views.keys())?;
    for run in runs {
        for requirement in run.view.admission_source_requirements().cross_run_sources() {
            let source = views
                .get(requirement.source_run_id())
                .ok_or(ReplayError::InvalidExport)?;
            requirement
                .verify_closed_source_view(source)
                .map_err(|_| ReplayError::InvalidExport)?;
        }
    }
    Ok(())
}

fn validate_export_coordinate(
    coordinate: &ExportCoordinate,
    run_id: &RunId,
    root: &VerifiedPortableRun,
) -> Result<()> {
    match coordinate {
        ExportCoordinate::SemanticHead {
            record_ref,
            containing_commit_digest,
        } => {
            if root.view.run_id() != run_id
                || root.view.run_phase() != RunPhase::Open
                || root.view.journal_head() != root.view.semantic_head()
                || record_ref.as_bytes() != root.view.semantic_head_record_ref().as_bytes()
                || &root.view.semantic_head().fields()?.commit_digest != containing_commit_digest
            {
                return Err(ReplayError::InvalidExport);
            }
        }
        ExportCoordinate::SemanticClosure {
            terminal_transition_ref,
            containing_commit_digest,
        } => {
            let closure = root
                .view
                .semantic_closure()
                .ok_or(ReplayError::InvalidExport)?
                .fields()?;
            if root.view.run_id() != run_id
                || root.view.run_phase() != RunPhase::Closed
                || root.view.journal_head() != root.view.semantic_head()
                || terminal_transition_ref.as_bytes() != closure.terminal_transition_ref.as_bytes()
                || containing_commit_digest != &closure.containing_commit_digest
            {
                return Err(ReplayError::InvalidExport);
            }
        }
        ExportCoordinate::JournalHead { journal_head } => {
            if journal_head != root.view.journal_head() {
                return Err(ReplayError::InvalidExport);
            }
        }
    }
    Ok(())
}
