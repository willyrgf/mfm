use super::*;

/// Descriptor identities embedded in the hash-defining spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorIdentity {
    /// State descriptor identity.
    State(Box<StateDescriptorIdentity>),
    /// Operation descriptor identity.
    Operation(Box<OperationDescriptorIdentity>),
    /// Public output renderer descriptor identity.
    Renderer(Box<RendererDescriptorIdentity>),
}

impl DescriptorIdentity {
    /// Returns this descriptor's family.
    pub fn family(&self) -> DescriptorFamily {
        match self {
            Self::State(_) => DescriptorFamily::State,
            Self::Operation(_) => DescriptorFamily::Operation,
            Self::Renderer(_) => DescriptorFamily::Renderer,
        }
    }

    /// Returns this descriptor's content-addressed id.
    pub fn descriptor_id(&self) -> &DescriptorId {
        match self {
            Self::State(identity) => &identity.descriptor_id,
            Self::Operation(identity) => &identity.descriptor_id,
            Self::Renderer(identity) => &identity.descriptor_id,
        }
    }

    /// Computes the canonical descriptor payload digest.
    pub fn descriptor_digest(&self) -> Result<ContentDigest> {
        content_digest(self.json())
    }

    /// Computes the descriptor reference used by node and contract records.
    pub fn descriptor_ref(&self) -> Result<DescriptorRef> {
        Ok(DescriptorRef {
            family: self.family(),
            descriptor_id: self.descriptor_id().clone(),
            descriptor_digest: self.descriptor_digest()?,
        })
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        match self {
            Self::State(identity) => {
                let mut json = identity.json();
                json["descriptor_family"] = serde_json::json!("state");
                json
            }
            Self::Operation(identity) => {
                let mut json = identity.json();
                json["descriptor_family"] = serde_json::json!("operation");
                json
            }
            Self::Renderer(identity) => {
                let mut json = identity.json();
                json["descriptor_family"] = serde_json::json!("renderer");
                json
            }
        }
    }
}

/// Descriptor family for content-addressed descriptor references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DescriptorFamily {
    /// State descriptor reference.
    State,
    /// Operation descriptor reference.
    Operation,
    /// Public-output renderer descriptor reference.
    Renderer,
}

impl DescriptorFamily {
    /// Returns the persisted descriptor family string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Operation => "operation",
            Self::Renderer => "renderer",
        }
    }

    /// Parses a persisted descriptor family string.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "state" => Ok(Self::State),
            "operation" => Ok(Self::Operation),
            "renderer" => Ok(Self::Renderer),
            other => Err(json_error(format!(
                "unsupported descriptor family {other:?}"
            ))),
        }
    }
}

/// Content-addressed reference to a descriptor table entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DescriptorRef {
    /// Descriptor family.
    pub family: DescriptorFamily,
    /// Descriptor identity.
    pub descriptor_id: DescriptorId,
    /// Canonical digest of the descriptor identity payload.
    pub descriptor_digest: ContentDigest,
}

impl DescriptorRef {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "descriptor_digest": self.descriptor_digest.as_str(),
            "descriptor_family": self.family.as_str(),
            "descriptor_id": self.descriptor_id.as_str(),
        })
    }
}

pub(crate) struct DescriptorJsonIndex {
    refs: BTreeMap<String, DescriptorRef>,
}

impl DescriptorJsonIndex {
    pub(crate) fn new(descriptors: &[DescriptorIdentity]) -> Result<Self> {
        let mut refs = BTreeMap::new();
        for descriptor in descriptors {
            let reference = descriptor.descriptor_ref()?;
            let key = reference.descriptor_id.as_str().to_owned();
            if refs.insert(key.clone(), reference).is_some() {
                return Err(json_error(format!("duplicate descriptor identity {key}")));
            }
        }
        Ok(Self { refs })
    }

    pub(crate) fn require(
        &self,
        descriptor_id: &DescriptorId,
        family: DescriptorFamily,
    ) -> Result<&DescriptorRef> {
        let reference = self
            .refs
            .get(descriptor_id.as_str())
            .ok_or_else(|| json_error(format!("missing descriptor identity {descriptor_id}")))?;
        if reference.family != family {
            return Err(json_error(format!(
                "descriptor {descriptor_id} is {}, expected {}",
                reference.family.as_str(),
                family.as_str()
            )));
        }
        Ok(reference)
    }
}

