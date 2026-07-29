use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mfm_canonical::{
    CanonicalBytes, CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContractV2,
    ReferenceTerminalKindV2, ValidatedCanonicalValueV2,
};
use mfm_ids::{ContentRef, FieldPath, RunId, SemanticDigest, TenantScopeId};
use mfm_journal::v1::{
    CrossRunSourceRef, ExecutorEnsureResult, ExecutorEnsureResultFields, FactClaimEnvelope,
    InputManifestRef, InputSourceFields, JournalHead, ObservationOutcomeFields, ObservationRef,
    PersistedJournalValue, ProducerBindingFields, Settlement, SettlementFields,
    StateTransitionCommitted, TransitionBodyFields, ValueRef,
};
use mfm_spec::v1::CanonicalExpansionPath;

use super::{
    FoldedTransitionEntry, InspectTrace, Result, RunAccessAuthority, StoreAuthorityContext,
    StoreError, StoreIdentity, VerifiedObservedAccess, VerifiedRunView,
};

const MAX_TRANSITION_TRACE_PAGE_LIMIT: u16 = 500;
const TRANSITION_TRACE_CONTRACT: &str = "mfm.transition-trace.v1";

/// One decoded, bounded request for a head-fixed transition-trace page.
///
/// The application owns opaque cursor encoding. This value carries only the exact physical head
/// recovered from that cursor, the next zero-based transition index, and the reviewed page bound.
pub struct TransitionTracePageRequest {
    at_journal_head: Option<JournalHead>,
    start: u32,
    limit: u16,
}

impl TransitionTracePageRequest {
    /// Constructs one bounded page request.
    ///
    /// Omitting `at_journal_head` fixes the first page to the current physical head. Continuation
    /// requests must supply the exact head returned by that first page.
    pub fn new(at_journal_head: Option<JournalHead>, start: u32, limit: u16) -> Result<Self> {
        if limit == 0 || limit > MAX_TRANSITION_TRACE_PAGE_LIMIT {
            return Err(StoreError::InvalidTracePage { field: "limit" });
        }
        Ok(Self {
            at_journal_head,
            start,
            limit,
        })
    }

    /// Returns the optional physical head decoded from the application cursor.
    pub const fn at_journal_head(&self) -> Option<&JournalHead> {
        self.at_journal_head.as_ref()
    }

    /// Returns the zero-based transition index.
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// Returns the bounded number of transitions requested.
    pub const fn limit(&self) -> u16 {
        self.limit
    }
}

/// Opaque first-phase result for one exact transition-trace page.
///
/// The value retains the verified root view, fixed physical head, and page coordinates. It is
/// intentionally non-cloneable and can be consumed only by the second trace phase. The listed
/// source run ids grant no source access.
#[must_use = "trace source requirements must be consumed by transition-trace inspection"]
pub struct TransitionTraceSourceRequirements {
    authority_context: StoreAuthorityContext,
    store_identity: StoreIdentity,
    tenant_scope_id: TenantScopeId,
    run_id: RunId,
    at_journal_head: JournalHead,
    start: u32,
    end: usize,
    has_more: bool,
    next_index: Option<u32>,
    source_run_ids: Vec<RunId>,
    page_sources: Vec<PageTraceSource>,
    view: VerifiedRunView,
}

struct PageTraceSource {
    source_run_id: RunId,
    source_ref: CrossRunSourceRef,
    destination_value_ref: ValueRef,
    redaction_digest: SemanticDigest,
}

impl TransitionTraceSourceRequirements {
    /// Returns direct source runs named by only this exact fixed-head page.
    ///
    /// Values are sorted by canonical run identity and contain no duplicates. Each source requires
    /// a fresh, separate `InspectTrace` policy decision before the second phase.
    pub fn source_run_ids(&self) -> &[RunId] {
        &self.source_run_ids
    }

    pub(super) fn validate_root(
        &self,
        context: &StoreAuthorityContext,
        authority: &RunAccessAuthority<InspectTrace>,
    ) -> Result<()> {
        if !self.authority_context.is_same_instance(context)
            || &self.store_identity != context.store_identity()
        {
            return Err(StoreError::AccessDenied {
                purpose: "inspect_trace",
            });
        }
        let target = context.validate_run(authority)?;
        if target.tenant_scope_id != self.tenant_scope_id || target.run_id != self.run_id {
            return Err(StoreError::AccessDenied {
                purpose: "inspect_trace",
            });
        }
        Ok(())
    }

    pub(super) fn validate_source_authorities(
        &self,
        context: &StoreAuthorityContext,
        authorities: &[RunAccessAuthority<InspectTrace>],
    ) -> Result<Vec<RunId>> {
        validate_trace_source_authorities(
            &self.tenant_scope_id,
            &self.source_run_ids,
            context,
            authorities,
        )
    }

