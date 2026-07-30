use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{FieldPath, NodeId};
use mfm_journal::v2::{
    AuthorizationScopeFields, CrossRunSourceRefFields, FactRef, InputBinding, InputManifest,
    InputManifestRef, InputSource, NodePhase, OutputRef, ProducerBinding, TransitionBodyFields,
    ValueRef,
};
use mfm_spec::v1::{
    CertifiedFrameBinding, CertifiedInputBinding, CertifiedNodeContract, CertifiedSourceSelector,
};

use super::objects::{derive_value_ref, validate_value_contract, PreparedAuthority};
use super::preparation::PreparedFrameMaterial;
use super::{
    PreparedFrame, PreparedValue, Result, StoreAuthorityContext, StoreError, VerifiedRunView,
};

pub(super) const RUN_ADMISSION_INPUT_PATH: &str = "run_admission.input";
pub(super) const CONFIGURED_VALUE_PATH: &str = "config.configured";

struct ResolvedSource {
    source: InputSource,
    root_ref: ValueRef,
    root_bytes: Vec<u8>,
    selected_bytes: Vec<u8>,
    source_field_path: Option<FieldPath>,
}

impl VerifiedRunView {
    pub(super) fn prepare_frame_material(
        &self,
        authority: &StoreAuthorityContext,
        node_id: &NodeId,
    ) -> Result<PreparedFrame> {
        let node = self
            .certified_spec()
            .nodes()
            .iter()
            .find(|node| node.node_id() == node_id)
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        let phase = self
            .node_phase(node_id)
            .ok_or(StoreError::TransitionFoldMismatch { field: "node_id" })?;
        let historical = historical_input_manifest_ref(self, node_id)?;
        match (phase, historical) {
            (NodePhase::Unstarted, Some(reference))
            | (NodePhase::AwaitingEffect, Some(reference))
            | (NodePhase::Terminal, Some(reference)) => {
                reconstruct_frame(self, authority, node, reference)
            }
            (NodePhase::Unstarted, None)
                if self
                    .ready_node_ids()
                    .into_iter()
                    .any(|ready| ready == node_id) =>
            {
                assemble_frame(self, authority, node)
            }
            (NodePhase::Unstarted | NodePhase::AwaitingEffect | NodePhase::Terminal, _) => {
                Err(StoreError::TransitionFoldMismatch {
                    field: "callback_frame_not_available",
                })
            }
        }
    }
}

fn historical_input_manifest_ref(
    view: &VerifiedRunView,
    node_id: &NodeId,
) -> Result<Option<InputManifestRef>> {
    let mut selected = None;
    for entry in view.transition_entries() {
        let fields = entry.transition().fields()?;
        if &fields.node_id != node_id {
            continue;
        }
        let reference = match fields.body.fields()? {
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
        };
        merge_manifest_ref(&mut selected, reference)?;
    }
    if let Ok(history) = view.access_history(node_id) {
        for attempt in history.entries() {
            let fields = attempt.authorization().fields()?;
            if let AuthorizationScopeFields::Read { input_manifest_ref } = fields.scope.fields()? {
                merge_manifest_ref(&mut selected, Some(input_manifest_ref))?;
            }
        }
    }
    Ok(selected)
}

fn merge_manifest_ref(
    selected: &mut Option<InputManifestRef>,
    candidate: Option<InputManifestRef>,
) -> Result<()> {
    let Some(candidate) = candidate else {
        return Ok(());
    };
    if selected
        .as_ref()
        .is_some_and(|existing| existing != &candidate)
    {
        return Err(StoreError::TransitionFoldMismatch {
            field: "input_manifest_ref",
        });
    }
    *selected = Some(candidate);
    Ok(())
}

