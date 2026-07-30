use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::{ContentRef, FieldPath, NodeId, StableAuthorKey, StableId};
use mfm_journal::v2::{ProducerBindingFields, ValueRef};
use mfm_program::{
    QualifiedEntryPointDefinition, QualifiedExecutorExpansion, QualifiedFrameworkPolicy,
    QualifiedInputContract, QualifiedProgramDefinition, QualifiedSourceContract,
    QualifiedStateContract, QualifiedStateDefinition, QualifiedSupportNodeSelector,
    QualifiedSupportNodeTemplate, QualifiedSupportSourceTemplate,
};
use mfm_spec::{
    AuthoredBaseKind, AuthoredNode, AuthoredSourceSelector, CanonicalAuthoredProgram,
    CanonicalExpansionPath, CanonicalExpansionStep, CertifiedFrameBinding, CertifiedInputBinding,
    CertifiedJournalProtocolContracts, CertifiedNodeContract, CertifiedOutputBinding,
    CertifiedSourceSelector, CertifiedStateExecution, EntryPointContract, ExpandedCertifiedSpec,
    PlanningProfile, PublicOutputContract, RetainedValueContract, RunTerminalContract,
};

use crate::{CertifyError, Result};

/// Thin certifier-local view over the registry-owned immutable definition.
pub(crate) struct PlanningCatalog {
    definition: std::sync::Arc<QualifiedProgramDefinition>,
}

impl PlanningCatalog {
    pub(crate) fn from_definition(
        definition: std::sync::Arc<QualifiedProgramDefinition>,
    ) -> Result<Self> {
        let catalog = Self { definition };
        catalog.validate_closed_references()?;
        Ok(catalog)
    }

    pub(crate) fn planner_contract_ref(&self) -> &ContentRef {
        self.definition.planner_contract_ref()
    }

    pub(crate) fn planner_implementation_ref(&self) -> &ContentRef {
        self.definition.planner_implementation_ref()
    }

    pub(crate) fn journal_protocol_contracts(&self) -> &CertifiedJournalProtocolContracts {
        self.definition.journal_protocol_contracts()
    }

    pub(crate) fn current_state_manifest(&self) -> Result<&mfm_spec::StateImplementationManifest> {
        sole_current(
            self.definition.state_manifests(),
            "state implementation manifest",
        )
    }

    pub(crate) fn current_capability_manifest(
        &self,
    ) -> Result<&mfm_spec::CapabilityBindingManifest> {
        sole_current(
            self.definition.capability_manifests(),
            "capability binding manifest",
        )
    }

    fn entry_point(
        &self,
        entry_point: &EntryPointContract,
    ) -> Result<&QualifiedEntryPointDefinition> {
        let planned = self
            .definition
            .entry_points()
            .get(entry_point.entry_point_id())
            .ok_or_else(|| {
                CertifyError::Planning("unregistered planning entry point".to_owned())
            })?;
        if planned.entry_point() != entry_point {
            return Err(CertifyError::Planning(
                "published entry point differs from its planning projection".to_owned(),
            ));
        }
        Ok(planned.as_ref())
    }

    fn state(&self, state_ref: &ContentRef) -> Result<&QualifiedStateDefinition> {
        self.definition
            .states()
            .get(state_ref)
            .map(std::sync::Arc::as_ref)
            .ok_or_else(|| {
                CertifyError::Planning(format!("unresolved state contract {state_ref:?}"))
            })
    }

    fn policy(&self, policy_ref: &ContentRef) -> Result<&QualifiedFrameworkPolicy> {
        self.definition
            .framework_policies()
            .get(policy_ref)
            .ok_or_else(|| {
                CertifyError::Planning(format!("unresolved framework policy {policy_ref:?}"))
            })
    }

    fn executor_for_state(
        &self,
        state: &QualifiedStateContract,
    ) -> Result<Option<&QualifiedExecutorExpansion>> {
        let CertifiedStateExecution::Effect {
            executor_operation_id,
            executor_binding_ref,
            ..
        } = state.execution()
        else {
            return Ok(None);
        };
        let effect = self
            .definition
            .effects()
            .get(&(executor_binding_ref.clone(), executor_operation_id.clone()))
            .ok_or_else(|| {
                CertifyError::Planning(
                    "effect state has no exact executor planning projection".to_owned(),
                )
            })?;
        self.definition
            .executors()
            .get(effect.executor_contract_ref())
            .map(Some)
            .ok_or_else(|| {
                CertifyError::Planning(
                    "effect state selects an unresolved executor expansion".to_owned(),
                )
            })
    }

