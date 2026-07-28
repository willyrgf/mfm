use std::collections::{btree_map::Entry, BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_capabilities::{CapabilityDescriptor, CapabilitySetDescriptor, CapabilitySpec};
use mfm_events::v1 as events;
use mfm_ids::{AdapterKind, AdapterVersion, DescriptorId, RuntimeBindingId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::framework::{
    framework_bridge_binding, framework_complete_run_binding, framework_public_output_binding,
    framework_resolve_saga_terminal_binding, framework_retention_manifest_binding,
};
use crate::spec_authority::{CurrentRuntimeSpecRef, CurrentSpecRead};
use crate::{
    CertifiedInvocationContext, ErasedRunCtx, ExecutableIdentityTemplate, PreInvocationRunCtx,
    Result, RunLaunchArtifact, RunLaunchEvidence, RunnerFactoryBinding, RuntimeError,
    StagedArtifact, StagedRetentionRefs,
};

/// Boxed future returned by an erased typed runner.
pub type ErasedRunnerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ErasedRunnerOutput>> + Send + 'a>>;

/// Boxed future returned by a pre-invocation resource-lane hook.
pub type PreInvocationRunnerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ErasedRunnerOutput>> + Send + 'a>>;

/// Boxed future returned by a runner ingress validator.
pub type RunnerIngressFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// Type-aware validator for context-bound state-output artifacts.
pub trait ContextOutputExtractor: Send + Sync {
    /// Validates decoded output context metadata against the certified output-cell context.
    fn validate_context_output(
        &self,
        cell_context: &spec::CellContextSpec,
        artifact: &store::ArtifactEvidenceRef,
        bytes: &[u8],
    ) -> Result<()>;
}

/// Pre-admission context supplied to runner ingress validators.
///
/// This context contains only certified launch authority and caller-supplied launch artifact bytes.
/// It exists so a runner can reject missing process-local capability before `RunAdmitted` is
/// appended, without reading mutable store state or executing the node.
pub struct RunnerIngressContext<'a> {
    runtime_spec: CurrentRuntimeSpecRef<'a>,
    node: &'a spec::NodeSpec,
    launch: &'a RunLaunchEvidence,
}

impl<'a> RunnerIngressContext<'a> {
    pub(crate) fn new<S>(
        runtime_spec: &'a S,
        node: &'a spec::NodeSpec,
        launch: &'a RunLaunchEvidence,
    ) -> Self
    where
        S: CurrentSpecRead + ?Sized,
    {
        Self {
            runtime_spec: runtime_spec.current_spec_ref(),
            node,
            launch,
        }
    }

    /// Returns the certified runtime spec being launched.
    pub fn runtime_spec(&self) -> CurrentRuntimeSpecRef<'_> {
        self.runtime_spec.current_spec_ref()
    }

    /// Returns the certified node bound to the runner.
    pub fn node(&self) -> &'a spec::NodeSpec {
        self.node
    }

    /// Returns certified transition-context authority for this ingress node.
    pub fn context(&self) -> Result<CertifiedInvocationContext> {
        CertifiedInvocationContext::for_node(&self.runtime_spec, self.node)
    }

    /// Returns the launch artifact matching this node's certified config ref.
    pub fn config_artifact(&self) -> Result<&'a RunLaunchArtifact> {
        self.config_artifact_for_node(self.node)
    }

    /// Returns the launch artifact matching another certified node's config ref.
    pub fn config_artifact_for_node(
        &self,
        node: &'a spec::NodeSpec,
    ) -> Result<&'a RunLaunchArtifact> {
        let config = &node.config_ref;
        self.launch
            .config_artifacts
            .iter()
            .find(|artifact| {
                artifact.evidence.artifact_id == config.artifact_id
                    && artifact.evidence.digest == config.digest
                    && artifact.evidence.byte_len == config.byte_len
                    && artifact.evidence.media_type == config.media_type
                    && artifact.evidence.schema_id.as_ref() == Some(&config.schema_id)
            })
            .ok_or_else(|| {
                RuntimeError::RunnerBinding(format!(
                    "missing launch config artifact for node {} schema {}",
                    node.node_id, config.schema_id
                ))
            })
    }
}

/// Object-safe erased runner boundary used after typed spec certification.
///
/// Runner selection is keyed by the certified node descriptor id. The runner receives only
/// store-verified input cell evidence and certified capability descriptors.
pub trait ErasedNodeRunner: Send + Sync {
    /// Validates process-local capability required to admit this certified node.
    fn validate_ingress<'a>(&'a self, _ctx: RunnerIngressContext<'a>) -> RunnerIngressFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    /// Returns type-aware output context validation for context-bound state outputs.
    fn context_output_extractor(&self) -> Option<&dyn ContextOutputExtractor> {
        None
    }

    /// Emits pre-invocation resource-lane claim evidence, if this runner owns such a claim.
    fn preclaim_resource_lane<'a>(
        &'a self,
        _ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async { Ok(ErasedRunnerOutput::new(Vec::new())) })
    }

    /// Executes one certified node attempt.
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a>;
}

