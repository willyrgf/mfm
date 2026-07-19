use std::collections::{BTreeMap, BTreeSet};

use mfm_events::v1 as events;
use mfm_ids::{ContentDigest, DigestAlgorithm, NodeId};
use mfm_spec::v1 as spec;

use crate::runners::{
    CapabilityImplementationBinding, ErasedRunnerBinding, ErasedRunnerRegistry,
    RunnerIngressContext,
};
use crate::{
    canonical_json, capability_implementation_identity_json, executable_identity_json,
    CertifiedRuntimeSpec, Result, RunLaunchEvidence, RuntimeError,
};

/// Runtime binding authority for one certified execution spec.
///
/// The context proves that every executable certified node has an available runner binding whose
/// executable identity matches the certified descriptor. It does not execute runners or create live
/// capability providers; those remain attempt-time behavior.
#[derive(Clone)]
pub struct BoundRuntimeContext {
    bindings: BTreeMap<NodeId, ErasedRunnerBinding>,
    capability_authorities: BTreeMap<NodeId, BoundCapabilityAuthority>,
    framework_handlers: BTreeMap<NodeId, BoundFrameworkHandlerAuthority>,
    runner_executables: Vec<events::ExecutableIdentity>,
    adapter_executables: Vec<events::ExecutableIdentity>,
    capability_implementations: Vec<events::CapabilityImplementationIdentity>,
}

/// Bound capability authority for a certified node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundCapabilityAuthority {
    node_id: NodeId,
    capabilities: mfm_capabilities::CapabilitySetDescriptor,
    implementations: Vec<CapabilityImplementationBinding>,
}

impl BoundCapabilityAuthority {
    /// Returns the node covered by this capability authority.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the certified capability descriptors available to the node.
    pub fn capabilities(&self) -> &mfm_capabilities::CapabilitySetDescriptor {
        &self.capabilities
    }

    /// Returns the runtime implementation binding for each certified capability.
    pub fn implementations(&self) -> &[CapabilityImplementationBinding] {
        &self.implementations
    }
}

/// Bound framework handler kind for a certified lifecycle node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BoundFrameworkHandlerKind {
    /// Same-value child-scope bridge handler.
    Bridge,
    /// Public output render handler.
    PublicOutputRender,
    /// Retention manifest projection handler.
    ProjectRetentionManifest,
    /// Forward-success completion handler.
    CompleteRun,
    /// Saga terminal resolution handler.
    ResolveSagaTerminal,
    /// Side-effect verification handler.
    SideEffectVerify,
}

/// Bound framework handler authority for a certified lifecycle node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundFrameworkHandlerAuthority {
    node_id: NodeId,
    kind: BoundFrameworkHandlerKind,
}

impl BoundFrameworkHandlerAuthority {
    /// Returns the framework node covered by this handler authority.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the framework handler kind bound for the node.
    pub fn kind(&self) -> BoundFrameworkHandlerKind {
        self.kind
    }
}

impl BoundRuntimeContext {
    /// Builds a bound context by checking every certified executable node.
    pub fn bind(
        runtime_spec: &CertifiedRuntimeSpec,
        runners: &ErasedRunnerRegistry,
    ) -> Result<Self> {
        let mut accumulator = BindingAccumulator::default();

        for node in runtime_spec.executable_nodes() {
            bind_node(runtime_spec, runners, node, &mut accumulator)?;
        }

        Ok(Self {
            bindings: accumulator.bindings,
            capability_authorities: accumulator.capability_authorities,
            framework_handlers: accumulator.framework_handlers,
            runner_executables: accumulator.runner_executables,
            adapter_executables: accumulator.adapter_executables,
            capability_implementations: accumulator
                .capability_implementations
                .into_iter()
                .collect(),
        })
    }

    /// Returns the runner executable identities admitted into `RunAdmitted` evidence.
    pub fn runner_executables(&self) -> &[events::ExecutableIdentity] {
        &self.runner_executables
    }

    /// Returns the adapter executable identities admitted into `RunAdmitted` evidence.
    pub fn adapter_executables(&self) -> &[events::ExecutableIdentity] {
        &self.adapter_executables
    }