fn reconstruct_frame(
    view: &VerifiedRunView,
    authority: &StoreAuthorityContext,
    node: &CertifiedNodeContract,
    input_manifest_ref: InputManifestRef,
) -> Result<PreparedFrame> {
    let input_manifest_object = view.input_manifest_object(&input_manifest_ref)?;
    let input_manifest = InputManifest::strict_decode(input_manifest_object.bytes())?;
    let fields = input_manifest.fields()?;
    if fields.input_schema_id != *node.input_contract().schema_id()
        || fields.bindings.len() != node.input_bindings().len()
    {
        return Err(StoreError::TransitionFoldMismatch {
            field: "input_manifest_contract",
        });
    }
    let config_ref = fields
        .config_ref
        .ok_or(StoreError::TransitionFoldMismatch {
            field: "input_manifest_config_ref",
        })?;
    validate_value_contract(node.config_binding().value_contract(), &config_ref)?;
    let config_object = view.retained_value(&config_ref)?;
    let config = PreparedValue::new(config_ref.clone(), config_object.bytes().to_vec());

    let context = match (node.context_binding(), fields.context_ref.as_ref()) {
        (Some(binding), Some(value_ref)) => {
            validate_value_contract(binding.value_contract(), value_ref)?;
            let object = view.retained_value(value_ref)?;
            Some(PreparedValue::new(
                value_ref.clone(),
                object.bytes().to_vec(),
            ))
        }
        (None, None) => None,
        _ => {
            return Err(StoreError::TransitionFoldMismatch {
                field: "input_manifest_context_ref",
            });
        }
    };

    validate_value_contract(node.input_contract(), &fields.root_input_ref)?;
    let input_object = view.retained_value(&fields.root_input_ref)?;
    validate_historical_bindings(view, node, &fields.bindings, input_object.bytes())?;
    let input = PreparedValue::new(fields.root_input_ref.clone(), input_object.bytes().to_vec());

    let mut authorities = vec![
        PreparedAuthority::preexisting(
            input_manifest_ref.value_ref()?,
            input_manifest_object.bytes().to_vec(),
        )?,
        PreparedAuthority::preexisting(config_ref, config_object.bytes().to_vec())?,
        PreparedAuthority::preexisting(fields.root_input_ref, input_object.bytes().to_vec())?,
    ];
    if let Some(value) = &context {
        authorities.push(PreparedAuthority::preexisting(
            value.value_ref().clone(),
            value.bytes().to_vec(),
        )?);
    }
    for binding in fields.bindings {
        let value_ref = binding.fields()?.value_ref;
        let object = view.retained_value(&value_ref)?;
        authorities.push(PreparedAuthority::preexisting(
            value_ref,
            object.bytes().to_vec(),
        )?);
    }

    Ok(PreparedFrame::new(PreparedFrameMaterial {
        authority: authority.clone(),
        tenant_scope_id: view.tenant_scope_id().clone(),
        run_id: view.run_id().clone(),
        journal_head: view.journal_head().clone(),
        node_id: node.node_id().clone(),
        input_manifest,
        input_manifest_ref,
        config,
        context,
        input,
        authorities,
    }))
}

pub(super) fn validate_historical_bindings(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    bindings: &[InputBinding],
    root_input_bytes: &[u8],
) -> Result<()> {
    let mut reconstructed = serde_json::Value::Object(serde_json::Map::new());
    for (recorded, certified) in bindings.iter().zip(node.input_bindings()) {
        let fields = recorded.fields()?;
        if fields.field_path != *certified.destination_field_path() {
            return Err(StoreError::TransitionFoldMismatch {
                field: "input_binding_destination",
            });
        }
        let root = view.retained_value(&fields.value_ref)?;
        let selected = select_canonical(root.bytes(), fields.source_field_path.as_ref())?;
        if fields.source_field_path.is_none() {
            validate_value_contract(certified.value_contract(), &fields.value_ref)?;
        }
        let matches_certified = certified.ordered_sources().iter().any(|selector| {
            source_matches_selector(view, selector, &fields.source, &fields.value_ref)
                .is_ok_and(|matches| matches)
                && selector.source_field_path() == fields.source_field_path.as_ref()
        });
        if !matches_certified {
            return Err(StoreError::TransitionFoldMismatch {
                field: "input_binding_source",
            });
        }
        insert_json_path(
            &mut reconstructed,
            certified.destination_field_path(),
            serde_json::from_slice(selected.as_bytes()).map_err(|_| StoreError::JournalContract)?,
        )?;
    }
    let encoded = serde_json::to_string(&reconstructed).map_err(|_| StoreError::JournalContract)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&encoded)
        .map_err(|_| StoreError::JournalContract)?;
    if canonical.as_bytes() != root_input_bytes {
        return Err(StoreError::TransitionFoldMismatch {
            field: "root_input_assembly",
        });
    }
    Ok(())
}

