use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_events::v1 as events;
use mfm_ids::DescriptorId;
use mfm_spec::v1 as spec;

use crate::framework::{
    framework_bootstrap_run_binding, framework_complete_run_binding,
    framework_public_output_binding, framework_resolve_saga_terminal_binding,
    framework_retention_manifest_binding,
};
use crate::{ErasedRunCtx, Result, RuntimeError, StagedArtifact, StagedRetentionRefs};

/// Boxed future returned by an erased typed runner.
pub type ErasedRunnerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ErasedRunnerOutput>> + Send + 'a>>;

/// Object-safe erased runner boundary used after typed spec certification.
///
/// Runner selection is keyed by the certified node descriptor id. The runner receives only
/// store-verified input cell evidence and certified capability descriptors.
pub trait ErasedNodeRunner: Send + Sync {
    /// Executes one certified node attempt.
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a>;
}

/// Runner-owned payloads that may be proposed by domain execution.
///
/// Scheduler, framework lifecycle, artifact reference, retention, and run lifecycle events are
/// intentionally absent. The runtime middleware derives those authoritative payloads after
/// validating this runner-facing payload set against the certified spec and store projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerEventPayload {
    /// Recorded read fact event.
    FactRecorded(events::FactRecorded),
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
            RunnerEventPayload::FactRecorded(payload) => Self::FactRecorded(payload),
            RunnerEventPayload::CellProduced(payload) => Self::CellProduced(payload),
            RunnerEventPayload::CellSkipped(payload) => Self::CellSkipped(payload),
            RunnerEventPayload::SideEffectIntentPersisted(payload) => {
                Self::SideEffectIntentPersisted(payload)
            }
            RunnerEventPayload::SideEffectClaimed(payload) => Self::SideEffectClaimed(payload),
            RunnerEventPayload::SideEffectClaimTakenOver(payload) => {
                Self::SideEffectClaimTakenOver(payload)
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

/// Typed payload batch returned by an erased runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasedRunnerOutput {
    /// Staged artifacts or sealed finalized handles referenced by payloads.
    pub staged_artifacts: Vec<StagedArtifact>,
    /// Retention refs staged by the runner for scheduler-owned event binding.
    pub staged_retention_refs: Vec<StagedRetentionRefs>,
    /// Runner-owned typed payloads to validate before runtime lifecycle derivation.
    pub payloads: Vec<RunnerEventPayload>,
}

impl ErasedRunnerOutput {
    /// Creates an output batch from payloads with no additional artifact evidence.
    pub fn new(payloads: Vec<RunnerEventPayload>) -> Self {
        Self {
            staged_artifacts: Vec::new(),
            staged_retention_refs: Vec::new(),
            payloads,
        }
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

/// Registry of erased runners keyed by certified state descriptor id.
#[derive(Clone, Default)]
pub struct ErasedRunnerRegistry {
    bindings: BTreeMap<DescriptorId, ErasedRunnerBinding>,
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

    pub(crate) fn resolve(
        &self,
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
    ) -> Result<ErasedRunnerBinding> {
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::BootstrapRun(_))
        ) {
            return framework_bootstrap_run_binding(node, descriptor);
        }
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
}