    fn validate_closed_references(&self) -> Result<()> {
        if self.definition.entry_points().is_empty() || self.definition.states().is_empty() {
            return Err(CertifyError::Planning(
                "planning catalog requires an entry point and a state".to_owned(),
            ));
        }
        for entry in self.definition.entry_points().values() {
            let profile = entry.entry_point().planning_profile();
            if profile.planner_contract_ref() != self.planner_contract_ref()
                || profile.planner_implementation_ref() != self.planner_implementation_ref()
                || profile
                    .framework_policy_refs()
                    .iter()
                    .any(|reference| !self.definition.framework_policies().contains_key(reference))
            {
                return Err(CertifyError::Planning(
                    "entry point selects planning values outside the catalog".to_owned(),
                ));
            }
        }
        let mut used_effects = BTreeSet::new();
        for state in self.definition.states().values() {
            if let CertifiedStateExecution::Effect {
                executor_operation_id,
                executor_binding_ref,
                ..
            } = state.contract().execution()
            {
                used_effects.insert((executor_binding_ref.clone(), executor_operation_id.clone()));
                self.executor_for_state(state.contract())?;
            }
        }
        if used_effects != self.definition.effects().keys().cloned().collect() {
            return Err(CertifyError::Planning(
                "effect planning projections are not an exact state closure".to_owned(),
            ));
        }
        for policy in self.definition.framework_policies().values() {
            for template in policy.pre_nodes().iter().chain(policy.post_nodes()) {
                self.state(template.state_contract_ref())?;
            }
        }
        for expansion in self.definition.executors().values() {
            for template in expansion.pre_nodes().iter().chain(expansion.post_nodes()) {
                let state = self.state(template.state_contract_ref())?;
                if self
                    .executor_for_state(state.contract())?
                    .is_some_and(|nested| {
                        !matches!(nested, QualifiedExecutorExpansion::Leaf { .. })
                    })
                {
                    return Err(CertifyError::Planning(
                        "executor support state selects a non-leaf executor".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn expected_state_implementations(
        &self,
        spec: &ExpandedCertifiedSpec,
    ) -> Result<BTreeMap<ContentRef, ContentRef>> {
        spec.nodes()
            .iter()
            .map(|node| {
                let state = self.state(node.state_contract_ref())?;
                Ok((
                    state.contract().state_contract_ref().clone(),
                    state.component_implementation_ref().clone(),
                ))
            })
            .collect()
    }

    pub(crate) fn expected_operation_bindings(
        &self,
        spec: &ExpandedCertifiedSpec,
    ) -> Result<BTreeMap<StableId, ContentRef>> {
        let mut bindings = BTreeMap::new();
        for node in spec.nodes() {
            let Some((operation_id, binding_ref)) = node.execution().operation_binding() else {
                continue;
            };
            if bindings
                .insert(operation_id.clone(), binding_ref.clone())
                .is_some_and(|previous| previous != *binding_ref)
            {
                return Err(CertifyError::Planning(
                    "one operation selects multiple capability bindings".to_owned(),
                ));
            }
        }
        Ok(bindings)
    }
}

fn sole_current<'a, K, V>(values: &'a BTreeMap<K, V>, description: &str) -> Result<&'a V> {
    let mut values = values.values();
    let value = values
        .next()
        .ok_or_else(|| CertifyError::Planning(format!("current {description} is unavailable")))?;
    if values.next().is_some() {
        return Err(CertifyError::Planning(format!(
            "current {description} is ambiguous"
        )));
    }
    Ok(value)
}

pub(crate) struct CompositePlanner<'a> {
    catalog: &'a PlanningCatalog,
}

impl<'a> CompositePlanner<'a> {
    pub(crate) const fn new(catalog: &'a PlanningCatalog) -> Self {
        Self { catalog }
    }

    pub(crate) fn expand(
        &self,
        entry_point: &EntryPointContract,
        authored: &CanonicalAuthoredProgram,
    ) -> Result<ExpandedCertifiedSpec> {
        if authored.entry_point_operation_id() != entry_point.entry_point_operation_id() {
            return Err(CertifyError::Planning(
                "authored operation differs from the published entry point".to_owned(),
            ));
        }
        let entry = self.catalog.entry_point(entry_point)?;
        let profile = entry_point.planning_profile();
        self.validate_profile(profile)?;
        let mut positioned = authored
            .nodes()
            .iter()
            .map(|node| authored_path(node, authored.nodes()).map(|path| (path, node)))
            .collect::<Result<Vec<_>>>()?;
        positioned.sort_by(|left, right| left.0.cmp(&right.0));

        let mut pending = Vec::new();
        let mut reserved = BTreeMap::new();
        let mut effective = BTreeMap::new();
        for (base_path, node) in &positioned {
            let state = self.catalog.state(node.state_contract_ref())?;
            let applicable = profile
                .framework_policy_refs()
                .iter()
                .enumerate()
                .filter_map(|(ordinal, reference)| {
                    let policy = self.catalog.policy(reference).ok()?;
                    (node.base_kind() == AuthoredBaseKind::Authored
                        && policy.applies_to(node.state_contract_ref()))
                    .then_some((ordinal, policy.clone()))
                })
                .collect::<Vec<_>>();
            let occurrence =
                self.reserve_framework(0, base_path.clone(), state, &applicable, &mut pending)?;
            effective.insert(node.stable_key().clone(), occurrence.effective());
            reserved.insert(node.stable_key().clone(), occurrence);
        }

        let mut boundaries = BTreeMap::new();
        for (_, node) in &positioned {
            boundaries.insert(
                node.stable_key().clone(),
                self.authored_bindings(entry, authored, node, &effective, &pending)?,
            );
        }
        for (_, node) in positioned {
            let occurrence = reserved.remove(node.stable_key()).ok_or_else(|| {
                CertifyError::Planning("reserved authored occurrence is missing".to_owned())
            })?;
            let boundary = boundaries.remove(node.stable_key()).ok_or_else(|| {
                CertifyError::Planning("authored occurrence bindings are missing".to_owned())
            })?;
            self.bind_framework(occurrence, &boundary, entry, &mut pending)?;
        }

        let ids = pending
            .iter()
            .map(|node| node.path.node_id(node.state.state_contract_ref()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let nodes = pending
            .into_iter()
            .enumerate()
            .map(|(index, node)| finalize_node(node, index, &ids))
            .collect::<Result<Vec<_>>>()?;
        let public_output_contract =
            self.public_output_contract(entry, authored, &effective, &ids, &nodes)?;
        let required_success_nodes = authored
            .required_success_node_keys()
            .iter()
            .map(|key| {
                effective
                    .get(key)
                    .map(|index| ids[*index].clone())
                    .ok_or_else(|| {
                        CertifyError::Planning(format!("required-success node {key} is unresolved"))
                    })
            })
            .collect::<Result<Vec<_>>>()?;

        ExpandedCertifiedSpec::new(
            authored.content_ref()?,
            profile.content_ref()?,
            nodes,
            public_output_contract,
            RunTerminalContract::new(required_success_nodes, true),
            self.catalog.journal_protocol_contracts().clone(),
        )
        .map_err(Into::into)
    }

    fn validate_profile(&self, profile: &PlanningProfile) -> Result<()> {
        if profile.planner_contract_ref() != self.catalog.planner_contract_ref()
            || profile.planner_implementation_ref() != self.catalog.planner_implementation_ref()
        {
            return Err(CertifyError::Planning(
                "planning profile selects a different planner".to_owned(),
            ));
        }
        for policy_ref in profile.framework_policy_refs() {
            self.catalog.policy(policy_ref)?;
        }
        Ok(())
    }

    fn reserve_framework(
        &self,
        index: usize,
        base_path: CanonicalExpansionPath,
        state: &QualifiedStateDefinition,
        applicable: &[(usize, QualifiedFrameworkPolicy)],
        pending: &mut Vec<PendingNode>,
    ) -> Result<ReservedFrameworkExpansion> {
        let Some((profile_ordinal, policy)) = applicable.get(index) else {
            return self
                .reserve_state(base_path, state, pending)
                .map(ReservedFrameworkExpansion::Base);
        };
        let inner = self.reserve_framework(
            index + 1,
            base_path.with_step(CanonicalExpansionStep::FrameworkProtected {
                policy_ref: policy.policy_ref().clone(),
                policy_ordinal: ordinal(*profile_ordinal)?,
            })?,
            state,
            applicable,
            pending,
        )?;
        let mut pre = Vec::new();
        for (state_ordinal, template) in policy.pre_nodes().iter().enumerate() {
            let support = self.catalog.state(template.state_contract_ref())?;
            pre.push(ReservedTemplateExpansion {
                template: template.clone(),
                expansion: self.reserve_state(
                    base_path.with_step(CanonicalExpansionStep::FrameworkPre {
                        policy_ref: policy.policy_ref().clone(),
                        policy_ordinal: ordinal(*profile_ordinal)?,
                        state_ordinal: ordinal(state_ordinal)?,
                    })?,
                    support,
                    pending,
                )?,
            });
        }
        let mut post = Vec::new();
        for (state_ordinal, template) in policy.post_nodes().iter().enumerate() {
            let support = self.catalog.state(template.state_contract_ref())?;
            post.push(ReservedTemplateExpansion {
                template: template.clone(),
                expansion: self.reserve_state(
                    base_path.with_step(CanonicalExpansionStep::FrameworkPost {
                        policy_ref: policy.policy_ref().clone(),
                        policy_ordinal: ordinal(*profile_ordinal)?,
                        state_ordinal: ordinal(state_ordinal)?,
                    })?,
                    support,
                    pending,
                )?,
            });
        }
        Ok(ReservedFrameworkExpansion::Policy {
            inner: Box::new(inner),
            pre,
            post,
        })
    }

    fn bind_framework(
        &self,
        occurrence: ReservedFrameworkExpansion,
        boundary: &PendingNodeBindings,
        entry: &QualifiedEntryPointDefinition,
        pending: &mut [PendingNode],
    ) -> Result<()> {
        match occurrence {
            ReservedFrameworkExpansion::Base(expansion) => {
                self.bind_state(expansion, boundary.clone(), entry, pending)
            }
            ReservedFrameworkExpansion::Policy { inner, pre, post } => {
                let protected = inner.effective();
                self.bind_framework(*inner, boundary, entry, pending)?;
                let relative = pre
                    .iter()
                    .chain(&post)
                    .map(|reserved| {
                        (
                            reserved.template.local_key().clone(),
                            reserved.expansion.effective,
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                for reserved in pre.into_iter().chain(post) {
                    let bindings = self.support_bindings(
                        &reserved.template,
                        boundary,
                        protected,
                        &relative,
                        entry,
                        pending,
                    )?;
                    self.bind_state(reserved.expansion, bindings, entry, pending)?;
                }
                Ok(())
            }
        }
    }

    fn reserve_state(
        &self,
        base_path: CanonicalExpansionPath,
        state: &QualifiedStateDefinition,
        pending: &mut Vec<PendingNode>,
    ) -> Result<ReservedStateExpansion> {
        let Some(executor) = self.catalog.executor_for_state(state.contract())? else {
            let index = push_pending(pending, base_path, state);
            return Ok(ReservedStateExpansion {
                protected: index,
                effective: index,
                fragments: Vec::new(),
            });
        };
        match executor {
            QualifiedExecutorExpansion::Leaf { .. } => {
                let index = push_pending(pending, base_path, state);
                Ok(ReservedStateExpansion {
                    protected: index,
                    effective: index,
                    fragments: Vec::new(),
                })
            }
            QualifiedExecutorExpansion::Chain {
                executor_contract_ref,
                pre_nodes,
                post_nodes,
            } => {
                let mut fragments = Vec::new();
                for (state_ordinal, template) in pre_nodes.iter().enumerate() {
                    fragments.push(ReservedLeafTemplate {
                        template: template.clone(),
                        index: self.reserve_executor_fragment(
                            base_path.with_step(CanonicalExpansionStep::ExecutorPre {
                                executor_contract_ref: executor_contract_ref.clone(),
                                state_ordinal: ordinal(state_ordinal)?,
                            })?,
                            template,
                            pending,
                        )?,
                    });
                }
                let protected = push_pending(
                    pending,
                    base_path.with_step(CanonicalExpansionStep::ExecutorProtected {
                        executor_contract_ref: executor_contract_ref.clone(),
                    })?,
                    state,
                );
                for (state_ordinal, template) in post_nodes.iter().enumerate() {
                    fragments.push(ReservedLeafTemplate {
                        template: template.clone(),
                        index: self.reserve_executor_fragment(
                            base_path.with_step(CanonicalExpansionStep::ExecutorPost {
                                executor_contract_ref: executor_contract_ref.clone(),
                                state_ordinal: ordinal(state_ordinal)?,
                            })?,
                            template,
                            pending,
                        )?,
                    });
                }
                let effective = fragments
                    .iter()
                    .rev()
                    .find(|fragment| {
                        post_nodes
                            .iter()
                            .any(|template| template.local_key() == fragment.template.local_key())
                    })
                    .map_or(protected, |fragment| fragment.index);
                Ok(ReservedStateExpansion {
                    protected,
                    effective,
                    fragments,
                })
            }
        }
    }

    fn reserve_executor_fragment(
        &self,
        path: CanonicalExpansionPath,
        template: &QualifiedSupportNodeTemplate,
        pending: &mut Vec<PendingNode>,
    ) -> Result<usize> {
        let state = self.catalog.state(template.state_contract_ref())?;
        if self
            .catalog
            .executor_for_state(state.contract())?
            .is_some_and(|executor| !matches!(executor, QualifiedExecutorExpansion::Leaf { .. }))
        {
            return Err(CertifyError::Planning(
                "executor fragment selects a non-leaf executor".to_owned(),
            ));
        }
        Ok(push_pending(pending, path, state))
    }

    fn bind_state(
        &self,
        expansion: ReservedStateExpansion,
        boundary: PendingNodeBindings,
        entry: &QualifiedEntryPointDefinition,
        pending: &mut [PendingNode],
    ) -> Result<()> {
        set_bindings(pending, expansion.protected, boundary.clone())?;
        let relative = expansion
            .fragments
            .iter()
            .map(|fragment| (fragment.template.local_key().clone(), fragment.index))
            .collect::<BTreeMap<_, _>>();
        for fragment in expansion.fragments {
            let bindings = self.support_bindings(
                &fragment.template,
                &boundary,
                expansion.protected,
                &relative,
                entry,
                pending,
            )?;
            set_bindings(pending, fragment.index, bindings)?;
        }
        Ok(())
    }

    fn authored_bindings(
        &self,
        entry: &QualifiedEntryPointDefinition,
        authored: &CanonicalAuthoredProgram,
        node: &AuthoredNode,
        effective: &BTreeMap<StableId, usize>,
        pending: &[PendingNode],
    ) -> Result<PendingNodeBindings> {
        let state = self.catalog.state(node.state_contract_ref())?.contract();
        let config = PendingFrameBinding {
            value_contract: state.config_contract().clone(),
            source: self.authored_config_source(
                entry,
                node.config_ref(),
                state.config_contract(),
            )?,
        };
        let context = match (state.context_contract(), node.context_ref()) {
            (None, None) => None,
            (Some(contract), Some(value_ref)) => {
                value_ref
                    .validate_contract(contract)
                    .map_err(|error| CertifyError::Planning(error.to_string()))?;
                if entry
                    .input_contract()
                    .context_contract()
                    .and_then(|source| source.selected_contract(None))
                    != Some(contract)
                    || !matches!(
                        value_ref
                            .fields()
                            .and_then(|fields| fields.producer_binding.fields())
                            .map_err(|error| CertifyError::Planning(error.to_string()))?,
                        ProducerBindingFields::SourceRun { .. }
                    )
                {
                    return Err(CertifyError::Planning(
                        "authored context differs from the entry-point context contract".to_owned(),
                    ));
                }
                Some(PendingFrameBinding {
                    value_contract: contract.clone(),
                    source: PendingSource::static_source(
                        CertifiedSourceSelector::Context {
                            source_field_path: None,
                        },
                        contract.clone(),
                    ),
                })
            }
            _ => {
                return Err(CertifyError::Planning(
                    "authored context presence differs from the state contract".to_owned(),
                ));
            }
        };

        let mut authored_inputs = BTreeMap::<u32, Vec<(u32, &AuthoredSourceSelector)>>::new();
        for binding in authored
            .input_bindings()
            .iter()
            .filter(|binding| binding.consumer_key() == node.stable_key())
        {
            authored_inputs
                .entry(binding.consumer_input_ordinal())
                .or_default()
                .push((binding.source_ordinal(), binding.source()));
        }
        if authored_inputs.len() != state.input_destinations().len() {
            return Err(CertifyError::Planning(
                "authored inputs are not total for the qualified state".to_owned(),
            ));
        }
        let mut input_bindings = Vec::with_capacity(state.input_destinations().len());
        for (index, destination) in state.input_destinations().iter().enumerate() {
            let input_ordinal = ordinal(index)?;
            let mut sources = authored_inputs.remove(&input_ordinal).ok_or_else(|| {
                CertifyError::Planning(
                    "authored input omits a qualified destination ordinal".to_owned(),
                )
            })?;
            sources.sort_by_key(|(source_ordinal, _)| *source_ordinal);
            if sources
                .iter()
                .enumerate()
                .any(|(source_ordinal, (actual, _))| {
                    *actual != u32::try_from(source_ordinal).unwrap_or(u32::MAX)
                })
            {
                return Err(CertifyError::Planning(
                    "authored input source ordinals are not dense".to_owned(),
                ));
            }
            let ordered_sources = sources
                .into_iter()
                .map(|(_, source)| {
                    self.authored_source(entry, source, effective, pending, destination)
                })
                .collect::<Result<Vec<_>>>()?;
            input_bindings.push(PendingInputBinding {
                destination_field_path: destination.destination_field_path().clone(),
                destination: destination.destination().clone(),
                value_contract: destination.value_contract().clone(),
                ordered_sources,
            });
        }
        Ok(PendingNodeBindings {
            state_contract_ref: state.state_contract_ref().clone(),
            config,
            context,
            input_bindings,
        })
    }

    fn authored_config_source(
        &self,
        entry: &QualifiedEntryPointDefinition,
        value_ref: &ValueRef,
        expected: &RetainedValueContract,
    ) -> Result<PendingSource> {
        value_ref
            .validate_contract(expected)
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        let producer = value_ref
            .fields()
            .and_then(|fields| fields.producer_binding.fields())
            .map_err(|error| CertifyError::Planning(error.to_string()))?;
        let selector = match producer {
            ProducerBindingFields::ConfiguredValue { entry_point_id, .. }
                if &entry_point_id == entry.entry_point().entry_point_id()
                    && entry
                        .input_contract()
                        .configured_value_contract()
                        .selected_contract(None)
                        == Some(expected) =>
            {
                CertifiedSourceSelector::Config {
                    source_field_path: None,
                }
            }
            ProducerBindingFields::QualifiedSupport { field_path, .. } => {
                let authored = AuthoredSourceSelector::QualifiedSupport {
                    member_path: field_path.clone(),
                    source_field_path: None,
                };
                if entry.authored_source_contract(&authored) != Some(expected) {
                    return Err(CertifyError::Planning(
                        "authored config support root differs from its qualified contract"
                            .to_owned(),
                    ));
                }
                CertifiedSourceSelector::QualifiedSupport {
                    member_path: field_path,
                    source_field_path: None,
                }
            }
            _ => {
                return Err(CertifyError::Planning(
                    "authored config has an inadmissible producer binding".to_owned(),
                ));
            }
        };
        Ok(PendingSource::static_source(selector, expected.clone()))
    }

    fn authored_source(
        &self,
        entry: &QualifiedEntryPointDefinition,
        source: &AuthoredSourceSelector,
        effective: &BTreeMap<StableId, usize>,
        pending: &[PendingNode],
        destination: &QualifiedInputContract,
    ) -> Result<PendingSource> {
        let pending_source = match source {
            AuthoredSourceSelector::NodeOutput {
                producer_key,
                producer_output_ordinal,
                source_field_path,
            } => {
                let index = *effective.get(producer_key).ok_or_else(|| {
                    CertifyError::Planning(format!(
                        "authored source names unknown producer {producer_key}"
                    ))
                })?;
                let contract = pending[index]
                    .state
                    .output_source(*producer_output_ordinal)
                    .and_then(|source| source.selected_contract(source_field_path.as_ref()))
                    .ok_or_else(|| {
                        CertifyError::Planning(
                            "authored output source has no qualified projection".to_owned(),
                        )
                    })?
                    .clone();
                PendingSource {
                    selector: PendingSelector::NodeOutput {
                        index,
                        output_ordinal: *producer_output_ordinal,
                        source_field_path: source_field_path.clone(),
                    },
                    value_contract: contract,
                }
            }
            _ => {
                let contract = entry.authored_source_contract(source).ok_or_else(|| {
                    CertifyError::Planning(
                        "authored non-node source has no qualified projection".to_owned(),
                    )
                })?;
                PendingSource::static_source(certified_authored_source(source)?, contract.clone())
            }
        };
        if &pending_source.value_contract != destination.value_contract() {
            return Err(CertifyError::Planning(
                "authored source contract differs from its input destination".to_owned(),
            ));
        }
        Ok(pending_source)
    }

    fn support_bindings(
        &self,
        template: &QualifiedSupportNodeTemplate,
        boundary: &PendingNodeBindings,
        protected: usize,
        relative: &BTreeMap<StableAuthorKey, usize>,
        entry: &QualifiedEntryPointDefinition,
        pending: &[PendingNode],
    ) -> Result<PendingNodeBindings> {
        let state = self
            .catalog
            .state(template.state_contract_ref())?
            .contract();
        let protected_state = self.catalog.state(&boundary.state_contract_ref)?.contract();
        let config_sources = self.support_sources(
            template.config_source(),
            boundary,
            protected_state,
            protected,
            relative,
            entry,
            pending,
        )?;
        let [config_source] = config_sources.as_slice() else {
            return Err(CertifyError::Planning(
                "support config source must resolve exactly once".to_owned(),
            ));
        };
        if &config_source.value_contract != state.config_contract() {
            return Err(CertifyError::Planning(
                "support config source contract mismatch".to_owned(),
            ));
        }
        let config = PendingFrameBinding {
            value_contract: state.config_contract().clone(),
            source: config_source.clone(),
        };
        let context = match (template.context_source(), state.context_contract()) {
            (None, None) => None,
            (Some(source), Some(contract)) => {
                let sources = self.support_sources(
                    source,
                    boundary,
                    protected_state,
                    protected,
                    relative,
                    entry,
                    pending,
                )?;
                let [source] = sources.as_slice() else {
                    return Err(CertifyError::Planning(
                        "support context source must resolve exactly once".to_owned(),
                    ));
                };
                if &source.value_contract != contract {
                    return Err(CertifyError::Planning(
                        "support context source contract mismatch".to_owned(),
                    ));
                }
                Some(PendingFrameBinding {
                    value_contract: contract.clone(),
                    source: source.clone(),
                })
            }
            _ => {
                return Err(CertifyError::Planning(
                    "support context presence differs from its qualified state".to_owned(),
                ));
            }
        };
        if template.input_bindings().len() != state.input_destinations().len() {
            return Err(CertifyError::Planning(
                "support input templates are not exact-total".to_owned(),
            ));
        }
        let mut input_bindings = Vec::with_capacity(template.input_bindings().len());
        for (input, destination) in template
            .input_bindings()
            .iter()
            .zip(state.input_destinations())
        {
            if input.destination_field_path() != destination.destination_field_path() {
                return Err(CertifyError::Planning(
                    "support input destination differs from its qualified state".to_owned(),
                ));
            }
            let ordered_sources = input
                .ordered_sources()
                .iter()
                .map(|source| {
                    self.support_sources(
                        source,
                        boundary,
                        protected_state,
                        protected,
                        relative,
                        entry,
                        pending,
                    )
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if ordered_sources.is_empty()
                || ordered_sources
                    .iter()
                    .any(|source| &source.value_contract != destination.value_contract())
            {
                return Err(CertifyError::Planning(
                    "support source contract differs from its input destination".to_owned(),
                ));
            }
            input_bindings.push(PendingInputBinding {
                destination_field_path: destination.destination_field_path().clone(),
                destination: destination.destination().clone(),
                value_contract: destination.value_contract().clone(),
                ordered_sources,
            });
        }
        Ok(PendingNodeBindings {
            state_contract_ref: state.state_contract_ref().clone(),
            config,
            context,
            input_bindings,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn support_sources(
        &self,
        source: &QualifiedSupportSourceTemplate,
        boundary: &PendingNodeBindings,
        boundary_state: &QualifiedStateContract,
        protected: usize,
        relative: &BTreeMap<StableAuthorKey, usize>,
        entry: &QualifiedEntryPointDefinition,
        pending: &[PendingNode],
    ) -> Result<Vec<PendingSource>> {
        match source {
            QualifiedSupportSourceTemplate::BoundaryInput {
                destination_field_path,
                source_field_path,
            } => {
                let input = boundary
                    .input_bindings
                    .iter()
                    .find(|input| &input.destination_field_path == destination_field_path)
                    .ok_or_else(|| {
                        CertifyError::Planning(
                            "support boundary source names an unknown destination".to_owned(),
                        )
                    })?;
                let contract = boundary_state
                    .input_destinations()
                    .iter()
                    .find(|input| input.destination_field_path() == destination_field_path)
                    .and_then(|input| {
                        input
                            .source_contract()
                            .selected_contract(source_field_path.as_ref())
                    })
                    .ok_or_else(|| {
                        CertifyError::Planning(
                            "support boundary source has no qualified projection".to_owned(),
                        )
                    })?;
                input
                    .ordered_sources
                    .iter()
                    .map(|source| source.project(source_field_path.as_ref(), contract.clone()))
                    .collect()
            }
            QualifiedSupportSourceTemplate::NodeOutput {
                producer,
                output_ordinal,
                source_field_path,
            } => {
                let index = relative_index(producer, protected, relative)?;
                let contract = pending[index]
                    .state
                    .output_source(*output_ordinal)
                    .and_then(|source| source.selected_contract(source_field_path.as_ref()))
                    .ok_or_else(|| {
                        CertifyError::Planning(
                            "support output source has no qualified projection".to_owned(),
                        )
                    })?
                    .clone();
                Ok(vec![PendingSource {
                    selector: PendingSelector::NodeOutput {
                        index,
                        output_ordinal: *output_ordinal,
                        source_field_path: source_field_path.clone(),
                    },
                    value_contract: contract,
                }])
            }
            QualifiedSupportSourceTemplate::NodeFact {
                producer,
                emission_ordinal,
            } => {
                let index = relative_index(producer, protected, relative)?;
                let slot = pending[index]
                    .state
                    .settlement_contract()
                    .invariant_fact_slot(*emission_ordinal)
                    .map_err(|error| CertifyError::Planning(error.to_string()))?;
                Ok(vec![PendingSource {
                    selector: PendingSelector::NodeFact {
                        index,
                        emission_ordinal: *emission_ordinal,
                    },
                    value_contract: slot.response_contract().clone(),
                }])
            }
            QualifiedSupportSourceTemplate::Config { source_field_path } => root_source(
                entry.input_contract().configured_value_contract(),
                source_field_path.as_ref(),
                CertifiedSourceSelector::Config {
                    source_field_path: source_field_path.clone(),
                },
            ),
            QualifiedSupportSourceTemplate::Seed { source_field_path } => {
                let source_contract = entry.input_contract().seed_contract().ok_or_else(|| {
                    CertifyError::Planning("support source requires an absent seed root".to_owned())
                })?;
                root_source(
                    source_contract,
                    source_field_path.as_ref(),
                    CertifiedSourceSelector::Seed {
                        source_field_path: source_field_path.clone(),
                    },
                )
            }
            QualifiedSupportSourceTemplate::Context { source_field_path } => {
                let source_contract =
                    entry.input_contract().context_contract().ok_or_else(|| {
                        CertifyError::Planning(
                            "support source requires an absent context root".to_owned(),
                        )
                    })?;
                root_source(
                    source_contract,
                    source_field_path.as_ref(),
                    CertifiedSourceSelector::Context {
                        source_field_path: source_field_path.clone(),
                    },
                )
            }
        }
    }

    fn public_output_contract(
        &self,
        entry: &QualifiedEntryPointDefinition,
        authored: &CanonicalAuthoredProgram,
        effective: &BTreeMap<StableId, usize>,
        ids: &[NodeId],
        nodes: &[CertifiedNodeContract],
    ) -> Result<PublicOutputContract> {
        let bindings = authored
            .public_output_bindings()
            .iter()
            .map(|binding| {
                let index = *effective.get(binding.producer_key()).ok_or_else(|| {
                    CertifyError::Planning(
                        "public output producer is not an authored occurrence".to_owned(),
                    )
                })?;
                let state = self.catalog.state(nodes[index].state_contract_ref())?;
                let contract = state
                    .contract()
                    .output_source(binding.producer_output_ordinal())
                    .and_then(|source| source.selected_contract(binding.source_field_path()))
                    .ok_or_else(|| {
                        CertifyError::Planning(
                            "public output has no qualified source projection".to_owned(),
                        )
                    })?;
                Ok(CertifiedOutputBinding::new(
                    FieldPath::new(binding.field().as_str())
                        .map_err(|error| CertifyError::Planning(error.to_string()))?,
                    ids[index].clone(),
                    binding.producer_output_ordinal(),
                    binding.source_field_path().cloned(),
                    contract.clone(),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        PublicOutputContract::new(entry.public_output_contract().clone(), bindings)
            .map_err(Into::into)
    }
}

fn root_source(
    source_contract: &QualifiedSourceContract,
    source_field_path: Option<&FieldPath>,
    selector: CertifiedSourceSelector,
) -> Result<Vec<PendingSource>> {
    let contract = source_contract
        .selected_contract(source_field_path)
        .ok_or_else(|| {
            CertifyError::Planning("root source has no qualified projection".to_owned())
        })?;
    Ok(vec![PendingSource::static_source(
        selector,
        contract.clone(),
    )])
}

fn relative_index(
    producer: &QualifiedSupportNodeSelector,
    protected: usize,
    relative: &BTreeMap<StableAuthorKey, usize>,
) -> Result<usize> {
    match producer {
        QualifiedSupportNodeSelector::Protected => Ok(protected),
        QualifiedSupportNodeSelector::Support(local_key) => {
            relative.get(local_key).copied().ok_or_else(|| {
                CertifyError::Planning("support source names an unknown local node".to_owned())
            })
        }
    }
}

fn certified_authored_source(source: &AuthoredSourceSelector) -> Result<CertifiedSourceSelector> {
    Ok(match source {
        AuthoredSourceSelector::RunAdmission { source_field_path } => {
            CertifiedSourceSelector::RunAdmission {
                source_field_path: source_field_path.clone(),
            }
        }
        AuthoredSourceSelector::Config { source_field_path } => CertifiedSourceSelector::Config {
            source_field_path: source_field_path.clone(),
        },
        AuthoredSourceSelector::QualifiedSupport {
            member_path,
            source_field_path,
        } => CertifiedSourceSelector::QualifiedSupport {
            member_path: member_path.clone(),
            source_field_path: source_field_path.clone(),
        },
        AuthoredSourceSelector::Seed { source_field_path } => CertifiedSourceSelector::Seed {
            source_field_path: source_field_path.clone(),
        },
        AuthoredSourceSelector::Context { source_field_path } => CertifiedSourceSelector::Context {
            source_field_path: source_field_path.clone(),
        },
        AuthoredSourceSelector::CrossRunEffectiveOutput { source_field_path } => {
            CertifiedSourceSelector::CrossRunEffectiveOutput {
                source_field_path: source_field_path.clone(),
            }
        }
        AuthoredSourceSelector::CrossRunEvidence {
            source_field_path,
            certified_evidence_role_ref,
        } => CertifiedSourceSelector::CrossRunEvidence {
            source_field_path: source_field_path.clone(),
            certified_evidence_role_ref: certified_evidence_role_ref.clone(),
        },
        AuthoredSourceSelector::NodeOutput { .. } => {
            return Err(CertifyError::Planning(
                "node output requires effective-occurrence resolution".to_owned(),
            ));
        }
    })
}

fn ordinal(value: usize) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| CertifyError::Planning("planner ordinal exceeds u32".to_owned()))
}

fn authored_path(
    node: &AuthoredNode,
    all_nodes: &[AuthoredNode],
) -> Result<CanonicalExpansionPath> {
    let mut steps = vec![CanonicalExpansionStep::EntryPoint];
    for (depth, stable_key) in node.child_path().iter().enumerate() {
        let parent = &node.child_path()[..depth];
        let siblings = all_nodes
            .iter()
            .filter(|candidate| {
                candidate.child_path().len() > depth && candidate.child_path()[..depth] == *parent
            })
            .map(|candidate| candidate.child_path()[depth].clone())
            .collect::<BTreeSet<_>>();
        let sibling_ordinal = siblings
            .iter()
            .position(|candidate| candidate == stable_key)
            .ok_or_else(|| {
                CertifyError::Planning(format!("unresolved child path component {stable_key}"))
            })?;
        steps.push(CanonicalExpansionStep::NestedChild {
            stable_key: stable_key.clone(),
            ordinal: ordinal(sibling_ordinal)?,
        });
    }
    let siblings = all_nodes
        .iter()
        .filter(|candidate| {
            candidate.child_path() == node.child_path() && candidate.base_kind() == node.base_kind()
        })
        .map(|candidate| candidate.local_stable_key().clone())
        .collect::<BTreeSet<_>>();
    let sibling_ordinal = siblings
        .iter()
        .position(|candidate| candidate == node.local_stable_key())
        .ok_or_else(|| {
            CertifyError::Planning(format!(
                "unresolved authored occurrence {}",
                node.stable_key()
            ))
        })?;
    steps.push(match node.base_kind() {
        AuthoredBaseKind::Authored => CanonicalExpansionStep::Authored {
            stable_key: node.local_stable_key().clone(),
            ordinal: ordinal(sibling_ordinal)?,
        },
        AuthoredBaseKind::Bridge => CanonicalExpansionStep::Bridge {
            stable_key: node.local_stable_key().clone(),
            ordinal: ordinal(sibling_ordinal)?,
        },
    });
    CanonicalExpansionPath::new(steps).map_err(Into::into)
}

struct PendingNode {
    path: CanonicalExpansionPath,
    state: QualifiedStateContract,
    bindings: Option<PendingNodeBindings>,
}

#[derive(Clone)]
struct PendingNodeBindings {
    state_contract_ref: ContentRef,
    config: PendingFrameBinding,
    context: Option<PendingFrameBinding>,
    input_bindings: Vec<PendingInputBinding>,
}

#[derive(Clone)]
struct PendingFrameBinding {
    value_contract: RetainedValueContract,
    source: PendingSource,
}

#[derive(Clone)]
struct PendingInputBinding {
    destination_field_path: FieldPath,
    destination: mfm_spec::CertifiedInputDestination,
    value_contract: RetainedValueContract,
    ordered_sources: Vec<PendingSource>,
}

#[derive(Clone)]
struct PendingSource {
    selector: PendingSelector,
    value_contract: RetainedValueContract,
}

#[derive(Clone)]
enum PendingSelector {
    Static(CertifiedSourceSelector),
    NodeOutput {
        index: usize,
        output_ordinal: u32,
        source_field_path: Option<FieldPath>,
    },
    NodeFact {
        index: usize,
        emission_ordinal: u32,
    },
}

impl PendingSource {
    fn static_source(
        selector: CertifiedSourceSelector,
        value_contract: RetainedValueContract,
    ) -> Self {
        Self {
            selector: PendingSelector::Static(selector),
            value_contract,
        }
    }

    fn project(
        &self,
        extra: Option<&FieldPath>,
        value_contract: RetainedValueContract,
    ) -> Result<Self> {
        let selector = match (&self.selector, extra) {
            (selector, None) => selector.clone(),
            (PendingSelector::NodeFact { .. }, Some(_)) => {
                return Err(CertifyError::Planning(
                    "fact sources do not admit nested field projection".to_owned(),
                ));
            }
            (
                PendingSelector::NodeOutput {
                    index,
                    output_ordinal,
                    source_field_path,
                },
                Some(extra),
            ) => PendingSelector::NodeOutput {
                index: *index,
                output_ordinal: *output_ordinal,
                source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
            },
            (PendingSelector::Static(source), Some(extra)) => {
                PendingSelector::Static(project_certified_source(source, extra)?)
            }
        };
        Ok(Self {
            selector,
            value_contract,
        })
    }

    fn finalize(self, ids: &[NodeId]) -> Result<CertifiedSourceSelector> {
        match self.selector {
            PendingSelector::Static(source) => Ok(source),
            PendingSelector::NodeOutput {
                index,
                output_ordinal,
                source_field_path,
            } => Ok(CertifiedSourceSelector::NodeOutput {
                producer_node_id: ids.get(index).cloned().ok_or_else(|| {
                    CertifyError::Planning("pending output producer is unresolved".to_owned())
                })?,
                output_ordinal,
                source_field_path,
            }),
            PendingSelector::NodeFact {
                index,
                emission_ordinal,
            } => Ok(CertifiedSourceSelector::NodeFact {
                producer_node_id: ids.get(index).cloned().ok_or_else(|| {
                    CertifyError::Planning("pending fact producer is unresolved".to_owned())
                })?,
                emission_ordinal,
            }),
        }
    }
}

fn project_certified_source(
    source: &CertifiedSourceSelector,
    extra: &FieldPath,
) -> Result<CertifiedSourceSelector> {
    Ok(match source {
        CertifiedSourceSelector::RunAdmission { source_field_path } => {
            CertifiedSourceSelector::RunAdmission {
                source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
            }
        }
        CertifiedSourceSelector::Config { source_field_path } => CertifiedSourceSelector::Config {
            source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
        },
        CertifiedSourceSelector::QualifiedSupport {
            member_path,
            source_field_path,
        } => CertifiedSourceSelector::QualifiedSupport {
            member_path: member_path.clone(),
            source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
        },
        CertifiedSourceSelector::Seed { source_field_path } => CertifiedSourceSelector::Seed {
            source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
        },
        CertifiedSourceSelector::Context { source_field_path } => {
            CertifiedSourceSelector::Context {
                source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
            }
        }
        CertifiedSourceSelector::NodeOutput {
            producer_node_id,
            output_ordinal,
            source_field_path,
        } => CertifiedSourceSelector::NodeOutput {
            producer_node_id: producer_node_id.clone(),
            output_ordinal: *output_ordinal,
            source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
        },
        CertifiedSourceSelector::CrossRunEffectiveOutput { source_field_path } => {
            CertifiedSourceSelector::CrossRunEffectiveOutput {
                source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
            }
        }
        CertifiedSourceSelector::CrossRunEvidence {
            source_field_path,
            certified_evidence_role_ref,
        } => CertifiedSourceSelector::CrossRunEvidence {
            source_field_path: Some(join_field_paths(source_field_path.as_ref(), extra)?),
            certified_evidence_role_ref: certified_evidence_role_ref.clone(),
        },
        CertifiedSourceSelector::NodeFact { .. } => {
            return Err(CertifyError::Planning(
                "fact sources do not admit nested field projection".to_owned(),
            ));
        }
    })
}

fn join_field_paths(prefix: Option<&FieldPath>, suffix: &FieldPath) -> Result<FieldPath> {
    let joined = prefix.map_or_else(
        || suffix.as_str().to_owned(),
        |prefix| format!("{}.{}", prefix.as_str(), suffix.as_str()),
    );
    FieldPath::new(joined).map_err(|error| CertifyError::Planning(error.to_string()))
}

struct ReservedStateExpansion {
    protected: usize,
    effective: usize,
    fragments: Vec<ReservedLeafTemplate>,
}

struct ReservedLeafTemplate {
    template: QualifiedSupportNodeTemplate,
    index: usize,
}

struct ReservedTemplateExpansion {
    template: QualifiedSupportNodeTemplate,
    expansion: ReservedStateExpansion,
}

enum ReservedFrameworkExpansion {
    Base(ReservedStateExpansion),
    Policy {
        inner: Box<ReservedFrameworkExpansion>,
        pre: Vec<ReservedTemplateExpansion>,
        post: Vec<ReservedTemplateExpansion>,
    },
}

impl ReservedFrameworkExpansion {
    fn effective(&self) -> usize {
        match self {
            Self::Base(expansion) => expansion.effective,
            Self::Policy { inner, post, .. } => post.last().map_or_else(
                || inner.effective(),
                |reserved| reserved.expansion.effective,
            ),
        }
    }
}

fn push_pending(
    pending: &mut Vec<PendingNode>,
    path: CanonicalExpansionPath,
    state: &QualifiedStateDefinition,
) -> usize {
    let index = pending.len();
    pending.push(PendingNode {
        path,
        state: state.contract().clone(),
        bindings: None,
    });
    index
}

fn set_bindings(
    pending: &mut [PendingNode],
    index: usize,
    bindings: PendingNodeBindings,
) -> Result<()> {
    let node = pending
        .get_mut(index)
        .ok_or_else(|| CertifyError::Planning("pending node is unresolved".to_owned()))?;
    if node.bindings.replace(bindings).is_some() {
        return Err(CertifyError::Planning(
            "pending node bindings were assigned more than once".to_owned(),
        ));
    }
    Ok(())
}

fn finalize_node(node: PendingNode, index: usize, ids: &[NodeId]) -> Result<CertifiedNodeContract> {
    let bindings = node
        .bindings
        .ok_or_else(|| CertifyError::Planning("pending node was never bound".to_owned()))?;
    let config_binding = CertifiedFrameBinding::new(
        bindings.config.value_contract,
        bindings.config.source.finalize(ids)?,
    );
    let context_binding = bindings
        .context
        .map(|binding| -> Result<CertifiedFrameBinding> {
            Ok(CertifiedFrameBinding::new(
                binding.value_contract,
                binding.source.finalize(ids)?,
            ))
        })
        .transpose()?;
    let input_bindings = bindings
        .input_bindings
        .into_iter()
        .map(|binding| {
            let ordered_sources = binding
                .ordered_sources
                .into_iter()
                .map(|source| source.finalize(ids))
                .collect::<Result<Vec<_>>>()?;
            CertifiedInputBinding::new(
                binding.destination_field_path,
                binding.destination,
                binding.value_contract,
                ordered_sources,
            )
            .map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    let certified = CertifiedNodeContract::new(
        node.path,
        node.state.state_contract_ref().clone(),
        config_binding,
        context_binding,
        node.state.input_contract().clone(),
        input_bindings,
        node.state.execution().clone(),
        node.state.settlement_contract().clone(),
    )?;
    if certified.node_id() != &ids[index] {
        return Err(CertifyError::Planning(
            "node identity changed during finalization".to_owned(),
        ));
    }
    Ok(certified)
}
