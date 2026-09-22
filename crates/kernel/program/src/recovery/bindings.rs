use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::ContentRef;
use mfm_values::{canonicalize_mfm_value, MfmValue};

use super::Handler;
use crate::{implementation_ref, nominal_contract_ref, ProgramError, Result};

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
        let (canonical, value) = canonicalize_mfm_value(value).map_err(|cause| {
            ProgramError::Diagnostic(cause.into_diagnostic("policy_parameters"))
        })?;
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
        Self::from_contract::<H>(nominal_contract_ref::<H::Params>()?)
    }
    pub(crate) fn from_contract<H: Handler>(params: ContentRef) -> Result<Self> {
        Ok(Self {
            implementation: implementation_ref("mfm.handler", H::implementation_id()?)?,
            params,
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

/// Immutable selected handler and its checked parameters.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerBinding {
    abi: HandlerAbi,
    params: PolicyParams,
}

impl HandlerBinding {
    /// Binds a static handler to its immutable checked parameters.
    pub fn new<H: Handler>(params: H::Params) -> Result<Self> {
        Self::from_abi(HandlerAbi::of::<H>()?, &params)
    }
    pub(crate) fn from_abi<T: MfmValue>(abi: HandlerAbi, params: &T) -> Result<Self> {
        Ok(Self {
            abi,
            params: PolicyParams::new(params)?,
        })
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