/// Runner-owned payloads that may be proposed by domain execution.
///
/// Scheduler, framework lifecycle, artifact reference, retention, and run lifecycle events are
/// intentionally absent. The runtime middleware derives those authoritative payloads after
/// validating this runner-facing payload set against the certified spec and store projections.
///
/// Payload variants stay unboxed so runner output uses the same direct event payload shapes as the
/// kernel stream at the validation boundary.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerEventPayload {
    /// Cell produced terminal event.
    CellProduced(events::CellProduced),
    /// Cell skipped terminal event.
    CellSkipped(events::CellSkipped),
    /// Side-effect intent persisted event.
    SideEffectIntentPersisted(events::side_effect::IntentPersisted),
    /// Side-effect claim acquired event.
    SideEffectClaimed(events::side_effect::Claimed),
    /// Side-effect claim takeover event.
    SideEffectClaimTakenOver(events::side_effect::ClaimTakenOver),
    /// Resource lane claim intent before store fill.
    ResourceLaneClaimIntent(events::ResourceLaneClaimIntent),
    /// Resource lane release intent before store fill.
    ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent),
    /// Side-effect invocation prepared event.
    SideEffectInvocationPrepared(events::side_effect::InvocationPrepared),
    /// Side-effect invocation started event.
    SideEffectInvocationStarted(events::side_effect::InvocationStarted),
    /// Side-effect not-submitted proof event.
    SideEffectNotSubmittedProven(events::side_effect::NotSubmittedProven),
    /// Side-effect submission observed event.
    SideEffectSubmissionObserved(events::side_effect::SubmissionObserved),
    /// Side-effect submission unknown event.
    SideEffectSubmissionUnknown(events::side_effect::SubmissionUnknown),
    /// Side-effect receipt observed event.
    SideEffectReceiptObserved(events::side_effect::ReceiptObserved),
    /// Side-effect confirmation observed event.
    SideEffectConfirmationObserved(events::side_effect::ConfirmationObserved),
    /// Side-effect ambiguity event.
    SideEffectAmbiguous(events::side_effect::Ambiguous),
    /// Side-effect failure event.
    SideEffectFailed(events::side_effect::Failed),
    /// Public output produced event.
    PublicOutputProduced(events::PublicOutputProduced),
    /// Public output render failure audit event.
    PublicOutputRenderFailed(events::PublicOutputRenderFailed),
}

impl From<RunnerEventPayload> for events::KernelEventPayload {
    fn from(payload: RunnerEventPayload) -> Self {
        match payload {
            RunnerEventPayload::CellProduced(payload) => Self::CellProduced(payload),
            RunnerEventPayload::CellSkipped(payload) => Self::CellSkipped(payload),
            RunnerEventPayload::SideEffectIntentPersisted(payload) => {
                Self::SideEffectIntentPersisted(payload)
            }
            RunnerEventPayload::SideEffectClaimed(payload) => Self::SideEffectClaimed(payload),
            RunnerEventPayload::SideEffectClaimTakenOver(payload) => {
                Self::SideEffectClaimTakenOver(payload)
            }
            RunnerEventPayload::ResourceLaneClaimIntent(payload) => {
                Self::ResourceLaneClaimIntent(payload)
            }
            RunnerEventPayload::ResourceLaneReleaseIntent(payload) => {
                Self::ResourceLaneReleaseIntent(payload)
            }
            RunnerEventPayload::SideEffectInvocationPrepared(payload) => {
                Self::SideEffectInvocationPrepared(payload)
            }
            RunnerEventPayload::SideEffectInvocationStarted(payload) => {
                Self::SideEffectInvocationStarted(payload)
            }
            RunnerEventPayload::SideEffectNotSubmittedProven(payload) => {
                Self::SideEffectNotSubmittedProven(payload)
            }
            RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
                Self::SideEffectSubmissionObserved(payload)
            }
            RunnerEventPayload::SideEffectSubmissionUnknown(payload) => {
                Self::SideEffectSubmissionUnknown(payload)
            }
            RunnerEventPayload::SideEffectReceiptObserved(payload) => {
                Self::SideEffectReceiptObserved(payload)
            }
            RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
                Self::SideEffectConfirmationObserved(payload)
            }
            RunnerEventPayload::SideEffectAmbiguous(payload) => Self::SideEffectAmbiguous(payload),
            RunnerEventPayload::SideEffectFailed(payload) => Self::SideEffectFailed(payload),
            RunnerEventPayload::PublicOutputProduced(payload) => {
                Self::PublicOutputProduced(payload)
            }
            RunnerEventPayload::PublicOutputRenderFailed(payload) => {
                Self::PublicOutputRenderFailed(payload)
            }
        }
    }
}

/// One-shot process-local settlement carried with a runner output until its append outcome.
///
/// Dropping this value discards the transient authority captured by the callback. The runtime
/// invokes the callback only after the corresponding commit is durably appended.
pub struct RunnerOutputSettlement {
    on_appended: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl RunnerOutputSettlement {
    /// Creates a one-shot settlement callback for transient runner authority.
    pub fn on_appended(callback: impl FnOnce() + Send + 'static) -> Self {
        Self {
            on_appended: Some(Box::new(callback)),
        }
    }

    /// Executes the callback after the associated commit was durably appended.
    pub(crate) fn settle_appended(mut self) {
        if let Some(callback) = self.on_appended.take() {
            callback();
        }
    }
}

impl std::fmt::Debug for RunnerOutputSettlement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunnerOutputSettlement")
            .field("pending", &self.on_appended.is_some())
            .finish()
    }
}

/// Typed payload batch returned by an erased runner.
pub struct ErasedRunnerOutput {
    /// Staged artifacts or sealed finalized handles referenced by payloads.
    staged_artifacts: Vec<StagedArtifact>,
    /// Retention refs staged by the runner for scheduler-owned event binding.
    staged_retention_refs: Vec<StagedRetentionRefs>,
    /// Runtime-owned facts staged by a read reducer.
    read_facts: Vec<events::FactRecorded>,
    /// Runner-owned typed payloads to validate before runtime lifecycle derivation.
    payloads: Vec<RunnerEventPayload>,
    /// Process-local authority settled only after a successful durable append.
    settlement: Option<RunnerOutputSettlement>,
}