    pub(super) fn verify_source_view(
        &self,
        source_run_id: &RunId,
        source_view: &VerifiedRunView,
    ) -> Result<()> {
        let mut matched = false;
        for source in self
            .page_sources
            .iter()
            .filter(|source| &source.source_run_id == source_run_id)
        {
            matched = true;
            super::journal::verify_cross_run_source_view(
                &source.source_ref,
                &source.destination_value_ref,
                source_view,
            )?;
        }
        if !matched {
            return Err(StoreError::PersistedMismatch {
                field: "trace_source_view",
            });
        }
        Ok(())
    }
}

fn validate_trace_source_authorities(
    tenant_scope_id: &TenantScopeId,
    source_run_ids: &[RunId],
    context: &StoreAuthorityContext,
    authorities: &[RunAccessAuthority<InspectTrace>],
) -> Result<Vec<RunId>> {
    let mut run_ids = Vec::with_capacity(authorities.len());
    for authority in authorities {
        let target = context.validate_run(authority)?;
        if &target.tenant_scope_id != tenant_scope_id
            || source_run_ids.binary_search(&target.run_id).is_err()
        {
            return Err(StoreError::InvalidTracePage {
                field: "source_authorities",
            });
        }
        run_ids.push(target.run_id.clone());
    }
    if run_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(StoreError::InvalidTracePage {
            field: "source_authorities",
        });
    }
    Ok(run_ids)
}

/// One annex-validated transition trace.
///
/// Every retained value is inlined in this owned value. It grants no follow-up object access.
pub struct VerifiedTransitionTrace {
    canonical_value: CanonicalValue,
}

impl VerifiedTransitionTrace {
    pub(super) fn new(canonical_value: CanonicalValue) -> Result<Self> {
        RecoverabilityContractV2::embedded()?
            .encode(TRANSITION_TRACE_CONTRACT, &canonical_value)?;
        Ok(Self { canonical_value })
    }

    /// Returns the exact annex-validated canonical trace value.
    pub const fn canonical_value(&self) -> &CanonicalValue {
        &self.canonical_value
    }
}

/// One store-verified, head-fixed transition-trace page.
pub struct VerifiedTransitionTracePage {
    run_id: RunId,
    at_journal_head: JournalHead,
    transitions: Vec<VerifiedTransitionTrace>,
    has_more: bool,
    next_index: Option<u32>,
}

impl VerifiedTransitionTracePage {
    pub(super) fn new(
        run_id: RunId,
        at_journal_head: JournalHead,
        transitions: Vec<VerifiedTransitionTrace>,
        has_more: bool,
        next_index: Option<u32>,
        start: u32,
    ) -> Result<Self> {
        let expected_next = if has_more {
            let length =
                u32::try_from(transitions.len()).map_err(|_| StoreError::InvalidTracePage {
                    field: "next_index",
                })?;
            Some(
                start
                    .checked_add(length)
                    .ok_or(StoreError::InvalidTracePage {
                        field: "next_index",
                    })?,
            )
        } else {
            None
        };
        if next_index != expected_next {
            return Err(StoreError::InvalidTracePage {
                field: "next_index",
            });
        }
        Ok(Self {
            run_id,
            at_journal_head,
            transitions,
            has_more,
            next_index,
        })
    }

    /// Returns the inspected root run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact physical head fixed by the first page.
    pub const fn at_journal_head(&self) -> &JournalHead {
        &self.at_journal_head
    }

    /// Returns complete transition traces in committed semantic order.
    pub fn transitions(&self) -> &[VerifiedTransitionTrace] {
        &self.transitions
    }

    /// Returns whether another transition exists after this page.
    pub const fn has_more(&self) -> bool {
        self.has_more
    }

    /// Returns the next zero-based transition index when another page exists.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }
}

