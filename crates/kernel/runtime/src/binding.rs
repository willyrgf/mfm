use std::collections::{BTreeMap, BTreeSet};

use mfm_events::v1 as events;
use mfm_ids::{ContentDigest, DigestAlgorithm, NodeId};
use mfm_spec::v1 as spec;

use crate::runners::{CapabilityImplementationBinding, ErasedRunnerBinding, ErasedRunnerRegistry};
use crate::{canonical_json, CertifiedRuntimeSpec, Result, RuntimeError};

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
    /// Public output render handler.
    PublicOutputRender,
    /// Retention manifest projection handler.
    ProjectRetentionManifest,
    /// Forward-success completion handler.
    CompleteRun,
    /// Saga terminal resolution handler.
    ResolveSagaTerminal,
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

        for node_id in runtime_spec.topological_order() {
            let node = runtime_spec.node(node_id).expect("topological node exists");
            bind_node(runtime_spec, runners, node, &mut accumulator)?;
        }
        for (_, node) in runtime_spec.remediations() {
            bind_node(runtime_spec, runners, node, &mut accumulator)?;
        }

        Ok(Self {
            bindings: accumulator.bindings,
            capability_authorities: accumulator.capability_authorities,
            framework_handlers: accumulator.framework_handlers,
            runner_executables: accumulator.runner_executables,
        })
    }

    /// Returns the runner executable identities admitted into `RunAdmitted` evidence.
    pub fn runner_executables(&self) -> &[events::ExecutableIdentity] {
        &self.runner_executables
    }

    pub(crate) fn validate_run_admitted_executables(
        &self,
        run_admitted: &events::RunAdmitted,
    ) -> Result<()> {
        if run_admitted.runner_executables != self.runner_executables {
            return Err(RuntimeError::RunnerBinding(
                "RunAdmitted runner executable identities do not match bound runtime context"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn admitted_binding_digest(
        &self,
        adapter_executables: &[events::ExecutableIdentity],
    ) -> Result<ContentDigest> {
        let canonical = canonical_json(serde_json::json!({
            "adapter_executables": adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
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
        self.validate_run_admitted_executables(run_admitted)?;
        let digest = self.admitted_binding_digest(&run_admitted.adapter_executables)?;
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
        for node_id in runtime_spec.topological_order() {
            let node = runtime_spec.node(node_id).expect("topological node exists");
            self.require_node_authority(node)?;
        }
        for (_, node) in runtime_spec.remediations() {
            self.require_node_authority(node)?;
        }
        Ok(())
    }

    fn require_node_authority(&self, node: &spec::NodeSpec) -> Result<()> {
        if is_parked_side_effect_verify(node) {
            return Ok(());
        }
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
}

fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "cargo_package_digest": identity.cargo_package_digest.as_str(),
        "factory_id": identity.factory_id.as_str(),
        "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(|value| value.as_str()),
        "nix_output_hash": identity.nix_output_hash.as_ref().map(|value| value.as_str()),
    })
}

#[derive(Default)]
struct BindingAccumulator {
    bindings: BTreeMap<NodeId, ErasedRunnerBinding>,
    capability_authorities: BTreeMap<NodeId, BoundCapabilityAuthority>,
    framework_handlers: BTreeMap<NodeId, BoundFrameworkHandlerAuthority>,
    seen_executables: BTreeSet<String>,
    runner_executables: Vec<events::ExecutableIdentity>,
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
    if is_parked_side_effect_verify(node) {
        return Ok(());
    }
    let descriptor = runtime_spec.state_descriptor_for_node(node)?;
    let binding = runners.resolve(node, descriptor)?;
    let capability_implementations = runners.resolve_capability_implementations(node)?;
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
    if accumulator.seen_executables.insert(executable_key) {
        accumulator
            .runner_executables
            .push(binding.executable().clone());
    }
    Ok(())
}

fn framework_handler_kind(node: &spec::NodeSpec) -> Option<BoundFrameworkHandlerKind> {
    match &node.framework {
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
        Some(spec::FrameworkNodeSpec::Bridge(_))
        | Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
        | None => None,
    }
}

fn is_parked_side_effect_verify(node: &spec::NodeSpec) -> bool {
    matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
    )
}