fn source_matches_selector(
    view: &VerifiedRunView,
    selector: &CertifiedSourceSelector,
    source: &InputSource,
    value_ref: &ValueRef,
) -> Result<bool> {
    let Some(resolved) = resolve_source(view, selector)? else {
        return Ok(false);
    };
    Ok(&resolved.root_ref == value_ref && &resolved.source == source)
}

fn assemble_frame(
    view: &VerifiedRunView,
    authority: &StoreAuthorityContext,
    node: &CertifiedNodeContract,
) -> Result<PreparedFrame> {
    let producer = ProducerBinding::input_assembly(view.run_id(), node.node_id())?;
    let mut authorities = Vec::new();

    let config_source = resolve_source(view, node.config_binding().source())?.ok_or(
        StoreError::TransitionFoldMismatch {
            field: "config_source",
        },
    )?;
    let config = prepared_frame_value(
        node.config_binding(),
        config_source,
        &producer,
        &mut authorities,
    )?;

    let context = node
        .context_binding()
        .map(|binding| {
            let source = resolve_source(view, binding.source())?.ok_or(
                StoreError::TransitionFoldMismatch {
                    field: "context_source",
                },
            )?;
            prepared_frame_value(binding, source, &producer, &mut authorities)
        })
        .transpose()?;

    let mut root = serde_json::Value::Object(serde_json::Map::new());
    let mut input_bindings = Vec::with_capacity(node.input_bindings().len());
    for binding in node.input_bindings() {
        let source = select_first_available(view, binding)?;
        if source.source_field_path.is_none() {
            validate_value_contract(binding.value_contract(), &source.root_ref)?;
        }
        insert_json_path(
            &mut root,
            binding.destination_field_path(),
            serde_json::from_slice(&source.selected_bytes)
                .map_err(|_| StoreError::JournalContract)?,
        )?;
        authorities.push(PreparedAuthority::preexisting(
            source.root_ref.clone(),
            source.root_bytes.clone(),
        )?);
        input_bindings.push(InputBinding::new(
            binding.destination_field_path(),
            &source.source,
            source.source_field_path.as_ref(),
            &source.root_ref,
        )?);
    }

    let encoded = serde_json::to_string(&root).map_err(|_| StoreError::JournalContract)?;
    let input_bytes = PlainCanonicalJsonBytes::from_json_str(&encoded)
        .map_err(|_| StoreError::JournalContract)?;
    let input_ref = derive_value_ref(node.input_contract(), &producer, input_bytes.as_bytes())?;
    authorities.push(PreparedAuthority::produced(
        input_ref.clone(),
        input_bytes.to_vec(),
    )?);

    let input_manifest = InputManifest::new(
        node.input_contract().schema_id(),
        &input_ref,
        Some(config.value_ref()),
        context.as_ref().map(PreparedValue::value_ref),
        &input_bindings,
    )?;
    let manifest_ref = derive_value_ref(
        view.certified_spec()
            .journal_protocol_contracts()
            .input_manifest_contract(),
        &producer,
        input_manifest.as_bytes(),
    )?;
    authorities.push(PreparedAuthority::produced(
        manifest_ref.clone(),
        input_manifest.as_bytes().to_vec(),
    )?);
    let input_manifest_ref = InputManifestRef::new(&manifest_ref)?;

    Ok(PreparedFrame::new(PreparedFrameMaterial {
        authority: authority.clone(),
        tenant_scope_id: view.tenant_scope_id().clone(),
        run_id: view.run_id().clone(),
        journal_head: view.journal_head().clone(),
        node_id: node.node_id().clone(),
        input_manifest,
        input_manifest_ref,
        config,
        context,
        input: PreparedValue::new(input_ref, input_bytes.to_vec()),
        authorities,
    }))
}

