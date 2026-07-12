use super::*;

/// Hash-defining certified transition context entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedContextSpec {
    /// Content-addressed context reference.
    pub context_ref: ContextRef,
    /// Descriptor identity for the typed context contract.
    pub context_descriptor_id: ContextDescriptorId,
    /// Schema identity for the context payload.
    pub schema_id: SchemaId,
    /// Semantic type identity for the context payload.
    pub semantic_type_id: SemanticTypeId,
    /// Canonicalizer identity used for the context payload.
    pub canonicalizer_identity: CanonicalizerIdentity,
    /// Canonical JSON context payload.
    pub canonical_context: PlainCanonicalJsonBytes,
    /// Digest of the canonical context payload bytes.
    pub canonical_context_digest: ContentDigest,
    /// Length of the canonical context payload bytes.
    pub canonical_context_byte_len: u64,
}

impl CertifiedContextSpec {
    /// Derives the stable context reference for a canonical context payload.
    pub fn derive_context_ref(
        context_descriptor_id: &ContextDescriptorId,
        schema_id: &SchemaId,
        semantic_type_id: &SemanticTypeId,
        canonicalizer_identity: &CanonicalizerIdentity,
        canonical_context: &PlainCanonicalJsonBytes,
    ) -> Result<ContextRef> {
        let digest = content_digest(serde_json::json!({
            "canonical_context_digest": canonical_context.content_digest().as_str(),
            "canonical_context_byte_len": canonical_context.as_bytes().len() as u64,
            "canonicalizer_identity": canonicalizer_identity.as_str(),
            "context_descriptor_id": context_descriptor_id.as_str(),
            "domain_separator": "mfm.certified_transition_context.v1",
            "schema_id": schema_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
        }))?;
        Ok(ContextRef::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *digest.digest(),
        ))
    }

    pub(super) fn json(&self) -> Result<serde_json::Value> {
        let canonical_context: serde_json::Value =
            serde_json::from_str(self.canonical_context.as_str())
                .map_err(|error| SpecError::Json(error.to_string()))?;
        Ok(serde_json::json!({
            "canonical_context": canonical_context,
            "canonical_context_byte_len": self.canonical_context_byte_len,
            "canonical_context_digest": self.canonical_context_digest.as_str(),
            "canonicalizer_identity": self.canonicalizer_identity.as_str(),
            "context_descriptor_id": self.context_descriptor_id.as_str(),
            "context_ref": self.context_ref.as_str(),
            "schema_id": self.schema_id.as_str(),
            "semantic_type_id": self.semantic_type_id.as_str(),
        }))
    }

    pub(super) fn validate_digest_and_ref(&self) -> Result<()> {
        let digest = self.canonical_context.content_digest();
        if self.canonical_context_digest != digest {
            return Err(json_error(format!(
                "certified context {} digest mismatch: expected {}, recomputed {}",
                self.context_ref, self.canonical_context_digest, digest
            )));
        }
        let byte_len = self.canonical_context.as_bytes().len() as u64;
        if self.canonical_context_byte_len != byte_len {
            return Err(json_error(format!(
                "certified context {} byte length mismatch: expected {}, recomputed {}",
                self.context_ref, self.canonical_context_byte_len, byte_len
            )));
        }
        let derived = Self::derive_context_ref(
            &self.context_descriptor_id,
            &self.schema_id,
            &self.semantic_type_id,
            &self.canonicalizer_identity,
            &self.canonical_context,
        )?;
        if self.context_ref != derived {
            return Err(json_error(format!(
                "certified context ref mismatch: expected {}, recomputed {}",
                self.context_ref, derived
            )));
        }
        Ok(())
    }
}

/// Context requirement for a certified node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeContextSpec {
    /// Node executes outside a semantic transition context.
    NoContext,
    /// Node executes under a certified context reference.
    Required {
        /// Required certified context reference.
        context_ref: ContextRef,
    },
}

impl NodeContextSpec {
    /// Returns a no-context node requirement.
    pub const fn no_context() -> Self {
        Self::NoContext
    }

    pub(super) fn json(&self) -> serde_json::Value {
        match self {
            Self::NoContext => serde_json::json!({
                "kind": "no_context",
            }),
            Self::Required { context_ref } => serde_json::json!({
                "context_ref": context_ref.as_str(),
                "kind": "required",
            }),
        }
    }

    pub(super) fn validate_known_ref(&self, refs: &BTreeMap<String, ()>) -> Result<()> {
        if let Self::Required { context_ref } = self {
            require_context_ref(refs, context_ref)?;
        }
        Ok(())
    }
}

/// Descriptor-level context contract for a state type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateContextDescriptorSpec {
    /// State executes outside a semantic transition context.
    NoContext,
    /// State requires a certified context value with this descriptor contract.
    Required(Box<StateContextDescriptorRequirementSpec>),
}

/// Descriptor-level requirement for a typed state transition context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateContextDescriptorRequirementSpec {
    /// Typed context descriptor identity.
    pub context_descriptor_id: ContextDescriptorId,
    /// Context value schema identity.
    pub schema_id: SchemaId,
    /// Context value semantic identity.
    pub semantic_type_id: SemanticTypeId,
    /// Canonicalizer used for context values.
    pub canonicalizer_identity: CanonicalizerIdentity,
}

impl StateContextDescriptorSpec {
    /// Returns the framework-owned no-context descriptor contract.
    pub const fn no_context() -> Self {
        Self::NoContext
    }