/// State descriptor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDescriptorIdentity {
    /// State descriptor id.
    pub descriptor_id: DescriptorId,
    /// Stable state descriptor name.
    pub name: String,
    /// State kind.
    pub state_kind: StateKind,
    /// State version.
    pub state_version: StateVersion,
    /// State transition-context descriptor contract.
    pub context: StateContextDescriptorSpec,
    /// State input context-resource contract.
    pub input_context: StateInputContextContractSpec,
    /// State output context-resource contract.
    pub output_context: StateOutputContextContractSpec,
    /// Config schema id.
    pub config_schema_id: SchemaId,
    /// Input schema id.
    pub input_schema_id: SchemaId,
    /// Output schema id.
    pub output_schema_id: SchemaId,
    /// Output semantic type id.
    pub output_semantic_type_id: SemanticTypeId,
    /// Effect kind.
    pub effect_kind: EffectKind,
    /// Semantic effect class string.
    pub effect_class: String,
    /// Framework-owned effect descriptor name.
    pub effect_name: String,
    /// Effect descriptor version.
    pub effect_version: EffectVersion,
    /// Capability descriptor set.
    pub capabilities: CapabilitySetDescriptor,
    /// Fact descriptors this state type is allowed to emit.
    pub emitted_fact_descriptors: Vec<FactDescriptorRef>,
    /// Runner kind recorded by the registered state.
    pub runner: String,
    /// Side-effect contract digest for external mutations.
    pub side_effect_contract_digest: Option<ContentDigest>,
}

impl StateDescriptorIdentity {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "capabilities": capability_set_json(&self.capabilities),
            "config_schema_id": self.config_schema_id.as_str(),
            "context": self.context.json(),
            "descriptor_id": self.descriptor_id.as_str(),
            "effect_class": self.effect_class.as_str(),
            "effect_kind": self.effect_kind.as_str(),
            "effect_name": self.effect_name.as_str(),
            "effect_version": self.effect_version.as_str(),
            "emitted_fact_descriptors": self.emitted_fact_descriptors
                .iter()
                .map(FactDescriptorRef::json)
                .collect::<Vec<_>>(),
            "input_context": self.input_context.json(),
            "input_schema_id": self.input_schema_id.as_str(),
            "name": self.name.as_str(),
            "output_context": self.output_context.json(),
            "output_schema_id": self.output_schema_id.as_str(),
            "output_semantic_type_id": self.output_semantic_type_id.as_str(),
            "runner": self.runner.as_str(),
            "side_effect_contract_digest": self.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
            "state_kind": self.state_kind.as_str(),
            "state_version": self.state_version.as_str(),
        })
    }
}

/// Content-addressed fact descriptor authority reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactDescriptorRef {
    /// Canonical fact descriptor content hash.
    pub descriptor_hash: ContentDigest,
}

impl FactDescriptorRef {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "descriptor_hash": self.descriptor_hash.as_str(),
        })
    }
}

/// Operation descriptor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationDescriptorIdentity {
    /// Operation descriptor id.
    pub descriptor_id: DescriptorId,
    /// Stable operation descriptor name.
    pub name: String,
    /// Operation kind.
    pub operation_kind: OperationKind,
    /// Operation version.
    pub operation_version: OperationVersion,
    /// Config schema id.
    pub config_schema_id: SchemaId,
    /// Input schema id.
    pub input_schema_id: SchemaId,
    /// Output schema id.
    pub output_schema_id: SchemaId,
    /// Deterministic expansion ABI.
    pub expansion_abi: String,
}

impl OperationDescriptorIdentity {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "config_schema_id": self.config_schema_id.as_str(),
            "descriptor_id": self.descriptor_id.as_str(),
            "expansion_abi": self.expansion_abi.as_str(),
            "input_schema_id": self.input_schema_id.as_str(),
            "name": self.name.as_str(),
            "operation_kind": self.operation_kind.as_str(),
            "operation_version": self.operation_version.as_str(),
            "output_schema_id": self.output_schema_id.as_str(),
        })
    }
}

/// Renderer descriptor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDescriptorIdentity {
    /// Renderer descriptor id.
    pub descriptor_id: DescriptorId,
    /// Renderer kind.
    pub renderer_kind: RendererKind,
    /// Renderer version.
    pub renderer_version: RendererVersion,
    /// Public output schema id.
    pub public_schema_id: SchemaId,
    /// Canonicalizer identity.
    pub canonicalizer_identity: CanonicalizerIdentity,
}

impl RendererDescriptorIdentity {
    /// Computes the content-addressed renderer descriptor id from descriptor fields.
    pub fn expected_descriptor_id(&self) -> Result<DescriptorId> {
        descriptor_id_from_payload(serde_json::json!({
            "canonicalizer_identity": self.canonicalizer_identity.as_str(),
            "public_schema_id": self.public_schema_id.as_str(),
            "renderer_kind": self.renderer_kind.as_str(),
            "renderer_version": self.renderer_version.as_str(),
        }))
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "canonicalizer_identity": self.canonicalizer_identity.as_str(),
            "descriptor_id": self.descriptor_id.as_str(),
            "public_schema_id": self.public_schema_id.as_str(),
            "renderer_kind": self.renderer_kind.as_str(),
            "renderer_version": self.renderer_version.as_str(),
        })
    }
}

fn descriptor_id_from_payload(value: serde_json::Value) -> Result<DescriptorId> {
    Ok(DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *content_digest(value)?.digest(),
    ))
}