type ErasedRunnerOutputParts = (
    Vec<StagedArtifact>,
    Vec<StagedRetentionRefs>,
    Vec<events::FactRecorded>,
    Vec<RunnerEventPayload>,
    Option<RunnerOutputSettlement>,
);

impl std::fmt::Debug for ErasedRunnerOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ErasedRunnerOutput")
            .field("staged_artifacts", &self.staged_artifacts)
            .field("staged_retention_refs", &self.staged_retention_refs)
            .field("read_facts", &self.read_facts)
            .field("payloads", &self.payloads)
            .field("settlement", &self.settlement)
            .finish()
    }
}

impl ErasedRunnerOutput {
    pub(crate) fn from_parts(
        staged_artifacts: Vec<StagedArtifact>,
        staged_retention_refs: Vec<StagedRetentionRefs>,
        payloads: Vec<RunnerEventPayload>,
    ) -> Self {
        Self::from_parts_with_read_facts(
            staged_artifacts,
            staged_retention_refs,
            Vec::new(),
            payloads,
        )
    }

    pub(crate) fn from_parts_with_read_facts(
        staged_artifacts: Vec<StagedArtifact>,
        staged_retention_refs: Vec<StagedRetentionRefs>,
        read_facts: Vec<events::FactRecorded>,
        payloads: Vec<RunnerEventPayload>,
    ) -> Self {
        Self {
            staged_artifacts,
            staged_retention_refs,
            read_facts,
            payloads,
            settlement: None,
        }
    }

    pub(crate) fn with_settlement(mut self, settlement: RunnerOutputSettlement) -> Self {
        self.settlement = Some(settlement);
        self
    }

    /// Creates an output batch from payloads with no additional artifact evidence.
    pub fn new(payloads: Vec<RunnerEventPayload>) -> Self {
        Self::from_parts(Vec::new(), Vec::new(), payloads)
    }

    /// Returns staged artifacts or sealed finalized handles referenced by payloads.
    pub fn staged_artifacts(&self) -> &[StagedArtifact] {
        &self.staged_artifacts
    }

    /// Returns retention refs staged by the runner for scheduler-owned event binding.
    pub fn staged_retention_refs(&self) -> &[StagedRetentionRefs] {
        &self.staged_retention_refs
    }

    /// Returns runner-owned typed payloads to validate before runtime lifecycle derivation.
    pub fn payloads(&self) -> &[RunnerEventPayload] {
        &self.payloads
    }

    #[cfg(test)]
    pub(crate) fn payloads_mut(&mut self) -> &mut [RunnerEventPayload] {
        &mut self.payloads
    }

    pub(crate) fn into_parts(self) -> ErasedRunnerOutputParts {
        (
            self.staged_artifacts,
            self.staged_retention_refs,
            self.read_facts,
            self.payloads,
            self.settlement,
        )
    }
}

/// Registered erased runner binding for one certified state descriptor.
#[derive(Clone)]
pub struct ErasedRunnerBinding {
    pub(crate) descriptor_id: DescriptorId,
    pub(crate) factory_id: events::RunnerFactoryId,
    pub(crate) executable: events::ExecutableIdentity,
    pub(crate) runner: Arc<dyn ErasedNodeRunner>,
}

impl ErasedRunnerBinding {
    /// Creates a runner binding for a certified state descriptor id.
    pub fn new(
        descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<Self> {
        if executable.factory_id != factory_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "executable factory {} does not match binding factory {}",
                executable.factory_id, factory_id
            )));
        }
        Ok(Self {
            descriptor_id,
            factory_id,
            executable,
            runner,
        })
    }

    /// Certified descriptor id this binding executes.
    pub fn descriptor_id(&self) -> &DescriptorId {
        &self.descriptor_id
    }

    /// Runner factory id.
    pub fn factory_id(&self) -> &events::RunnerFactoryId {
        &self.factory_id
    }

    /// Executable identity for run-start evidence.
    pub fn executable(&self) -> &events::ExecutableIdentity {
        &self.executable
    }
}

/// Non-secret runtime identifier for one concrete capability implementation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapabilityImplementationId(RuntimeBindingId);

impl CapabilityImplementationId {
    /// Creates a checked runtime capability implementation id.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        RuntimeBindingId::new(&value).map(Self).map_err(|_| {
            RuntimeError::RunnerBinding(format!("invalid capability implementation id {value:?}"))
        })
    }

    /// Returns the stable runtime implementation id string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn runtime_binding_id(&self) -> &RuntimeBindingId {
        &self.0
    }
}

impl fmt::Display for CapabilityImplementationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Registered runtime binding between a certified capability descriptor and its implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityImplementationBinding {
    descriptor: CapabilityDescriptor,
    implementation_id: CapabilityImplementationId,
}

impl CapabilityImplementationBinding {
    /// Creates capability implementation binding evidence.
    pub fn new(
        descriptor: CapabilityDescriptor,
        implementation_id: CapabilityImplementationId,
    ) -> Self {
        Self {
            descriptor,
            implementation_id,
        }
    }

    /// Certified capability descriptor covered by this binding.
    pub fn descriptor(&self) -> &CapabilityDescriptor {
        &self.descriptor
    }

    /// Non-secret runtime implementation id selected for this capability.
    pub fn implementation_id(&self) -> &CapabilityImplementationId {
        &self.implementation_id
    }
}

/// Registered runtime executable evidence for one certified adapter binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterExecutableBinding {
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    executable: events::ExecutableIdentity,
}