fn prepared_frame_value(
    binding: &CertifiedFrameBinding,
    source: ResolvedSource,
    producer: &ProducerBinding,
    authorities: &mut Vec<PreparedAuthority>,
) -> Result<PreparedValue> {
    if source.source_field_path.is_none() {
        validate_value_contract(binding.value_contract(), &source.root_ref)?;
        authorities.push(PreparedAuthority::preexisting(
            source.root_ref.clone(),
            source.root_bytes,
        )?);
        return Ok(PreparedValue::new(source.root_ref, source.selected_bytes));
    }
    let value_ref = derive_value_ref(binding.value_contract(), producer, &source.selected_bytes)?;
    authorities.push(PreparedAuthority::produced(
        value_ref.clone(),
        source.selected_bytes.clone(),
    )?);
    Ok(PreparedValue::new(value_ref, source.selected_bytes))
}

fn select_first_available(
    view: &VerifiedRunView,
    binding: &CertifiedInputBinding,
) -> Result<ResolvedSource> {
    for selector in binding.ordered_sources() {
        if let Some(source) = resolve_source(view, selector)? {
            return Ok(source);
        }
    }
    Err(StoreError::TransitionFoldMismatch {
        field: "input_source_not_available",
    })
}

fn resolve_source(
    view: &VerifiedRunView,
    selector: &CertifiedSourceSelector,
) -> Result<Option<ResolvedSource>> {
    match selector {
        CertifiedSourceSelector::RunAdmission { source_field_path } => {
            let root_ref = initial_binding(view, RUN_ADMISSION_INPUT_PATH)?;
            let admission_ref = admission_record_ref(view)?;
            resolved(
                view,
                InputSource::run_admission(&admission_ref)?,
                root_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::Config { source_field_path } => {
            let root_ref = initial_binding(view, CONFIGURED_VALUE_PATH)?;
            resolved(
                view,
                InputSource::config(&root_ref)?,
                root_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::QualifiedSupport {
            member_path,
            source_field_path,
        } => {
            let root_ref = initial_binding(view, member_path.as_str())?;
            resolved(
                view,
                InputSource::qualified_support(member_path, &root_ref)?,
                root_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::Seed { source_field_path } => {
            let entries = view
                .seed_manifest()
                .entries()?
                .into_iter()
                .map(|entry| Ok((entry.field_path()?, entry.value_ref()?)))
                .collect::<std::result::Result<Vec<_>, mfm_journal::v2::JournalError>>()?;
            let (entry_path, root_ref, nested) =
                select_manifest_value(entries, source_field_path.as_ref())?;
            require_initial_binding(view, &format!("seed.{}", entry_path.as_str()), &root_ref)?;
            resolved(view, InputSource::seed(&root_ref)?, root_ref, nested).map(Some)
        }
        CertifiedSourceSelector::Context { source_field_path } => {
            let entries = view
                .context_manifest()
                .entries()?
                .into_iter()
                .map(|entry| Ok((entry.field_path()?, entry.value_ref()?)))
                .collect::<std::result::Result<Vec<_>, mfm_journal::v2::JournalError>>()?;
            let (entry_path, root_ref, nested) =
                select_manifest_value(entries, source_field_path.as_ref())?;
            require_initial_binding(view, &format!("context.{}", entry_path.as_str()), &root_ref)?;
            resolved(view, InputSource::context(&root_ref)?, root_ref, nested).map(Some)
        }
        CertifiedSourceSelector::NodeOutput {
            producer_node_id,
            output_ordinal,
            source_field_path,
        } => {
            let Some(transition_ref) = view.node_terminal_transition_ref(producer_node_id) else {
                return Ok(None);
            };
            let Some(output) = view.node_outputs(producer_node_id).iter().find(|output| {
                output
                    .fields()
                    .is_ok_and(|fields| fields.output_ordinal == *output_ordinal)
            }) else {
                return Ok(None);
            };
            let root_ref = output.fields()?.value_ref;
            let output_ref = OutputRef::new(transition_ref, *output_ordinal)?;
            resolved(
                view,
                InputSource::transition_output(view.run_id(), transition_ref, &output_ref)?,
                root_ref,
                source_field_path.clone(),
            )
            .map(Some)
        }
        CertifiedSourceSelector::NodeFact {
            producer_node_id,
            emission_ordinal,
        } => {
            let Some(transition_ref) = view.node_terminal_transition_ref(producer_node_id) else {
                return Ok(None);
            };
            let Some(fact) = view.node_facts(producer_node_id).iter().find(|fact| {
                fact.fields()
                    .is_ok_and(|fields| fields.emission_ordinal == *emission_ordinal)
            }) else {
                return Ok(None);
            };
            let root_ref = fact.fields()?.claim_ref;
            let fact_ref = FactRef::new(transition_ref, *emission_ordinal)?;
            resolved(
                view,
                InputSource::transition_fact(view.run_id(), transition_ref, &fact_ref)?,
                root_ref,
                None,
            )
            .map(Some)
        }
        CertifiedSourceSelector::CrossRunEffectiveOutput { source_field_path } => {
            resolve_cross_run(view, source_field_path.as_ref(), None)
        }
        CertifiedSourceSelector::CrossRunEvidence {
            source_field_path,
            certified_evidence_role_ref,
        } => resolve_cross_run(
            view,
            source_field_path.as_ref(),
            Some(certified_evidence_role_ref),
        ),
    }
}

fn resolve_cross_run(
    view: &VerifiedRunView,
    selected_path: Option<&FieldPath>,
    evidence_role: Option<&mfm_ids::ContentRef>,
) -> Result<Option<ResolvedSource>> {
    let mut entries = Vec::new();
    for entry in view.cross_run_source_manifest().entries()? {
        let source = entry.source()?;
        let role_matches = match (source.fields()?, evidence_role) {
            (CrossRunSourceRefFields::EffectiveOutput { .. }, None) => true,
            (
                CrossRunSourceRefFields::EvidenceOnly {
                    certified_evidence_role_ref,
                    ..
                },
                Some(expected),
            ) => certified_evidence_role_ref == *expected,
            _ => false,
        };
        if role_matches {
            entries.push((entry.field_path()?, source));
        }
    }
    let (entry_path, source, nested) = select_manifest_value(entries, selected_path)?;
    let root_ref = initial_binding(view, &format!("cross_run.{}", entry_path.as_str()))?;
    resolved(view, InputSource::cross_run(&source)?, root_ref, nested).map(Some)
}

fn resolved(
    view: &VerifiedRunView,
    source: InputSource,
    root_ref: ValueRef,
    source_field_path: Option<FieldPath>,
) -> Result<ResolvedSource> {
    let object = view.retained_value(&root_ref)?;
    let selected = select_canonical(object.bytes(), source_field_path.as_ref())?;
    Ok(ResolvedSource {
        source,
        root_ref,
        root_bytes: object.bytes().to_vec(),
        selected_bytes: selected.to_vec(),
        source_field_path,
    })
}

pub(super) fn select_manifest_value<T>(
    entries: Vec<(FieldPath, T)>,
    selected_path: Option<&FieldPath>,
) -> Result<(FieldPath, T, Option<FieldPath>)> {
    if entries.is_empty() {
        return Err(StoreError::TransitionFoldMismatch {
            field: "admission_manifest_source",
        });
    }
    let selected = match selected_path {
        None if entries.len() == 1 => {
            let mut entries = entries;
            let (path, value) = entries.pop().ok_or(StoreError::TransitionFoldMismatch {
                field: "admission_manifest_source",
            })?;
            return Ok((path, value, None));
        }
        None => {
            return Err(StoreError::TransitionFoldMismatch {
                field: "admission_manifest_source",
            });
        }
        Some(selected) => entries
            .iter()
            .enumerate()
            .filter_map(|(index, (entry, _))| {
                path_suffix(selected, entry).map(|suffix| (index, entry.as_str().len(), suffix))
            })
            .max_by_key(|(_, length, _)| *length),
    };
    let Some((index, _, suffix)) = selected else {
        let mut entries = entries;
        if entries.len() == 1 {
            let (path, value) = entries.pop().ok_or(StoreError::TransitionFoldMismatch {
                field: "admission_manifest_source",
            })?;
            return Ok((path, value, selected_path.cloned()));
        }
        return Err(StoreError::TransitionFoldMismatch {
            field: "admission_manifest_source",
        });
    };
    let (path, value) =
        entries
            .into_iter()
            .nth(index)
            .ok_or(StoreError::TransitionFoldMismatch {
                field: "admission_manifest_source",
            })?;
    Ok((path, value, suffix))
}

pub(super) fn path_suffix(selected: &FieldPath, root: &FieldPath) -> Option<Option<FieldPath>> {
    if selected == root {
        return Some(None);
    }
    selected
        .as_str()
        .strip_prefix(root.as_str())
        .and_then(|suffix| suffix.strip_prefix('.'))
        .and_then(|suffix| FieldPath::new(suffix).ok())
        .map(Some)
}

fn initial_bindings(view: &VerifiedRunView) -> Result<BTreeMap<FieldPath, ValueRef>> {
    view.admission()
        .fields()?
        .initial_bindings
        .into_iter()
        .map(|binding| {
            let fields = binding.fields()?;
            Ok((fields.field_path, fields.value_ref))
        })
        .collect::<std::result::Result<_, mfm_journal::v2::JournalError>>()
        .map_err(Into::into)
}

fn initial_binding(view: &VerifiedRunView, path: &str) -> Result<ValueRef> {
    let path = FieldPath::new(path)?;
    initial_bindings(view)?
        .remove(&path)
        .ok_or(StoreError::TransitionFoldMismatch {
            field: "initial_binding",
        })
}

fn require_initial_binding(view: &VerifiedRunView, path: &str, expected: &ValueRef) -> Result<()> {
    if &initial_binding(view, path)? == expected {
        Ok(())
    } else {
        Err(StoreError::TransitionFoldMismatch {
            field: "initial_binding",
        })
    }
}

fn admission_record_ref(view: &VerifiedRunView) -> Result<mfm_journal::v2::RecordRef> {
    view.journal()
        .commits()
        .first()
        .and_then(|commit| commit.records().first())
        .ok_or(StoreError::EmptyJournal)?
        .record_ref(view.run_id(), 1)
}

pub(super) fn select_canonical(
    bytes: &[u8],
    source_field_path: Option<&FieldPath>,
) -> Result<PlainCanonicalJsonBytes> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| StoreError::JournalContract)?;
    if let Some(path) = source_field_path {
        for segment in path.as_str().split('.') {
            value = match value {
                serde_json::Value::Object(mut entries) => {
                    entries
                        .remove(segment)
                        .ok_or(StoreError::TransitionFoldMismatch {
                            field: "source_field_path",
                        })?
                }
                serde_json::Value::Array(entries) => {
                    let index = segment.parse::<usize>().map_err(|_| {
                        StoreError::TransitionFoldMismatch {
                            field: "source_field_path",
                        }
                    })?;
                    entries
                        .into_iter()
                        .nth(index)
                        .ok_or(StoreError::TransitionFoldMismatch {
                            field: "source_field_path",
                        })?
                }
                _ => {
                    return Err(StoreError::TransitionFoldMismatch {
                        field: "source_field_path",
                    });
                }
            };
        }
    }
    let encoded = serde_json::to_string(&value).map_err(|_| StoreError::JournalContract)?;
    PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| StoreError::JournalContract)
}

pub(super) fn insert_json_path(
    root: &mut serde_json::Value,
    field_path: &FieldPath,
    value: serde_json::Value,
) -> Result<()> {
    let segments = field_path.as_str().split('.').collect::<Vec<_>>();
    insert_json_segments(root, &segments, value)
}

fn insert_json_segments(
    current: &mut serde_json::Value,
    segments: &[&str],
    value: serde_json::Value,
) -> Result<()> {
    let (segment, remaining) =
        segments
            .split_first()
            .ok_or(StoreError::TransitionFoldMismatch {
                field: "input_destination",
            })?;
    let object = current
        .as_object_mut()
        .ok_or(StoreError::TransitionFoldMismatch {
            field: "input_destination",
        })?;
    if remaining.is_empty() {
        if object.insert((*segment).to_owned(), value).is_some() {
            return Err(StoreError::TransitionFoldMismatch {
                field: "input_destination",
            });
        }
        return Ok(());
    }
    let nested = object
        .entry((*segment).to_owned())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    insert_json_segments(nested, remaining, value)
}