pub(super) fn prepare_transition_trace_requirements(
    authority_context: StoreAuthorityContext,
    request: TransitionTracePageRequest,
    view: VerifiedRunView,
) -> Result<TransitionTraceSourceRequirements> {
    let at_journal_head = request
        .at_journal_head
        .unwrap_or_else(|| view.journal_head().clone());
    let at_fields = at_journal_head.fields()?;
    let commit_index = usize::try_from(at_fields.run_sequence.checked_sub(1).ok_or(
        StoreError::InvalidTracePage {
            field: "at_journal_head",
        },
    )?)
    .map_err(|_| StoreError::InvalidTracePage {
        field: "at_journal_head",
    })?;
    let exact_head = view
        .journal()
        .commits()
        .get(commit_index)
        .map(|commit| commit.envelope().journal_head())
        .transpose()?
        .is_some_and(|head| head == at_journal_head);
    if !exact_head {
        return Err(StoreError::InvalidTracePage {
            field: "at_journal_head",
        });
    }

    let mut visible_transition_count = 0;
    for entry in view.transition_entries() {
        if entry.containing_journal_head().fields()?.run_sequence > at_fields.run_sequence {
            break;
        }
        visible_transition_count += 1;
    }
    let start = usize::try_from(request.start)
        .map_err(|_| StoreError::InvalidTracePage { field: "start" })?;
    if start > visible_transition_count {
        return Err(StoreError::InvalidTracePage { field: "start" });
    }
    let end = start
        .saturating_add(usize::from(request.limit))
        .min(visible_transition_count);
    let has_more = end < visible_transition_count;
    let next_index = has_more
        .then(|| {
            u32::try_from(end).map_err(|_| StoreError::InvalidTracePage {
                field: "next_index",
            })
        })
        .transpose()?;

    let mut source_run_ids = BTreeSet::new();
    let mut page_sources = Vec::new();
    for entry in view.transition_entries().skip(start).take(end - start) {
        let Some(input_manifest_ref) = transition_input_manifest_ref(entry.transition())? else {
            continue;
        };
        for binding in view.input_manifest(&input_manifest_ref)?.fields()?.bindings {
            let binding = binding.fields()?;
            if let InputSourceFields::CrossRun { source_ref } = binding.source.fields()? {
                let source_run_id = source_ref.identity()?.source_run_id;
                let redaction_digest = source_ref.redaction_digest()?;
                source_run_ids.insert(source_run_id.clone());
                page_sources.push(PageTraceSource {
                    source_run_id,
                    source_ref,
                    destination_value_ref: binding.value_ref,
                    redaction_digest,
                });
            }
        }
    }
    page_sources.sort_by(|left, right| {
        left.source_run_id
            .cmp(&right.source_run_id)
            .then_with(|| left.source_ref.as_bytes().cmp(right.source_ref.as_bytes()))
            .then_with(|| {
                left.destination_value_ref
                    .as_bytes()
                    .cmp(right.destination_value_ref.as_bytes())
            })
    });
    page_sources.dedup_by(|left, right| {
        left.source_run_id == right.source_run_id
            && left.source_ref == right.source_ref
            && left.destination_value_ref == right.destination_value_ref
    });

    Ok(TransitionTraceSourceRequirements {
        authority_context,
        store_identity: view.store_identity().clone(),
        tenant_scope_id: view.tenant_scope_id().clone(),
        run_id: view.run_id().clone(),
        at_journal_head,
        start: request.start,
        end,
        has_more,
        next_index,
        source_run_ids: source_run_ids.into_iter().collect(),
        page_sources,
        view,
    })
}