    /// Returns the concrete capability implementation identities admitted into `RunAdmitted`.
    pub fn capability_implementations(&self) -> &[events::CapabilityImplementationIdentity] {
        &self.capability_implementations
    }

    pub(crate) fn admitted_binding_digest(&self) -> Result<ContentDigest> {
        let canonical = canonical_json(serde_json::json!({
            "adapter_executables": self.adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
            "capability_implementations": self.capability_implementations.iter().map(capability_implementation_identity_json).collect::<Vec<_>>(),
            "runner_executables": self.runner_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
        }))?;
        Ok(ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            canonical.digest_bytes(),
        ))
    }

    pub(crate) fn validate_run_admitted_binding(
        &self,
        run_admitted: &events::RunAdmitted,
    ) -> Result<()> {
        if run_admitted.runner_executables != self.runner_executables {
            return Err(RuntimeError::RunnerBinding(
                "RunAdmitted runner executable identities do not match bound runtime context"
                    .to_owned(),
            ));
        }
        if run_admitted.adapter_executables != self.adapter_executables {
            return Err(RuntimeError::RunnerBinding(
                "RunAdmitted adapter executable identities do not match bound runtime context"
                    .to_owned(),
            ));
        }
        if run_admitted.capability_implementations != self.capability_implementations {
            return Err(RuntimeError::RunnerBinding(
                "RunAdmitted capability implementation identities do not match bound runtime context"
                    .to_owned(),
            ));
        }
        let digest = self.admitted_binding_digest()?;
        if digest != run_admitted.admitted_binding_digest {
            return Err(RuntimeError::RunnerBinding(
                "RunAdmitted binding digest does not match bound runtime context".to_owned(),
            ));
        }
        Ok(())
    }

    /// Returns the bound capability authority for a certified node.
    pub fn capability_authority_for(&self, node_id: &NodeId) -> Option<&BoundCapabilityAuthority> {
        self.capability_authorities.get(node_id)
    }

    /// Returns the bound framework handler authority for a certified lifecycle node.
    pub fn framework_handler_for(
        &self,
        node_id: &NodeId,
    ) -> Option<&BoundFrameworkHandlerAuthority> {
        self.framework_handlers.get(node_id)
    }

    pub(crate) fn runner_binding_for(&self, node: &spec::NodeSpec) -> Result<ErasedRunnerBinding> {
        let binding = self.bindings.get(&node.node_id).ok_or_else(|| {
            RuntimeError::RunnerBinding(format!(
                "missing bound runner context for node {}",
                node.node_id
            ))
        })?;
        if binding.descriptor_id() != &node.descriptor_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "bound runner descriptor {} does not match certified descriptor {} for node {}",
                binding.descriptor_id(),
                node.descriptor_id,
                node.node_id
            )));
        }
        Ok(binding.clone())
    }

    pub(crate) fn validate_admission_authority(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
    ) -> Result<()> {
        for node in runtime_spec.executable_nodes() {
            self.require_node_authority(node)?;
        }
        Ok(())
    }

    pub(crate) async fn validate_launch_ingress(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        launch: &RunLaunchEvidence,
    ) -> Result<()> {
        let mut required = Vec::new();
        for node in runtime_spec.executable_nodes() {
            if node.framework.is_none() {
                collect_runtime_config_requirement(
                    self.validate_node_ingress(runtime_spec, node, launch).await,
                    &mut required,
                )?;
            }
        }
        runtime_config_requirement_result(required)
    }

    /// Validates process-local ingress only for domain nodes that may still execute.
    ///
    /// Admission validates every domain node before persisting `RunAdmitted`. On recovery, a
    /// completed node cannot regain work, so revalidating its process-local provider would make
    /// later deterministic work depend on configuration it no longer needs. Pending domain nodes
    /// retain the same ingress check before the scheduler acquires a claim.
    pub(crate) async fn validate_pending_launch_ingress(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &mfm_ids::RunId,
        projection: &mfm_store::v1::ProjectionSnapshot,
        launch: &RunLaunchEvidence,
    ) -> Result<()> {
        let mut required = Vec::new();
        for node in runtime_spec.executable_nodes() {
            if node.framework.is_none()
                && projection
                    .cell_terminal_for_run(run_id, &node.output_cell)
                    .is_none()
            {
                collect_runtime_config_requirement(
                    self.validate_node_ingress(runtime_spec, node, launch).await,
                    &mut required,
                )?;
            }
        }
        runtime_config_requirement_result(required)
    }

    fn require_node_authority(&self, node: &spec::NodeSpec) -> Result<()> {
        self.runner_binding_for(node)?;
        let Some(capabilities) = self.capability_authority_for(&node.node_id) else {
            return Err(RuntimeError::RunnerBinding(format!(
                "missing bound capability authority for node {}",
                node.node_id
            )));
        };
        if capabilities.capabilities != node.capability_bindings {
            return Err(RuntimeError::RunnerBinding(format!(
                "bound capability authority for node {} differs from certified descriptor",
                node.node_id
            )));
        }
        if capabilities.implementations.len() != node.capability_bindings.capabilities.len() {
            return Err(RuntimeError::RunnerBinding(format!(
                "bound capability authority for node {} has incomplete implementation evidence",
                node.node_id
            )));
        }
        for (expected, implementation) in node
            .capability_bindings
            .capabilities
            .iter()
            .zip(capabilities.implementations.iter())
        {
            if implementation.descriptor() != expected {
                return Err(RuntimeError::RunnerBinding(format!(
                    "bound capability implementation for node {} differs from certified descriptor",
                    node.node_id
                )));
            }
        }
        if let Some(expected) = framework_handler_kind(node) {
            let Some(handler) = self.framework_handler_for(&node.node_id) else {
                return Err(RuntimeError::RunnerBinding(format!(
                    "missing bound framework handler for node {}",
                    node.node_id
                )));
            };
            if handler.kind != expected {
                return Err(RuntimeError::RunnerBinding(format!(
                    "bound framework handler for node {} does not match certified framework role",
                    node.node_id
                )));
            }
        }
        Ok(())
    }

    async fn validate_node_ingress(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        node: &spec::NodeSpec,
        launch: &RunLaunchEvidence,
    ) -> Result<()> {
        let binding = self.runner_binding_for(node)?;
        binding
            .runner
            .validate_ingress(RunnerIngressContext::new(runtime_spec, node, launch))
            .await
    }
}

