use std::collections::{btree_map::Entry, BTreeMap};
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
use crate::{
    CertifiedInvocationContext, CertifiedRuntimeSpec, ErasedRunCtx, PreInvocationRunCtx, Result,
    RunLaunchArtifact, RunLaunchEvidence, RuntimeError, StagedArtifact, StagedRetentionRefs,
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
#[derive(Clone, Copy)]
pub struct RunnerIngressContext<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    node: &'a spec::NodeSpec,
    launch: &'a RunLaunchEvidence,
}

impl<'a> RunnerIngressContext<'a> {
    pub(crate) fn new(
        runtime_spec: &'a CertifiedRuntimeSpec,
        node: &'a spec::NodeSpec,
        launch: &'a RunLaunchEvidence,
    ) -> Self {
        Self {
            runtime_spec,
            node,
            launch,
        }
    }

    /// Returns the certified runtime spec being launched.
    pub fn runtime_spec(&self) -> &'a CertifiedRuntimeSpec {
        self.runtime_spec
    }

    /// Returns the certified node bound to the runner.
    pub fn node(&self) -> &'a spec::NodeSpec {
        self.node
    }

    /// Returns certified transition-context authority for this ingress node.
    pub fn context(&self) -> Result<CertifiedInvocationContext> {
        self.runtime_spec.invocation_context_for_node(self.node)
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

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<StagedArtifact>,
        Vec<StagedRetentionRefs>,
        Vec<events::FactRecorded>,
        Vec<RunnerEventPayload>,
        Option<RunnerOutputSettlement>,
    ) {
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
#[derive(Clone, Default)]
pub struct ErasedRunnerRegistry {
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
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one erased runner binding.
    pub fn register(&mut self, binding: ErasedRunnerBinding) -> Result<()> {
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

    pub(crate) fn resolve(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
    ) -> Result<ErasedRunnerBinding> {
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ) {
            return framework_public_output_binding(node, descriptor);
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) {
            return framework_retention_manifest_binding(node, descriptor);
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        ) {
            return framework_complete_run_binding(node, descriptor);
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) {
            return framework_resolve_saga_terminal_binding(node, descriptor);
        }
        if matches!(&node.framework, Some(spec::FrameworkNodeSpec::Bridge(_))) {
            return framework_bridge_binding(node, descriptor);
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
}

fn side_effect_verify_submit_descriptor_id<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
) -> Result<&'a DescriptorId> {
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