pub(super) fn render_transition_trace_page(
    requirements: TransitionTraceSourceRequirements,
    source_views: &BTreeMap<RunId, VerifiedRunView>,
) -> Result<VerifiedTransitionTracePage> {
    let TransitionTraceSourceRequirements {
        run_id,
        at_journal_head,
        start,
        end,
        has_more,
        next_index,
        page_sources,
        view,
        ..
    } = requirements;
    let start_index =
        usize::try_from(start).map_err(|_| StoreError::InvalidTracePage { field: "start" })?;
    let redaction_digests = page_sources
        .iter()
        .map(|source| {
            (
                source.source_ref.as_bytes().to_vec(),
                source.redaction_digest.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let transitions = view
        .transition_entries()
        .skip(start_index)
        .take(end - start_index)
        .map(|entry| {
            transition_trace(&view, entry, source_views, &redaction_digests)
                .and_then(VerifiedTransitionTrace::new)
        })
        .collect::<Result<Vec<_>>>()?;
    VerifiedTransitionTracePage::new(
        run_id,
        at_journal_head,
        transitions,
        has_more,
        next_index,
        start,
    )
}

fn transition_trace(
    view: &VerifiedRunView,
    entry: &FoldedTransitionEntry,
    source_views: &BTreeMap<RunId, VerifiedRunView>,
    redaction_digests: &BTreeMap<Vec<u8>, SemanticDigest>,
) -> Result<CanonicalValue> {
    let comparison = super::comparison::comparison_frame(view, entry)?;
    let transition = entry.transition().fields()?;
    let node = comparison.certified_node();
    let body = transition.body.fields()?;
    let (kind, input_manifest_ref, settlement, consumed_observation_ref, blocking_sources) =
        match body {
            TransitionBodyFields::PureSettled {
                input_manifest_ref,
                settlement,
            } => (
                "pure_settled",
                Some(input_manifest_ref),
                Some(settlement),
                None,
                Vec::new(),
            ),
            TransitionBodyFields::ReadSettled {
                input_manifest_ref,
                consumed_observation_ref,
                settlement,
                ..
            } => (
                "read_settled",
                Some(input_manifest_ref),
                Some(settlement),
                Some(consumed_observation_ref),
                Vec::new(),
            ),
            TransitionBodyFields::EffectRequested {
                input_manifest_ref, ..
            } => (
                "effect_requested",
                Some(input_manifest_ref),
                None,
                None,
                Vec::new(),
            ),
            TransitionBodyFields::EffectSettled {
                request_input_manifest_ref,
                consumed_terminal_observation_ref,
                settlement,
                ..
            } => (
                "effect_settled",
                Some(request_input_manifest_ref),
                Some(settlement),
                Some(consumed_terminal_observation_ref),
                Vec::new(),
            ),
            TransitionBodyFields::DependencySkipped { blocking_sources } => {
                ("dependency_skipped", None, None, None, blocking_sources)
            }
        };

    let consumed = consumed_observation_ref
        .as_ref()
        .map(|reference| consumed_observation(view, node.node_id(), reference))
        .transpose()?;
    let consumed_observation = consumed
        .as_ref()
        .map(|observed| retained_journal_value(observed.observation()))
        .transpose()?
        .unwrap_or(CanonicalValue::Null);
    let evidence = match (kind, consumed.as_ref()) {
        ("read_settled", Some(observed)) => read_evidence(view, observed)?,
        ("effect_settled", Some(observed)) => terminal_effect_evidence(view, observed)?,
        ("pure_settled" | "effect_requested" | "dependency_skipped", None) => Vec::new(),
        _ => return Err(trace_mismatch("trace_consumed_observation")),
    };
    let (result, outputs, facts, terminal_outcome) =
        settlement_trace(view, settlement.as_ref(), &blocking_sources)?;
    let inputs = input_manifest_ref
        .as_ref()
        .map(|reference| trace_inputs(view, reference, source_views, redaction_digests))
        .transpose()?
        .unwrap_or_default();
    let request = comparison
        .request()
        .map(|value| retained_value(view, value.value_ref()))
        .transpose()?
        .unwrap_or(CanonicalValue::Null);
    let containing_commit_digest = entry
        .containing_journal_head()
        .fields()?
        .commit_digest
        .as_str()
        .to_owned();

    trace_object([
        (
            "version",
            CanonicalValue::String(TRANSITION_TRACE_CONTRACT.to_owned()),
        ),
        ("transition_ref", entry.transition_ref().canonical_value()?),
        (
            "containing_commit_digest",
            CanonicalValue::String(containing_commit_digest),
        ),
        (
            "node_id",
            CanonicalValue::String(node.node_id().as_str().to_owned()),
        ),
        (
            "canonical_expansion_path",
            canonical_expansion_path(node.canonical_expansion_path())?,
        ),
        ("transition_kind", CanonicalValue::String(kind.to_owned())),
        ("before", transition.before.canonical_value()?),
        ("inputs", CanonicalValue::Array(inputs)),
        ("request", request),
        ("consumed_observation", consumed_observation),
        ("result", result),
        ("outputs", CanonicalValue::Array(outputs)),
        ("facts", CanonicalValue::Array(facts)),
        ("evidence", CanonicalValue::Array(evidence)),
        ("terminal_outcome", terminal_outcome),
        ("after", transition.after.canonical_value()?),
        (
            "closure_ref",
            entry
                .closure_ref()
                .map(|reference| reference.canonical_value())
                .transpose()?
                .unwrap_or(CanonicalValue::Null),
        ),
    ])
}

fn canonical_expansion_path(path: &CanonicalExpansionPath) -> Result<CanonicalValue> {
    let json = serde_json::to_string(path).map_err(|_| StoreError::JournalContract)?;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| StoreError::JournalContract)?;
    RecoverabilityContractV2::embedded()?
        .strict_decode("mfm.canonical-expansion-path.v1", canonical.as_bytes())?
        .canonical_value()
        .map_err(Into::into)
}

fn trace_inputs(
    view: &VerifiedRunView,
    input_manifest_ref: &InputManifestRef,
    source_views: &BTreeMap<RunId, VerifiedRunView>,
    redaction_digests: &BTreeMap<Vec<u8>, SemanticDigest>,
) -> Result<Vec<CanonicalValue>> {
    view.input_manifest(input_manifest_ref)?
        .fields()?
        .bindings
        .into_iter()
        .map(|binding| {
            let binding = binding.fields()?;
            let (lineage, expose_value) = match binding.source.fields()? {
                InputSourceFields::RunAdmission { record_ref } => (
                    trace_tagged_object(
                        "run_admission",
                        [("record_ref", record_ref.canonical_value()?)],
                    )?,
                    true,
                ),
                InputSourceFields::TransitionOutput {
                    transition_ref,
                    output_ref,
                    ..
                } => (
                    trace_tagged_object(
                        "transition_output",
                        [
                            ("transition_ref", transition_ref.canonical_value()?),
                            ("output_ref", output_ref.canonical_value()?),
                        ],
                    )?,
                    true,
                ),
                InputSourceFields::TransitionFact {
                    transition_ref,
                    fact_ref,
                    ..
                } => (
                    trace_tagged_object(
                        "transition_fact",
                        [
                            ("transition_ref", transition_ref.canonical_value()?),
                            ("fact_ref", fact_ref.canonical_value()?),
                        ],
                    )?,
                    true,
                ),
                InputSourceFields::CrossRun { source_ref } => {
                    let source_run_id = source_ref.identity()?.source_run_id;
                    if source_views.contains_key(&source_run_id) {
                        (
                            trace_tagged_object(
                                "cross_run_value",
                                [("source_ref", source_ref.canonical_value()?)],
                            )?,
                            true,
                        )
                    } else {
                        let digest = redaction_digests
                            .get(source_ref.as_bytes())
                            .ok_or_else(|| trace_mismatch("trace_source_redaction"))?;
                        (
                            trace_tagged_object(
                                "cross_run_redacted",
                                [(
                                    "source_ref_digest",
                                    CanonicalValue::String(digest.as_str().to_owned()),
                                )],
                            )?,
                            false,
                        )
                    }
                }
                InputSourceFields::Config { value_ref } => (
                    trace_tagged_object("config", [("value_ref", value_ref.canonical_value()?)])?,
                    true,
                ),
                InputSourceFields::QualifiedSupport {
                    member_path,
                    value_ref,
                } => (
                    trace_tagged_object(
                        "qualified_support",
                        [
                            (
                                "member_path",
                                CanonicalValue::String(member_path.as_str().to_owned()),
                            ),
                            ("value_ref", value_ref.canonical_value()?),
                        ],
                    )?,
                    true,
                ),
                InputSourceFields::Seed { value_ref } => (
                    trace_tagged_object("seed", [("value_ref", value_ref.canonical_value()?)])?,
                    true,
                ),
                InputSourceFields::Context { value_ref } => (
                    trace_tagged_object("context", [("value_ref", value_ref.canonical_value()?)])?,
                    true,
                ),
            };
            trace_object([
                (
                    "field_path",
                    CanonicalValue::String(binding.field_path.as_str().to_owned()),
                ),
                ("lineage", lineage),
                (
                    "source_field_path",
                    binding
                        .source_field_path
                        .map(|path| CanonicalValue::String(path.as_str().to_owned()))
                        .unwrap_or(CanonicalValue::Null),
                ),
                (
                    "value",
                    if expose_value {
                        retained_value(view, &binding.value_ref)?
                    } else {
                        CanonicalValue::Null
                    },
                ),
            ])
        })
        .collect()
}

fn consumed_observation<'view>(
    view: &'view VerifiedRunView,
    node_id: &mfm_ids::NodeId,
    observation_ref: &ObservationRef,
) -> Result<VerifiedObservedAccess<'view>> {
    let history = view.access_history(node_id)?;
    for attempt in history.entries() {
        let Some(observed) = attempt.observed() else {
            continue;
        };
        if observed.observation_ref() == observation_ref {
            return Ok(observed);
        }
    }
    Err(trace_mismatch("trace_consumed_observation"))
}