fn collect_runtime_config_requirement(
    result: Result<()>,
    required: &mut Vec<mfm_capabilities::RedactedProviderDiagnostic>,
) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(RuntimeError::Failure(failure))
            if failure.code().as_str() == "RuntimeConfigRequired" =>
        {
            required.extend_from_slice(failure.diagnostics());
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn runtime_config_requirement_result(
    required: Vec<mfm_capabilities::RedactedProviderDiagnostic>,
) -> Result<()> {
    if required.is_empty() {
        return Ok(());
    }
    let failure = crate::RuntimeFailure::new(
        events::ErrorCode::new("RuntimeConfigRequired")?,
        events::ErrorCategory::Capability,
        "runtime configuration is required",
        required,
    )?;
    Err(RuntimeError::Failure(failure))
}

#[cfg(test)]
mod runtime_config_requirement_tests {
    use super::*;
    use mfm_capabilities::{ProviderDiagnosticCode, ProviderDiagnosticValue};
    use mfm_ids::LocalPublicId;

    fn id(value: &str) -> LocalPublicId {
        LocalPublicId::new(value).expect("checked test id")
    }

    fn diagnostic(family: &str, network: &str) -> mfm_capabilities::RedactedProviderDiagnostic {
        mfm_capabilities::RedactedProviderDiagnostic::new(
            id(family),
            ProviderDiagnosticCode::ProviderConfigurationMissing,
        )
        .with_field(id("network_id"), ProviderDiagnosticValue::Id(id(network)))
    }

    fn failure(
        code: &str,
        diagnostics: Vec<mfm_capabilities::RedactedProviderDiagnostic>,
    ) -> RuntimeError {
        RuntimeError::Failure(
            crate::RuntimeFailure::new(
                events::ErrorCode::new(code).expect("error code"),
                events::ErrorCategory::Capability,
                "reviewed failure",
                diagnostics,
            )
            .expect("runtime failure"),
        )
    }

    #[test]
    fn mixed_requirements_are_sorted_and_deduplicated() {
        let evm = diagnostic("evm", "test-evm");
        let bitcoin = diagnostic("bitcoin", "test-btc");
        let mut required = Vec::new();

        collect_runtime_config_requirement(
            Err(failure("RuntimeConfigRequired", vec![evm.clone(), evm])),
            &mut required,
        )
        .expect("EVM requirement");
        collect_runtime_config_requirement(
            Err(failure("RuntimeConfigRequired", vec![bitcoin])),
            &mut required,
        )
        .expect("Bitcoin requirement");

        let RuntimeError::Failure(failure) =
            runtime_config_requirement_result(required).expect_err("requirements reject admission")
        else {
            panic!("expected structured failure");
        };
        assert_eq!(failure.code().as_str(), "RuntimeConfigRequired");
        assert_eq!(failure.diagnostics().len(), 2);
        assert_eq!(
            failure.diagnostics()[0].provider_family().as_str(),
            "bitcoin"
        );
        assert_eq!(failure.diagnostics()[1].provider_family().as_str(), "evm");
    }

    #[test]
    fn invalid_configuration_remains_fail_fast() {
        let invalid = failure("RuntimeConfigInvalid", vec![diagnostic("evm", "test-evm")]);
        let error = collect_runtime_config_requirement(Err(invalid), &mut Vec::new())
            .expect_err("invalid configuration is not a missing requirement");
        let RuntimeError::Failure(failure) = error else {
            panic!("expected structured failure");
        };
        assert_eq!(failure.code().as_str(), "RuntimeConfigInvalid");
    }

    #[test]
    fn admitted_binding_digest_binds_capability_implementation_identity() {
        let context = |capability_implementations| BoundRuntimeContext {
            bindings: BTreeMap::new(),
            capability_authorities: BTreeMap::new(),
            framework_handlers: BTreeMap::new(),
            runner_executables: Vec::new(),
            adapter_executables: Vec::new(),
            capability_implementations,
        };
        let without_capability = context(Vec::new())
            .admitted_binding_digest()
            .expect("empty binding digest");
        let with_capability = context(vec![events::CapabilityImplementationIdentity {
            capability_kind: mfm_ids::CapabilityKind::new(
                "mfm.test",
                "capability",
                DigestAlgorithm::Sha256JcsV1,
                mfm_ids::DigestBytes::from_array([7; 32]),
            )
            .expect("capability kind"),
            capability_version: mfm_ids::CapabilityVersion::new("mfm.test.capability.v1")
                .expect("capability version"),
            implementation_id: mfm_ids::RuntimeBindingId::new("mfm.test.capability.runtime.v1")
                .expect("implementation id"),
        }])
        .admitted_binding_digest()
        .expect("capability binding digest");

        assert_ne!(without_capability, with_capability);
    }
}

#[derive(Default)]
struct BindingAccumulator {
    bindings: BTreeMap<NodeId, ErasedRunnerBinding>,
    capability_authorities: BTreeMap<NodeId, BoundCapabilityAuthority>,
    framework_handlers: BTreeMap<NodeId, BoundFrameworkHandlerAuthority>,
    seen_runner_executables: BTreeSet<String>,
    seen_adapter_executables: BTreeSet<(String, String)>,
    runner_executables: Vec<events::ExecutableIdentity>,
    adapter_executables: Vec<events::ExecutableIdentity>,
    capability_implementations: BTreeSet<events::CapabilityImplementationIdentity>,
}

/// Loader for runtime binding authority.
#[derive(Clone)]
pub struct BoundRuntimeContextLoader {
    runners: ErasedRunnerRegistry,
}

impl BoundRuntimeContextLoader {
    /// Creates a loader backed by an erased runner registry.
    pub fn new(runners: ErasedRunnerRegistry) -> Self {
        Self { runners }
    }

    /// Validates runner binding authority for a certified runtime spec.
    pub fn load(&self, runtime_spec: &CertifiedRuntimeSpec) -> Result<BoundRuntimeContext> {
        BoundRuntimeContext::bind(runtime_spec, &self.runners)
    }
}

fn bind_node(
    runtime_spec: &CertifiedRuntimeSpec,
    runners: &ErasedRunnerRegistry,
    node: &spec::NodeSpec,
    accumulator: &mut BindingAccumulator,
) -> Result<()> {
    let descriptor = runtime_spec.state_descriptor_for_node(node)?;
    let binding = runners.resolve(runtime_spec, node, descriptor)?;
    let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} references missing output cell {}",
            node.node_id, node.output_cell
        ))
    })?;
    if matches!(output_cell.context, spec::CellContextSpec::Bound { .. })
        && binding.runner.context_output_extractor().is_none()
    {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} produces context-bound output cell {} without a registered context output extractor",
            node.node_id, output_cell.cell_id
        )));
    }
    let capability_implementations = runners.resolve_capability_implementations(node)?;
    for implementation in &capability_implementations {
        accumulator
            .capability_implementations
            .insert(events::CapabilityImplementationIdentity {
                capability_kind: implementation.descriptor().kind.clone(),
                capability_version: implementation.descriptor().version.clone(),
                implementation_id: implementation
                    .implementation_id()
                    .runtime_binding_id()
                    .clone(),
            });
    }
    let adapter_executables = runners.resolve_adapter_executables(node)?;
    if accumulator
        .bindings
        .insert(node.node_id.clone(), binding.clone())
        .is_some()
    {
        return Err(RuntimeError::InvalidSpec(format!(
            "node {} appears more than once in runtime binding context",
            node.node_id
        )));
    }
    if accumulator
        .capability_authorities
        .insert(
            node.node_id.clone(),
            BoundCapabilityAuthority {
                node_id: node.node_id.clone(),
                capabilities: node.capability_bindings.clone(),
                implementations: capability_implementations,
            },
        )
        .is_some()
    {
        return Err(RuntimeError::InvalidSpec(format!(
            "node {} appears more than once in runtime capability authority",
            node.node_id
        )));
    }
    if let Some(kind) = framework_handler_kind(node) {
        if accumulator
            .framework_handlers
            .insert(
                node.node_id.clone(),
                BoundFrameworkHandlerAuthority {
                    node_id: node.node_id.clone(),
                    kind,
                },
            )
            .is_some()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "node {} appears more than once in runtime framework handler authority",
                node.node_id
            )));
        }
    }

    let executable_key = binding.executable().factory_id.as_str().to_owned();
    if accumulator.seen_runner_executables.insert(executable_key) {
        accumulator
            .runner_executables
            .push(binding.executable().clone());
    }
    for adapter in adapter_executables {
        let key = (
            adapter.adapter_kind().as_str().to_owned(),
            adapter.adapter_version().as_str().to_owned(),
        );
        if accumulator.seen_adapter_executables.insert(key) {
            accumulator
                .adapter_executables
                .push(adapter.executable().clone());
        }
    }
    Ok(())
}

fn framework_handler_kind(node: &spec::NodeSpec) -> Option<BoundFrameworkHandlerKind> {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::Bridge(_)) => Some(BoundFrameworkHandlerKind::Bridge),
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => {
            Some(BoundFrameworkHandlerKind::PublicOutputRender)
        }
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
            Some(BoundFrameworkHandlerKind::ProjectRetentionManifest)
        }
        Some(spec::FrameworkNodeSpec::CompleteRun(_)) => {
            Some(BoundFrameworkHandlerKind::CompleteRun)
        }
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) => {
            Some(BoundFrameworkHandlerKind::ResolveSagaTerminal)
        }
        Some(spec::FrameworkNodeSpec::SideEffectVerify(_)) => {
            Some(BoundFrameworkHandlerKind::SideEffectVerify)
        }
        None => None,
    }
}