impl AdapterExecutableBinding {
    /// Creates adapter executable binding evidence.
    pub fn new(
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        executable: events::ExecutableIdentity,
    ) -> Self {
        Self {
            adapter_kind,
            adapter_version,
            executable,
        }
    }

    /// Certified adapter kind covered by this executable.
    pub fn adapter_kind(&self) -> &AdapterKind {
        &self.adapter_kind
    }

    /// Certified adapter version covered by this executable.
    pub fn adapter_version(&self) -> &AdapterVersion {
        &self.adapter_version
    }

    /// Executable identity to bind into `RunAdmitted`.
    pub fn executable(&self) -> &events::ExecutableIdentity {
        &self.executable
    }
}

/// Registry of erased runners keyed by certified state descriptor id.
#[derive(Clone)]
pub struct ErasedRunnerRegistry {
    executable_identity_template: ExecutableIdentityTemplate,
    bindings: BTreeMap<DescriptorId, ErasedRunnerBinding>,
    side_effect_verify_bindings: BTreeMap<DescriptorId, ErasedFrameworkRunnerBinding>,
    capability_implementations: BTreeMap<(String, String), CapabilityImplementationBinding>,
    adapter_executables: BTreeMap<(String, String), AdapterExecutableBinding>,
}

#[derive(Clone)]
struct ErasedFrameworkRunnerBinding {
    factory_id: events::RunnerFactoryId,
    executable: events::ExecutableIdentity,
    runner: Arc<dyn ErasedNodeRunner>,
}

impl ErasedRunnerRegistry {
    /// Creates an empty registry bound to one explicitly attested executable identity.
    pub fn new(executable_identity_template: ExecutableIdentityTemplate) -> Self {
        Self {
            executable_identity_template,
            bindings: BTreeMap::new(),
            side_effect_verify_bindings: BTreeMap::new(),
            capability_implementations: BTreeMap::new(),
            adapter_executables: BTreeMap::new(),
        }
    }

    /// Mints a factory binding under this registry's executable identity.
    pub fn factory_binding(&self, factory_id: events::RunnerFactoryId) -> RunnerFactoryBinding {
        self.executable_identity_template
            .factory_binding(factory_id)
    }

    /// Validates exact runtime binding coverage for a published authoring catalog.
    ///
    /// The catalog supplies semantic keys only. Concrete capability implementation and executable
    /// identities remain process bindings validated independently by this registry.
    pub fn validate_authoring_catalog(
        &self,
        catalog: &mfm_certify::ProgramAuthoringCatalog,
    ) -> Result<()> {
        let domain_states = catalog
            .state_descriptors()
            .filter(|descriptor| !catalog.is_framework_state(&descriptor.descriptor_id))
            .collect::<Vec<_>>();
        if self.bindings.len() != domain_states.len() {
            return Err(runtime_catalog_mismatch(
                "runner bindings",
                domain_states.len(),
                self.bindings.len(),
            ));
        }
        for descriptor in domain_states {
            let Some(binding) = self.bindings.get(&descriptor.descriptor_id) else {
                return Err(RuntimeError::RunnerBinding(format!(
                    "missing runner binding for catalog state {}",
                    descriptor.descriptor_id
                )));
            };
            if binding.descriptor_id() != &descriptor.descriptor_id
                || binding.factory_id().as_str() != descriptor.runner
            {
                return Err(RuntimeError::RunnerBinding(format!(
                    "runner binding for catalog state {} has the wrong semantic key",
                    descriptor.descriptor_id
                )));
            }
            self.validate_executable(binding.factory_id(), binding.executable())?;
        }

        for descriptor in catalog
            .state_descriptors()
            .filter(|descriptor| catalog.is_framework_state(&descriptor.descriptor_id))
        {
            if !is_closed_framework_catalog_state(&descriptor.name)
                || self.bindings.contains_key(&descriptor.descriptor_id)
            {
                return Err(RuntimeError::RunnerBinding(format!(
                    "catalog framework state {} is not owned exclusively by the runtime bootstrap",
                    descriptor.descriptor_id
                )));
            }
        }

        let expected_capabilities = catalog.capability_descriptors().collect::<Vec<_>>();
        if self.capability_implementations.len() != expected_capabilities.len() {
            return Err(runtime_catalog_mismatch(
                "capability bindings",
                expected_capabilities.len(),
                self.capability_implementations.len(),
            ));
        }
        for descriptor in expected_capabilities {
            let key = capability_implementation_key(descriptor);
            match self.capability_implementations.get(&key) {
                Some(binding) if binding.descriptor() == descriptor => {}
                _ => {
                    return Err(RuntimeError::RunnerBinding(format!(
                        "capability binding {}:{} does not match the authoring catalog",
                        descriptor.kind, descriptor.version
                    )))
                }
            }
        }

        let expected_adapters = catalog.adapter_bindings().collect::<Vec<_>>();
        if self.adapter_executables.len() != expected_adapters.len() {
            return Err(runtime_catalog_mismatch(
                "adapter executable bindings",
                expected_adapters.len(),
                self.adapter_executables.len(),
            ));
        }
        for expected in expected_adapters {
            let key = adapter_executable_key(&expected.adapter_kind, &expected.adapter_version);
            let Some(binding) = self.adapter_executables.get(&key) else {
                return Err(RuntimeError::RunnerBinding(format!(
                    "missing adapter executable for catalog binding {}:{}",
                    expected.adapter_kind, expected.adapter_version
                )));
            };
            if binding.adapter_kind() != &expected.adapter_kind
                || binding.adapter_version() != &expected.adapter_version
            {
                return Err(RuntimeError::RunnerBinding(format!(
                    "adapter executable does not match catalog binding {}:{}",
                    expected.adapter_kind, expected.adapter_version
                )));
            }
            self.validate_executable(&binding.executable().factory_id, binding.executable())?;
        }

        let expected_side_effects = catalog
            .side_effect_state_descriptor_ids()
            .cloned()
            .collect::<BTreeSet<_>>();
        let actual_side_effects = self
            .side_effect_verify_bindings
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if actual_side_effects != expected_side_effects {
            return Err(RuntimeError::RunnerBinding(
                "side-effect verification bindings do not equal the authoring catalog".to_owned(),
            ));
        }
        Ok(())
    }