fn read_evidence(
    view: &VerifiedRunView,
    observed: &VerifiedObservedAccess<'_>,
) -> Result<Vec<CanonicalValue>> {
    let observation = observed.observation().fields()?;
    let (mut values, permits_scan_attestation) = match observation.outcome.fields()? {
        ObservationOutcomeFields::Returned { result_ref } => (
            vec![named_observation_value(
                view,
                observed.audit().authorization_ref(),
                result_ref,
                "outcome.result_ref",
            )?],
            true,
        ),
        ObservationOutcomeFields::DidNotEnter { safe_failure }
        | ObservationOutcomeFields::Indeterminate { safe_failure } => {
            let values = safe_failure
                .fields()?
                .diagnostic_ref
                .map(|diagnostic_ref| {
                    named_observation_value(
                        view,
                        observed.audit().authorization_ref(),
                        diagnostic_ref,
                        "outcome.safe_failure.diagnostic_ref",
                    )
                })
                .transpose()?
                .into_iter()
                .collect();
            (values, false)
        }
    };
    if let Some(attestation_ref) = observation.fact_selection_scan_attestation_ref {
        if !permits_scan_attestation {
            return Err(trace_mismatch("trace_read_scan_attestation"));
        }
        values.push(named_observation_value(
            view,
            observed.audit().authorization_ref(),
            attestation_ref,
            "fact_selection_scan_attestation_ref",
        )?);
    }
    sort_named_values(values)
}

