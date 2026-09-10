use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::ContentRef;
use mfm_values::{canonicalize_mfm_value, MfmValue};

use super::{
    scope::{ScopeId, ScopedBoundary},
    Checkpoint,
};
use super::{Handler, ValueMap};
use crate::{implementation_ref, nominal_contract_ref, ProgramError, Result};

/// Exact implementation and value contracts of an explicit conversion.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapAbi {
    implementation: ContentRef,
    input: ContentRef,
    output: ContentRef,
    params: ContentRef,
}

impl MapAbi {
    /// Derives a complete conversion ABI without executable callbacks.
    pub fn of<M: ValueMap>() -> Result<Self> {
        Ok(Self {
            implementation: implementation_ref("mfm.value-map", M::implementation_id()?)?,
            input: nominal_contract_ref::<M::Input>()?,
            output: nominal_contract_ref::<M::Output>()?,
            params: nominal_contract_ref::<M::Params>()?,
        })
    }
    /// Implementation identity.
    pub const fn implementation(&self) -> &ContentRef {
        &self.implementation
    }
    /// Exact input contract.
    pub const fn input(&self) -> &ContentRef {
        &self.input
    }
    /// Exact output contract.
    pub const fn output(&self) -> &ContentRef {
        &self.output
    }
    /// Exact parameter contract.
    pub const fn params(&self) -> &ContentRef {
        &self.params
    }
}

/// An exact typed conversion with immutable parameters, used in explicit root mapping paths.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapBinding {
    abi: MapAbi,
    params: PolicyParams,
}

impl MapBinding {
    /// Binds one explicit typed conversion to checked immutable parameters.
    pub fn new<M: ValueMap>(params: &M::Params) -> Result<Self> {
        Ok(Self {
            abi: MapAbi::of::<M>()?,
            params: PolicyParams::new(params)?,
        })
    }

    /// Returns the exact conversion ABI.
    pub const fn abi(&self) -> &MapAbi {
        &self.abi
    }

    /// Returns the qualified parameters.
    pub const fn params(&self) -> &PolicyParams {
        &self.params
    }
}

/// Qualified immutable policy parameters bound into Program identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PolicyParams {
    value: ContentRef,
    #[serde(serialize_with = "serialize_canonical")]
    canonical: PlainCanonicalJsonBytes,
}

impl PolicyParams {
    /// Qualifies bounded canonical parameters with their exact schema.
    pub fn new<T: MfmValue>(value: &T) -> Result<Self> {
        let (canonical, value) =
            canonicalize_mfm_value(value).map_err(|_| ProgramError::InvalidContract)?;
        Ok(Self { value, canonical })
    }
    pub(crate) fn matches(&self, contract: &ContentRef) -> bool {
        self.value.schema_id() == contract.schema_id()
            && contract.content_digest() == &mfm_canonical::raw_content_digest(b"mfm.contract.v1")
    }
    /// Exact parameter instance identity.
    pub const fn value_ref(&self) -> &ContentRef {
        &self.value
    }
    /// Canonical checked parameter bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

fn serialize_canonical<S: serde::Serializer>(
    canonical: &PlainCanonicalJsonBytes,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let raw: &serde_json::value::RawValue =
        serde_json::from_slice(canonical.as_bytes()).map_err(serde::ser::Error::custom)?;
    serde::Serialize::serialize(raw, serializer)
}

impl<'de> serde::Deserialize<'de> for PolicyParams {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            value: ContentRef,
            canonical: Box<serde_json::value::RawValue>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(wire.canonical.get())
            .map_err(serde::de::Error::custom)?;
        if canonical.as_bytes().len() > mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES
            || wire.value.content_digest()
                != &mfm_canonical::raw_content_digest(canonical.as_bytes())
        {
            return Err(serde::de::Error::custom("invalid policy parameters"));
        }
        Ok(Self {
            value: wire.value,
            canonical,
        })
    }
}

/// Exact handler implementation and parameter contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerAbi {
    implementation: ContentRef,
    params: ContentRef,
}

impl HandlerAbi {
    /// Derives an exact implementation/parameter association.
    pub fn of<H: Handler>() -> Result<Self> {
        Ok(Self {
            implementation: implementation_ref("mfm.handler", H::implementation_id()?)?,
            params: nominal_contract_ref::<H::Params>()?,
        })
    }
    /// Handler implementation identity.
    pub const fn implementation(&self) -> &ContentRef {
        &self.implementation
    }
    /// Parameter contract.
    pub const fn params(&self) -> &ContentRef {
        &self.params
    }
}

/// Immutable selected handler and its scoped authoring targets.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerBinding {
    abi: HandlerAbi,
    params: PolicyParams,
    // Lowering moves these authoring-only tokens into checked declaration targets.
    #[serde(skip)]
    pub(crate) checkpoints: Vec<ScopedBoundary>,
}

impl HandlerBinding {
    /// Binds a static handler to its immutable checked parameters.
    pub fn new<H: Handler>(params: H::Params) -> Result<Self> {
        Ok(Self {
            abi: HandlerAbi::of::<H>()?,
            params: PolicyParams::new(&params)?,
            checkpoints: Vec::new(),
        })
    }

    /// Attaches a typed target; installation separately checks scope ownership.
    pub fn checkpoint<T: MfmValue>(mut self, checkpoint: &Checkpoint<T>) -> Result<Self> {
        if self.checkpoints.contains(&checkpoint.boundary) {
            return Err(ProgramError::InvalidContract);
        }
        self.checkpoints.push(checkpoint.boundary.clone());
        Ok(self)
    }

    pub(crate) fn require_scope(&self, owner: &ScopeId) -> Result<()> {
        for checkpoint in &self.checkpoints {
            checkpoint.require_owner(owner)?;
        }
        Ok(())
    }

    /// Complete exact association contract.
    pub const fn abi(&self) -> &HandlerAbi {
        &self.abi
    }
    /// Handler parameters.
    pub const fn params(&self) -> &PolicyParams {
        &self.params
    }
}