    /// Validates the implementation id selected by the same live object used for execution.
    pub fn validate_capability_implementation<C>(&self, implementation_id: &str) -> Result<()>
    where
        C: CapabilitySpec,
    {
        let descriptor =
            C::descriptor().map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?;
        let key = capability_implementation_key(&descriptor);
        let Some(binding) = self.capability_implementations.get(&key) else {
            return Err(RuntimeError::RunnerBinding(format!(
                "missing capability implementation for {}:{}",
                descriptor.kind, descriptor.version
            )));
        };
        if binding.descriptor() != &descriptor
            || binding.implementation_id().as_str() != implementation_id
        {
            return Err(RuntimeError::RunnerBinding(format!(
                "selected capability implementation does not match {}:{}",
                descriptor.kind, descriptor.version
            )));
        }
        Ok(())
    }

    /// Registers one erased runner binding.
    pub fn register(&mut self, binding: ErasedRunnerBinding) -> Result<()> {
        self.validate_executable(binding.factory_id(), binding.executable())?;
        if self
            .bindings
            .insert(binding.descriptor_id.clone(), binding)
            .is_some()
        {
            return Err(RuntimeError::RunnerBinding(
                "duplicate runner binding".to_owned(),
            ));
        }
        Ok(())
    }

    /// Registers the adapter-owned runner used by certified side-effect verify framework nodes
    /// for one certified side-effect submit descriptor.
    pub fn register_side_effect_verify_runner(
        &mut self,
        submit_descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<()> {
        if executable.factory_id != factory_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "side-effect verify executable factory {} does not match binding factory {}",
                executable.factory_id, factory_id
            )));
        }
        self.validate_executable(&factory_id, &executable)?;
        match self.side_effect_verify_bindings.entry(submit_descriptor_id) {
            Entry::Vacant(entry) => {
                entry.insert(ErasedFrameworkRunnerBinding {
                    factory_id,
                    executable,
                    runner,
                });
            }
            Entry::Occupied(entry) => {
                return Err(RuntimeError::RunnerBinding(format!(
                    "duplicate side-effect verify runner binding for submit descriptor {}",
                    entry.key()
                )));
            }
        }
        Ok(())
    }

    /// Registers one concrete capability implementation binding.
    pub fn register_capability(&mut self, binding: CapabilityImplementationBinding) -> Result<()> {
        let key = capability_implementation_key(binding.descriptor());
        match self.capability_implementations.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(binding);
                Ok(())
            }
            Entry::Occupied(entry) if entry.get() == &binding => Ok(()),
            Entry::Occupied(entry) => Err(RuntimeError::RunnerBinding(format!(
                "duplicate capability implementation for {}:{} conflicts with registered implementation {}",
                binding.descriptor.kind,
                binding.descriptor.version,
                entry.get().implementation_id
            ))),
        }
    }

    /// Registers one concrete implementation for a typed capability contract.
    pub fn register_capability_spec<C>(
        &mut self,
        implementation_id: CapabilityImplementationId,
    ) -> Result<()>
    where
        C: CapabilitySpec,
    {
        let descriptor =
            C::descriptor().map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?;
        self.register_capability(CapabilityImplementationBinding::new(
            descriptor,
            implementation_id,
        ))
    }

    /// Registers executable evidence for one certified adapter binding.
    pub fn register_adapter_executable(&mut self, binding: AdapterExecutableBinding) -> Result<()> {
        self.validate_executable(&binding.executable.factory_id, &binding.executable)?;
        let key = adapter_executable_key(binding.adapter_kind(), binding.adapter_version());
        match self.adapter_executables.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(binding);
                Ok(())
            }
            Entry::Occupied(entry) if entry.get() == &binding => Ok(()),
            Entry::Occupied(entry) => Err(RuntimeError::RunnerBinding(format!(
                "duplicate adapter executable for {}:{} conflicts with registered executable {}",
                binding.adapter_kind(),
                binding.adapter_version(),
                entry.get().executable().factory_id
            ))),
        }
    }

    /// Registers one implementation id for every descriptor in a capability set.
    pub fn register_capability_set(
        &mut self,
        capabilities: &CapabilitySetDescriptor,
        implementation_id: CapabilityImplementationId,
    ) -> Result<()> {
        for descriptor in &capabilities.capabilities {
            self.register_capability(CapabilityImplementationBinding::new(
                descriptor.clone(),
                implementation_id.clone(),
            ))?;
        }
        Ok(())
    }

    pub(crate) fn resolve<S>(
        &self,
        runtime_spec: &S,
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
    ) -> Result<ErasedRunnerBinding>
    where
        S: CurrentSpecRead + ?Sized,
    {
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ) {
            return framework_public_output_binding(
                &self.executable_identity_template,
                node,
                descriptor,
            );
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) {
            return framework_retention_manifest_binding(
                &self.executable_identity_template,
                node,
                descriptor,
            );
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        ) {
            return framework_complete_run_binding(
                &self.executable_identity_template,
                node,
                descriptor,
            );
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) {
            return framework_resolve_saga_terminal_binding(
                &self.executable_identity_template,
                node,
                descriptor,
            );
        }
        if matches!(&node.framework, Some(spec::FrameworkNodeSpec::Bridge(_))) {
            return framework_bridge_binding(&self.executable_identity_template, node, descriptor);
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
        ) {
            let submit_descriptor_id = side_effect_verify_submit_descriptor_id(runtime_spec, node)?;
            let binding = self
                .side_effect_verify_bindings
                .get(submit_descriptor_id)
                .ok_or_else(|| {
                    RuntimeError::RunnerBinding(format!(
                        "missing side-effect verify runner binding for node {} submit descriptor {}",
                        node.node_id, submit_descriptor_id
                    ))
                })?;
            if binding.factory_id.as_str() != descriptor.runner {
                return Err(RuntimeError::RunnerBinding(format!(
                    "side-effect verify runner factory {} does not match descriptor runner {} for node {}",
                    binding.factory_id, descriptor.runner, node.node_id
                )));
            }
            return ErasedRunnerBinding::new(
                node.descriptor_id.clone(),
                binding.factory_id.clone(),
                binding.executable.clone(),
                binding.runner.clone(),
            );
        }
        let binding = self.bindings.get(&node.descriptor_id).ok_or_else(|| {
            RuntimeError::RunnerBinding(format!(
                "missing runner binding for node {} descriptor {}",
                node.node_id, node.descriptor_id
            ))
        })?;
        if binding.descriptor_id != node.descriptor_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "runner binding descriptor mismatch for node {}",
                node.node_id
            )));
        }
        if binding.factory_id.as_str() != descriptor.runner {
            return Err(RuntimeError::RunnerBinding(format!(
                "runner binding factory {} does not match descriptor runner {} for node {}",
                binding.factory_id, descriptor.runner, node.node_id
            )));
        }
        Ok(binding.clone())
    }

    pub(crate) fn resolve_capability_implementations(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<Vec<CapabilityImplementationBinding>> {
        let mut bindings = Vec::with_capacity(node.capability_bindings.capabilities.len());
        for descriptor in &node.capability_bindings.capabilities {
            let key = capability_implementation_key(descriptor);
            let binding = self.capability_implementations.get(&key).ok_or_else(|| {
                RuntimeError::RunnerBinding(format!(
                    "missing capability implementation for node {} capability {}:{}",
                    node.node_id, descriptor.kind, descriptor.version
                ))
            })?;
            if binding.descriptor() != descriptor {
                return Err(RuntimeError::RunnerBinding(format!(
                    "capability implementation {} for node {} differs from certified descriptor {}:{}",
                    binding.implementation_id(),
                    node.node_id,
                    descriptor.kind,
                    descriptor.version
                )));
            }
            bindings.push(binding.clone());
        }
        Ok(bindings)
    }

    pub(crate) fn resolve_adapter_executables(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<Vec<AdapterExecutableBinding>> {
        let mut bindings = Vec::with_capacity(node.adapter_bindings.len());
        for adapter in &node.adapter_bindings {
            let key = adapter_executable_key(&adapter.adapter_kind, &adapter.adapter_version);
            let binding = self.adapter_executables.get(&key).ok_or_else(|| {
                RuntimeError::RunnerBinding(format!(
                    "missing adapter executable for node {} adapter {}:{}",
                    node.node_id, adapter.adapter_kind, adapter.adapter_version
                ))
            })?;
            if binding.adapter_kind() != &adapter.adapter_kind
                || binding.adapter_version() != &adapter.adapter_version
            {
                return Err(RuntimeError::RunnerBinding(format!(
                    "adapter executable for node {} differs from certified adapter {}:{}",
                    node.node_id, adapter.adapter_kind, adapter.adapter_version
                )));
            }
            bindings.push(binding.clone());
        }
        Ok(bindings)
    }

    fn validate_executable(
        &self,
        factory_id: &events::RunnerFactoryId,
        executable: &events::ExecutableIdentity,
    ) -> Result<()> {
        let expected = self
            .executable_identity_template
            .executable(factory_id.clone());
        if executable != &expected {
            return Err(RuntimeError::RunnerBinding(format!(
                "factory {factory_id} executable identity does not match the registry template"
            )));
        }
        Ok(())
    }
}

fn side_effect_verify_submit_descriptor_id<'a, S>(
    runtime_spec: &'a S,
    node: &spec::NodeSpec,
) -> Result<&'a DescriptorId>
where
    S: CurrentSpecRead + ?Sized,
{
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a side-effect verify node",
            node.node_id
        )));
    };
    let submit_node = runtime_spec.node(&verify.submit_node_id).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "side-effect verify node {} references missing submit node {}",
            node.node_id, verify.submit_node_id
        ))
    })?;
    if submit_node.side_effect.is_none() {
        return Err(RuntimeError::InvalidSpec(format!(
            "side-effect verify node {} references non-side-effect submit node {}",
            node.node_id, submit_node.node_id
        )));
    }
    Ok(&submit_node.descriptor_id)
}