fn terminal_effect_evidence(
    view: &VerifiedRunView,
    observed: &VerifiedObservedAccess<'_>,
) -> Result<Vec<CanonicalValue>> {
    let observation = observed.observation().fields()?;
    let ObservationOutcomeFields::Returned { result_ref } = observation.outcome.fields()? else {
        return Err(trace_mismatch("trace_terminal_effect_observation"));
    };
    if observation.fact_selection_scan_attestation_ref.is_some() {
        return Err(trace_mismatch("trace_terminal_effect_observation"));
    }

    let authorization_ref = observed.audit().authorization_ref();
    let contract = RecoverabilityContractV2::embedded()?;
    let result_fields = result_ref.fields()?;
    if !matches!(
        result_fields.producer_binding.fields()?,
        ProducerBindingFields::ExternalObservation {
            authorization_ref: ref producer_authorization,
            ref field_path,
        } if producer_authorization == authorization_ref
            && field_path.as_str() == "executor.ensure_result"
    ) {
        return Err(trace_mismatch("trace_terminal_effect_ensure_result"));
    }
    let ensure_object = view.retained_value(&result_ref)?;
    let ensure_result = ExecutorEnsureResult::strict_decode(ensure_object.bytes())?;
    let ExecutorEnsureResultFields::Terminal { evidence_ref } = ensure_result.fields()? else {
        return Err(trace_mismatch("trace_terminal_effect_ensure_result"));
    };
    let evidence_key = evidence_ref.as_bytes().to_vec();
    let mut pending = VecDeque::from([result_ref]);
    let mut visited = BTreeSet::new();
    let mut values = Vec::new();
    while let Some(value_ref) = pending.pop_front() {
        if !visited.insert(value_ref.as_bytes().to_vec()) {
            continue;
        }
        if visited.len() > 65_536 {
            return Err(trace_mismatch("trace_terminal_effect_closure"));
        }
        let fields = value_ref.fields()?;
        let ProducerBindingFields::ExternalObservation {
            authorization_ref: producer_authorization,
            field_path,
        } = fields.producer_binding.fields()?
        else {
            return Err(trace_mismatch("trace_terminal_effect_closure"));
        };
        if producer_authorization != *authorization_ref
            || !super::comparison::executor_path(
                &field_path,
                &fields.content_digest.digest().to_string(),
            )
        {
            return Err(trace_mismatch("trace_terminal_effect_closure"));
        }
        let object = view.retained_value(&value_ref)?;
        let validated = contract.strict_decode_schema_id(&fields.schema_id, object.bytes())?;
        for edge in contract.reference_edges(&validated)? {
            if edge.terminal_kind() == ReferenceTerminalKindV2::ValueRef {
                pending.push_back(ValueRef::strict_decode(edge.value().as_bytes())?);
            }
        }
        values.push(named_retained_value(
            &field_path,
            None,
            retained_value_from_validated(&fields, &validated)?,
        )?);
    }
    if !visited.contains(&evidence_key) {
        return Err(trace_mismatch("trace_terminal_effect_closure"));
    }
    sort_named_values(values)
}

fn named_observation_value(
    view: &VerifiedRunView,
    authorization_ref: &mfm_journal::v1::AuthorizationRef,
    value_ref: ValueRef,
    expected_path: &str,
) -> Result<CanonicalValue> {
    let fields = value_ref.fields()?;
    let ProducerBindingFields::ExternalObservation {
        authorization_ref: producer_authorization,
        field_path,
    } = fields.producer_binding.fields()?
    else {
        return Err(trace_mismatch("trace_observation_evidence"));
    };
    if producer_authorization != *authorization_ref || field_path.as_str() != expected_path {
        return Err(trace_mismatch("trace_observation_evidence"));
    }
    named_retained_value(&field_path, None, retained_value(view, &value_ref)?)
}