    pub(super) fn json(&self) -> serde_json::Value {
        match self {
            Self::NoContext => serde_json::json!({
                "kind": "no_context",
            }),
            Self::Required(requirement) => {
                let StateContextDescriptorRequirementSpec {
                    context_descriptor_id,
                    schema_id,
                    semantic_type_id,
                    canonicalizer_identity,
                } = requirement.as_ref();
                serde_json::json!({
                    "canonicalizer_identity": canonicalizer_identity.as_str(),
                    "context_descriptor_id": context_descriptor_id.as_str(),
                    "kind": "required",
                    "schema_id": schema_id.as_str(),
                    "semantic_type_id": semantic_type_id.as_str(),
                })
            }
        }
    }
}

/// Certified producer constraint for a context-bound resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextProducerSpec {
    /// Descriptor ids authorized to produce this resource, when node-produced.
    pub producer_descriptor_ids: Vec<DescriptorId>,
    /// Whether seed producers can satisfy this context-bound resource.
    pub seed_producers_allowed: bool,
}

impl ContextProducerSpec {
    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "producer_descriptor_ids": self.producer_descriptor_ids.iter().map(DescriptorId::as_str).collect::<Vec<_>>(),
            "seed_producers_allowed": self.seed_producers_allowed,
        })
    }
}

/// Descriptor-level context contract for a state input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateInputContextContractSpec {
    /// Input does not require a context-bound resource.
    NoContext,
    /// Input must consume a context-bound resource matching this contract.
    Required {
        /// Required resource kind.
        resource_kind: ContextResourceKind,
        /// Required resource stage.
        stage: ContextStage,
        /// Required producer contract.
        producer: Box<ContextProducerSpec>,
    },
}

impl StateInputContextContractSpec {
    /// Returns the no-context input contract.
    pub const fn no_context() -> Self {
        Self::NoContext
    }

    pub(super) fn json(&self) -> serde_json::Value {
        match self {
            Self::NoContext => serde_json::json!({
                "kind": "no_context",
            }),
            Self::Required {
                resource_kind,
                stage,
                producer,
            } => serde_json::json!({
                "kind": "required",
                "producer": producer.json(),
                "resource_kind": resource_kind.as_str(),
                "stage": stage.as_str(),
            }),
        }
    }
}

/// Descriptor-level context contract for a state output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateOutputContextContractSpec {
    /// Output is not context-bound.
    NoContext,
    /// Output produces a context-bound resource under the node's context.
    Produces {
        /// Produced resource kind.
        resource_kind: ContextResourceKind,
        /// Produced resource stage.
        stage: ContextStage,
    },
}

impl StateOutputContextContractSpec {
    /// Returns the no-context output contract.
    pub const fn no_context() -> Self {
        Self::NoContext
    }

    pub(super) fn json(&self) -> serde_json::Value {
        match self {
            Self::NoContext => serde_json::json!({
                "kind": "no_context",
            }),
            Self::Produces {
                resource_kind,
                stage,
            } => serde_json::json!({
                "kind": "produces",
                "resource_kind": resource_kind.as_str(),
                "stage": stage.as_str(),
            }),
        }
    }
}

/// Context constraint for a planned cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellContextSpec {
    /// Cell is not context-bound.
    NoContext,
    /// Cell is bound to a certified context as a specific resource/stage.
    Bound {
        /// Certified context reference.
        context_ref: ContextRef,
        /// Domain resource kind.
        resource_kind: ContextResourceKind,
        /// Domain resource stage.
        stage: ContextStage,
        /// Producer constraint for this resource.
        producer: Box<ContextProducerSpec>,
    },
}

impl CellContextSpec {
    /// Returns a no-context cell constraint.
    pub const fn no_context() -> Self {
        Self::NoContext
    }

    pub(super) fn json(&self) -> serde_json::Value {
        match self {
            Self::NoContext => serde_json::json!({
                "kind": "no_context",
            }),
            Self::Bound {
                context_ref,
                resource_kind,
                stage,
                producer,
            } => serde_json::json!({
                "context_ref": context_ref.as_str(),
                "kind": "bound",
                "producer": producer.json(),
                "resource_kind": resource_kind.as_str(),
                "stage": stage.as_str(),
            }),
        }
    }

    pub(super) fn validate_known_ref(&self, refs: &BTreeMap<String, ()>) -> Result<()> {
        if let Self::Bound { context_ref, .. } = self {
            require_context_ref(refs, context_ref)?;
        }
        Ok(())
    }
}

/// Context constraint for an input cell binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputContextSpec {
    /// Input is not context-bound.
    NoContext,
    /// Input must consume a context-bound resource with these certified constraints.
    Required {
        /// Required certified context reference.
        context_ref: ContextRef,
        /// Required resource kind.
        resource_kind: ContextResourceKind,
        /// Required resource stage.
        stage: ContextStage,
        /// Required producer constraint.
        producer: Box<ContextProducerSpec>,
    },
}

impl InputContextSpec {
    /// Returns a no-context input constraint.
    pub const fn no_context() -> Self {
        Self::NoContext
    }

    pub(super) fn json(&self) -> serde_json::Value {
        match self {
            Self::NoContext => serde_json::json!({
                "kind": "no_context",
            }),
            Self::Required {
                context_ref,
                resource_kind,
                stage,
                producer,
            } => serde_json::json!({
                "context_ref": context_ref.as_str(),
                "kind": "required",
                "producer": producer.json(),
                "resource_kind": resource_kind.as_str(),
                "stage": stage.as_str(),
            }),
        }
    }

    pub(super) fn validate_known_ref(&self, refs: &BTreeMap<String, ()>) -> Result<()> {
        if let Self::Required { context_ref, .. } = self {
            require_context_ref(refs, context_ref)?;
        }
        Ok(())
    }
}