fn capability_implementation_key(descriptor: &CapabilityDescriptor) -> (String, String) {
    (
        descriptor.kind.as_str().to_owned(),
        descriptor.version.as_str().to_owned(),
    )
}

fn adapter_executable_key(kind: &AdapterKind, version: &AdapterVersion) -> (String, String) {
    (kind.as_str().to_owned(), version.as_str().to_owned())
}

fn runtime_catalog_mismatch(surface: &'static str, expected: usize, actual: usize) -> RuntimeError {
    RuntimeError::RunnerBinding(format!(
        "runtime {surface} do not equal the authoring catalog: expected {expected}, found {actual}"
    ))
}

fn is_closed_framework_catalog_state(name: &str) -> bool {
    matches!(
        name,
        "mfm.framework.bridge_same_value"
            | "mfm.framework.side_effect_verify"
            | "mfm.framework.render_public_outputs"
            | "mfm.framework.project_retention_manifest"
            | "mfm.framework.complete_run"
            | "mfm.framework.resolve_saga_terminal"
    )
}

#[cfg(test)]
mod authoring_catalog_tests {
    use super::*;
    use mfm_capabilities::{CapabilitySpec, ReadExternal, ReadExternalRole};
    use mfm_ids::{
        AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, ContentDigest,
        DigestAlgorithm, DigestBytes, StateKind, StateVersion,
    };
    use mfm_program::{
        AdapterBindingSpec, CertifiedContext, ExternalReadEvidenceSet, NoContext, ReadState,
        StateResult, StateSpec,
    };
    use mfm_program_derive::{MfmConfig, MfmValue};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct CatalogConfig {
        multiplier: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
    #[mfm(
        namespace = "mfm.runtime.catalog-test",
        name = "value",
        version = "1",
        schema = "mfm.runtime.catalog_test.value"
    )]
    struct CatalogValue {
        amount: u64,
    }

    struct CatalogReadCapability;

    impl CapabilitySpec for CatalogReadCapability {
        type Role = ReadExternalRole;

        fn kind() -> mfm_capabilities::Result<CapabilityKind> {
            CapabilityKind::new(
                "mfm.runtime.catalog-test",
                "read",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x31; 32]),
            )
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn version() -> mfm_capabilities::Result<CapabilityVersion> {
            CapabilityVersion::new("mfm.runtime.catalog_test.read.v1")
                .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.runtime.catalog-test.read"
        }
    }

    struct ExtraCapability;

    impl CapabilitySpec for ExtraCapability {
        type Role = ReadExternalRole;

        fn kind() -> mfm_capabilities::Result<CapabilityKind> {
            CapabilityKind::new(
                "mfm.runtime.catalog-test",
                "extra",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x32; 32]),
            )
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn version() -> mfm_capabilities::Result<CapabilityVersion> {
            CapabilityVersion::new("mfm.runtime.catalog_test.extra.v1")
                .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.runtime.catalog-test.extra"
        }
    }

    struct CatalogReadState {
        _config: CatalogConfig,
    }

    impl StateSpec for CatalogReadState {
        type Config = CatalogConfig;
        type Context = NoContext;
        type Input = CatalogValue;
        type Output = CatalogValue;
        type Effect = ReadExternal;
        type Caps = (CatalogReadCapability,);

        fn kind() -> mfm_program::Result<StateKind> {
            StateKind::new(
                "mfm.runtime.catalog-test",
                "read",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x33; 32]),
            )
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
        }

        fn version() -> mfm_program::Result<StateVersion> {
            StateVersion::new("mfm.runtime.catalog_test.read_state.v1")
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.runtime.catalog-test.read-state"
        }

        fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
            Ok(vec![catalog_adapter()])
        }

        fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
            Ok(Self {
                _config: config.into_inner(),
            })
        }
    }

    impl ReadState for CatalogReadState {
        type Plan = CatalogValue;
        type Evidence = CatalogValue;
        type Facts = ();

        fn plan(
            &self,
            input: &Self::Input,
            _context: &CertifiedContext<Self::Context>,
        ) -> StateResult<Self::Plan> {
            Ok(input.clone())
        }

        fn reduce(
            &self,
            _input: &Self::Input,
            evidence: &ExternalReadEvidenceSet<Self::Evidence>,
            _context: &CertifiedContext<Self::Context>,
        ) -> StateResult<(Self::Output, Self::Facts)> {
            Ok((evidence.primary_evidence().clone(), ()))
        }
    }

    struct EmptyRunner;

    impl ErasedNodeRunner for EmptyRunner {
        fn run_erased<'a>(&'a self, _ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async { Ok(ErasedRunnerOutput::new(Vec::new())) })
        }
    }

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn catalog_adapter() -> AdapterBindingSpec {
        AdapterBindingSpec {
            adapter_kind: AdapterKind::new(
                "mfm.runtime.catalog-test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x34; 32]),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.runtime.catalog_test.adapter.v1")
                .expect("adapter version"),
        }
    }

    fn extra_adapter() -> AdapterBindingSpec {
        AdapterBindingSpec {
            adapter_kind: AdapterKind::new(
                "mfm.runtime.catalog-test",
                "extra-adapter",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x35; 32]),
            )
            .expect("extra adapter kind"),
            adapter_version: AdapterVersion::new("mfm.runtime.catalog_test.extra_adapter.v1")
                .expect("extra adapter version"),
        }
    }

    fn fixture() -> (mfm_certify::ProgramAuthoringCatalog, ErasedRunnerRegistry) {
        let mut catalog = mfm_certify::ProgramAuthoringCatalog::__new();
        catalog
            .__register_state::<CatalogReadState>()
            .expect("catalog state");

        let mut registry = ErasedRunnerRegistry::new(ExecutableIdentityTemplate::new(digest(0x41)));
        let read_factory = registry
            .factory_binding(events::RunnerFactoryId::new("read_external").expect("read factory"));
        let adapter_factory = registry.factory_binding(
            events::RunnerFactoryId::new("catalog_adapter").expect("adapter factory"),
        );
        registry
            .register_capability_spec::<CatalogReadCapability>(
                CapabilityImplementationId::new("mfm.runtime.catalog-test.read.impl")
                    .expect("implementation id"),
            )
            .expect("capability registration");
        let mut registrations = crate::RunnerRegistrationBuilder::new(&mut registry);
        registrations
            .register_state_runner_with_factory::<CatalogReadState>(
                &read_factory,
                Arc::new(EmptyRunner),
            )
            .expect("runner registration");
        let adapter = catalog_adapter();
        registrations
            .register_adapter_executable_with_factory(
                adapter.adapter_kind,
                adapter.adapter_version,
                &adapter_factory,
            )
            .expect("adapter registration");
        (catalog, registry)
    }

    #[test]
    fn catalog_validation_rejects_missing_and_extra_runtime_surfaces() {
        let (catalog, registry) = fixture();
        registry
            .validate_authoring_catalog(&catalog)
            .expect("exact runtime catalog");

        let mut missing_runner = registry.clone();
        missing_runner.bindings.clear();
        assert!(missing_runner.validate_authoring_catalog(&catalog).is_err());

        let mut extra_runner = registry.clone();
        let existing = extra_runner
            .bindings
            .values()
            .next()
            .expect("runner binding")
            .clone();
        let extra_id = DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x42; 32]),
        );
        let mut extra_binding = existing;
        extra_binding.descriptor_id = extra_id.clone();
        extra_runner.bindings.insert(extra_id, extra_binding);
        assert!(extra_runner.validate_authoring_catalog(&catalog).is_err());

        let mut missing_capability = registry.clone();
        missing_capability.capability_implementations.clear();
        assert!(missing_capability
            .validate_authoring_catalog(&catalog)
            .is_err());

        let mut extra_capability = registry.clone();
        let descriptor = ExtraCapability::descriptor().expect("extra capability");
        extra_capability.capability_implementations.insert(
            capability_implementation_key(&descriptor),
            CapabilityImplementationBinding::new(
                descriptor,
                CapabilityImplementationId::new("mfm.runtime.catalog-test.extra.impl")
                    .expect("extra implementation"),
            ),
        );
        assert!(extra_capability
            .validate_authoring_catalog(&catalog)
            .is_err());

        let mut missing_adapter = registry.clone();
        missing_adapter.adapter_executables.clear();
        assert!(missing_adapter
            .validate_authoring_catalog(&catalog)
            .is_err());

        let mut extra_adapter_registry = registry.clone();
        let adapter = extra_adapter();
        let factory = extra_adapter_registry.factory_binding(
            events::RunnerFactoryId::new("extra_adapter").expect("extra adapter factory"),
        );
        extra_adapter_registry.adapter_executables.insert(
            adapter_executable_key(&adapter.adapter_kind, &adapter.adapter_version),
            AdapterExecutableBinding::new(
                adapter.adapter_kind,
                adapter.adapter_version,
                factory.executable(),
            ),
        );
        assert!(extra_adapter_registry
            .validate_authoring_catalog(&catalog)
            .is_err());

        let mut extra_side_effect_verifier = registry;
        let factory = extra_side_effect_verifier.factory_binding(
            events::RunnerFactoryId::new("read_external").expect("verify factory"),
        );
        extra_side_effect_verifier
            .side_effect_verify_bindings
            .insert(
                DescriptorId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0x43; 32]),
                ),
                ErasedFrameworkRunnerBinding {
                    factory_id: factory.factory_id(),
                    executable: factory.executable(),
                    runner: Arc::new(EmptyRunner),
                },
            );
        assert!(extra_side_effect_verifier
            .validate_authoring_catalog(&catalog)
            .is_err());
    }

    #[test]
    fn catalog_validation_rejects_wrong_executables_and_implementation_ids() {
        let (catalog, registry) = fixture();

        let mut wrong_runner_executable = registry.clone();
        wrong_runner_executable
            .bindings
            .values_mut()
            .next()
            .expect("runner binding")
            .executable
            .binary_digest = digest(0x51);
        assert!(wrong_runner_executable
            .validate_authoring_catalog(&catalog)
            .is_err());

        let mut wrong_adapter_executable = registry.clone();
        wrong_adapter_executable
            .adapter_executables
            .values_mut()
            .next()
            .expect("adapter binding")
            .executable
            .binary_digest = digest(0x52);
        assert!(wrong_adapter_executable
            .validate_authoring_catalog(&catalog)
            .is_err());

        registry
            .validate_capability_implementation::<CatalogReadCapability>(
                "mfm.runtime.catalog-test.read.impl",
            )
            .expect("selected implementation");
        assert!(registry
            .validate_capability_implementation::<CatalogReadCapability>(
                "mfm.runtime.catalog-test.wrong.impl",
            )
            .is_err());
    }

    #[test]
    fn duplicate_runtime_bindings_are_rejected() {
        let (_catalog, mut registry) = fixture();
        let runner = registry
            .bindings
            .values()
            .next()
            .expect("runner binding")
            .clone();
        assert!(registry.register(runner).is_err());

        let descriptor = CatalogReadCapability::descriptor().expect("capability descriptor");
        assert!(registry
            .register_capability(CapabilityImplementationBinding::new(
                descriptor,
                CapabilityImplementationId::new("mfm.runtime.catalog-test.conflict.impl")
                    .expect("conflicting implementation"),
            ))
            .is_err());

        let adapter = catalog_adapter();
        let conflicting_factory = registry.factory_binding(
            events::RunnerFactoryId::new("conflicting_adapter").expect("conflicting factory"),
        );
        assert!(registry
            .register_adapter_executable(AdapterExecutableBinding::new(
                adapter.adapter_kind,
                adapter.adapter_version,
                conflicting_factory.executable(),
            ))
            .is_err());
    }
}