fn settlement_trace(
    view: &VerifiedRunView,
    settlement: Option<&Settlement>,
    blocking_sources: &[mfm_journal::v1::BlockingSource],
) -> Result<(
    CanonicalValue,
    Vec<CanonicalValue>,
    Vec<CanonicalValue>,
    CanonicalValue,
)> {
    let Some(settlement) = settlement else {
        let terminal_outcome = if blocking_sources.is_empty() {
            CanonicalValue::Null
        } else {
            trace_tagged_object(
                "skipped",
                [(
                    "blocking_sources",
                    CanonicalValue::Array(
                        blocking_sources
                            .iter()
                            .map(|source| source.canonical_value())
                            .collect::<mfm_journal::v1::Result<Vec<_>>>()?,
                    ),
                )],
            )?
        };
        return Ok((
            CanonicalValue::Null,
            Vec::new(),
            Vec::new(),
            terminal_outcome,
        ));
    };

    match settlement.fields()? {
        SettlementFields::Succeeded {
            output_bindings,
            fact_emissions,
        } => {
            let outputs = output_bindings
                .into_iter()
                .map(|binding| {
                    let fields = binding.fields()?;
                    named_retained_value(
                        &fields.field_path,
                        Some(fields.output_ordinal),
                        retained_value(view, &fields.value_ref)?,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let facts = fact_emissions
                .into_iter()
                .map(|emission| {
                    let fields = emission.fields()?;
                    let claim = FactClaimEnvelope::strict_decode(
                        view.retained_value(&fields.claim_ref)?.bytes(),
                    )?;
                    let claim_fields = claim.fields()?;
                    if claim_fields.fact_descriptor_ref != fields.fact_descriptor_ref {
                        return Err(trace_mismatch("trace_fact_claim"));
                    }
                    trace_object([
                        (
                            "emission_ordinal",
                            CanonicalValue::Unsigned(u64::from(fields.emission_ordinal)),
                        ),
                        (
                            "fact_descriptor_ref",
                            content_ref_value(&fields.fact_descriptor_ref)?,
                        ),
                        ("claim", retained_value(view, &fields.claim_ref)?),
                        ("subject", retained_value(view, &claim_fields.subject_ref)?),
                        (
                            "response",
                            retained_value(view, &claim_fields.response_ref)?,
                        ),
                    ])
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((
                CanonicalValue::Null,
                outputs,
                facts,
                trace_tagged_object("succeeded", std::iter::empty::<(&str, CanonicalValue)>())?,
            ))
        }
        SettlementFields::Failed { typed_failure_ref } => Ok((
            retained_value(view, &typed_failure_ref)?,
            Vec::new(),
            Vec::new(),
            trace_tagged_object(
                "failed",
                [("typed_failure_ref", typed_failure_ref.canonical_value()?)],
            )?,
        )),
    }
}

fn retained_value(view: &VerifiedRunView, value_ref: &ValueRef) -> Result<CanonicalValue> {
    let fields = value_ref.fields()?;
    let object = view.retained_value(value_ref)?;
    if fields.media_type == "application/json" {
        let validated = RecoverabilityContractV2::embedded()?
            .strict_decode("mfm.primitive-canonical_value.v1", object.bytes())?;
        retained_value_from_validated(&fields, &validated)
    } else {
        trace_retained_bytes(
            fields.schema_id.as_str(),
            fields.content_digest.as_str(),
            &fields.media_type,
            object.bytes(),
        )
    }
}

fn retained_value_from_validated(
    fields: &mfm_journal::v1::ValueRefFields,
    validated: &ValidatedCanonicalValueV2,
) -> Result<CanonicalValue> {
    trace_retained_json(
        fields.schema_id.as_str(),
        fields.content_digest.as_str(),
        &fields.media_type,
        validated.canonical_value()?,
    )
}

fn retained_journal_value(value: &impl PersistedJournalValue) -> Result<CanonicalValue> {
    let content_ref = value.content_ref()?;
    trace_retained_json(
        value.schema_id().as_str(),
        content_ref.content_digest().as_str(),
        "application/json",
        value.canonical_value()?,
    )
}

fn trace_retained_json(
    schema_id: &str,
    digest: &str,
    media_type: &str,
    canonical: CanonicalValue,
) -> Result<CanonicalValue> {
    trace_retained_value(
        schema_id,
        digest,
        media_type,
        trace_tagged_object("canonical_json", [("value", canonical)])?,
    )
}

fn trace_retained_bytes(
    schema_id: &str,
    digest: &str,
    media_type: &str,
    bytes: &[u8],
) -> Result<CanonicalValue> {
    trace_retained_value(
        schema_id,
        digest,
        media_type,
        trace_tagged_object(
            "bytes",
            [(
                "value",
                CanonicalValue::Bytes(CanonicalBytes::new(bytes.to_vec())),
            )],
        )?,
    )
}

fn trace_retained_value(
    schema_id: &str,
    digest: &str,
    media_type: &str,
    content: CanonicalValue,
) -> Result<CanonicalValue> {
    trace_object([
        ("schema_id", CanonicalValue::String(schema_id.to_owned())),
        ("digest", CanonicalValue::String(digest.to_owned())),
        ("media_type", CanonicalValue::String(media_type.to_owned())),
        ("content", content),
    ])
}

fn named_retained_value(
    name: &FieldPath,
    ordinal: Option<u32>,
    value: CanonicalValue,
) -> Result<CanonicalValue> {
    trace_object([
        ("name", CanonicalValue::String(name.as_str().to_owned())),
        (
            "ordinal",
            ordinal
                .map(|value| CanonicalValue::Unsigned(u64::from(value)))
                .unwrap_or(CanonicalValue::Null),
        ),
        ("value", value),
    ])
}

fn sort_named_values(mut values: Vec<CanonicalValue>) -> Result<Vec<CanonicalValue>> {
    values.sort_by(|left, right| {
        named_value_path(left)
            .unwrap_or_default()
            .cmp(named_value_path(right).unwrap_or_default())
    });
    if values.windows(2).any(|pair| {
        named_value_path(&pair[0])
            .is_some_and(|left| named_value_path(&pair[1]).is_some_and(|right| left == right))
    }) {
        return Err(trace_mismatch("trace_evidence_order"));
    }
    Ok(values)
}

fn named_value_path(value: &CanonicalValue) -> Option<&str> {
    let CanonicalValue::Object(value) = value else {
        return None;
    };
    let CanonicalValue::String(name) = value
        .entries()
        .find_map(|(key, value)| (key == "name").then_some(value))?
    else {
        return None;
    };
    Some(name.as_str())
}

fn content_ref_value(reference: &ContentRef) -> Result<CanonicalValue> {
    trace_object([
        (
            "schema_id",
            CanonicalValue::String(reference.schema_id().as_str().to_owned()),
        ),
        (
            "content_digest",
            CanonicalValue::String(reference.content_digest().as_str().to_owned()),
        ),
    ])
}

fn trace_tagged_object(
    kind: &str,
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    let mut values = vec![("kind".to_owned(), CanonicalValue::String(kind.to_owned()))];
    values.extend(entries.into_iter().map(|(key, value)| (key.into(), value)));
    trace_object(values)
}

fn trace_object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| StoreError::JournalContract)
}

fn trace_mismatch(field: &'static str) -> StoreError {
    StoreError::PersistedMismatch { field }
}

fn transition_input_manifest_ref(
    transition: &StateTransitionCommitted,
) -> Result<Option<InputManifestRef>> {
    Ok(match transition.fields()?.body.fields()? {
        TransitionBodyFields::PureSettled {
            input_manifest_ref, ..
        }
        | TransitionBodyFields::ReadSettled {
            input_manifest_ref, ..
        }
        | TransitionBodyFields::EffectRequested {
            input_manifest_ref, ..
        } => Some(input_manifest_ref),
        TransitionBodyFields::EffectSettled {
            request_input_manifest_ref,
            ..
        } => Some(request_input_manifest_ref),
        TransitionBodyFields::DependencySkipped { .. } => None,
    })
}

#[cfg(test)]
mod tests {
    use mfm_ids::{RunId, StoreEpoch, StoreScopeId, TenantScopeId};

    use super::{validate_trace_source_authorities, TransitionTracePageRequest};
    use crate::v1::{StoreAuthorityContext, StoreError, StoreIdentity};

    #[test]
    fn trace_page_request_rejects_out_of_bounds_limits() {
        for limit in [0, 501] {
            assert!(matches!(
                TransitionTracePageRequest::new(None, 0, limit),
                Err(StoreError::InvalidTracePage { field: "limit" })
            ));
        }
    }

    #[test]
    fn trace_source_authorities_reject_noncanonical_order() {
        let identity = StoreIdentity::new(
            StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "a".repeat(32)))
                .expect("store scope"),
            StoreEpoch::new(1),
        );
        let (context, issuer) = StoreAuthorityContext::bootstrap(identity);
        let tenant = TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "b".repeat(32)))
            .expect("tenant");
        let first = run_id('1');
        let second = run_id('2');
        let source_run_ids = vec![first.clone(), second.clone()];
        let authorities = [
            issuer.authorize_inspect_trace(tenant.clone(), second),
            issuer.authorize_inspect_trace(tenant.clone(), first),
        ];

        assert!(matches!(
            validate_trace_source_authorities(&tenant, &source_run_ids, &context, &authorities),
            Err(StoreError::InvalidTracePage {
                field: "source_authorities"
            })
        ));
    }

    fn run_id(fill: char) -> RunId {
        RunId::parse(format!("run:sha256-jcs-v1:{}", fill.to_string().repeat(64))).expect("run id")
    }
}
