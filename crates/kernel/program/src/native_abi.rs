//! Exact selected semantic/native code association retained in each capability occurrence.

use crate::{
    capability_contract_ref, effect_capability_contract_ref, nominal_contract_ref, Result,
};
use mfm_capabilities::{
    EffectCapabilityContract, EffectImplementation, ReadCapabilityContract, ReadImplementation,
};
use mfm_ids::ContentRef;

/// Complete native association; binding values are retained separately by content identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAbi {
    pub(crate) implementation: ContentRef,
    pub(crate) capability: ContentRef,
    pub(crate) request: ContentRef,
    pub(crate) evidence: ContentRef,
    pub(crate) native_request: ContentRef,
    pub(crate) native_evidence: ContentRef,
    pub(crate) operational_error: ContentRef,
    pub(crate) binding: ContentRef,
}
impl NativeAbi {
    /// Derives the complete exact ABI from its owning Read capability and implementation.
    pub fn read<C, I>() -> Result<Self>
    where
        C: ReadCapabilityContract,
        I: ReadImplementation<C>,
    {
        Self::from_contracts(
            I::implementation_id()?,
            capability_contract_ref::<C>()?,
            [
                nominal_contract_ref::<C::Intent>()?,
                nominal_contract_ref::<C::Evidence>()?,
                nominal_contract_ref::<I::NativeIntent>()?,
                nominal_contract_ref::<I::NativeEvidence>()?,
                nominal_contract_ref::<I::OperationalError>()?,
                nominal_contract_ref::<I::Binding>()?,
            ],
        )
    }
    /// Derives the complete exact ABI from its owning Effect capability and implementation.
    pub fn effect<C, I>() -> Result<Self>
    where
        C: EffectCapabilityContract,
        I: EffectImplementation<C>,
    {
        Self::from_contracts(
            I::implementation_id()?,
            effect_capability_contract_ref::<C>()?,
            [
                nominal_contract_ref::<C::Command>()?,
                nominal_contract_ref::<C::Evidence>()?,
                nominal_contract_ref::<I::NativeCommand>()?,
                nominal_contract_ref::<I::NativeEvidence>()?,
                nominal_contract_ref::<I::OperationalError>()?,
                nominal_contract_ref::<I::Binding>()?,
            ],
        )
    }
    fn from_contracts(
        id: mfm_ids::StableId,
        capability: ContentRef,
        [request, evidence, native_request, native_evidence, operational_error, binding]: [ContentRef;
            6],
    ) -> Result<Self> {
        #[derive(serde::Serialize)]
        struct Descriptor<'a> {
            domain: &'static str,
            id: mfm_ids::StableId,
            capability: &'a ContentRef,
            request: &'a ContentRef,
            evidence: &'a ContentRef,
            native_request: &'a ContentRef,
            native_evidence: &'a ContentRef,
            operational_error: &'a ContentRef,
            binding: &'a ContentRef,
        }
        let json = serde_json::to_string(&Descriptor {
            domain: "mfm.native-implementation.v2",
            id,
            capability: &capability,
            request: &request,
            evidence: &evidence,
            native_request: &native_request,
            native_evidence: &native_evidence,
            operational_error: &operational_error,
            binding: &binding,
        })
        .map_err(mfm_canonical::JsonError::new)?;
        let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)?;
        let schema = mfm_ids::SchemaId::new(
            "mfm.native-implementation",
            "2",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0; 32]),
        )?;
        let implementation = ContentRef::new(
            schema,
            mfm_canonical::raw_content_digest(canonical.as_bytes()),
        )?;
        Ok(Self {
            implementation,
            capability,
            request,
            evidence,
            native_request,
            native_evidence,
            operational_error,
            binding,
        })
    }
    /// Exact installed native implementation identity.
    pub fn implementation(&self) -> &ContentRef {
        &self.implementation
    }
    /// Semantic capability contract, including its mode.
    pub fn capability(&self) -> &ContentRef {
        &self.capability
    }
    /// Semantic intent or command contract.
    pub fn request(&self) -> &ContentRef {
        &self.request
    }
    /// Projected semantic evidence contract.
    pub fn evidence(&self) -> &ContentRef {
        &self.evidence
    }
    /// Native intent or command contract.
    pub fn native_request(&self) -> &ContentRef {
        &self.native_request
    }
    /// Authoritative native evidence contract.
    pub fn native_evidence(&self) -> &ContentRef {
        &self.native_evidence
    }
    /// Original native operational error contract.
    pub fn operational_error(&self) -> &ContentRef {
        &self.operational_error
    }
    /// Exact public binding contract.
    pub fn binding(&self) -> &ContentRef {
        &self.binding
    }
    pub(crate) fn contracts(&self) -> [&ContentRef; 6] {
        [
            &self.request,
            &self.evidence,
            &self.native_request,
            &self.native_evidence,
            &self.operational_error,
            &self.binding,
        ]
    }
}

/// Derives the exact selected Read implementation reference, including every semantic/native ABI contract.
pub fn read_implementation_ref<C, I>() -> Result<ContentRef>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<C>,
{
    Ok(NativeAbi::read::<C, I>()?.implementation)
}

/// Derives the exact selected Effect implementation reference for deterministic native preparation.
/// Request specializations of one implementation family retain distinct references.
pub fn effect_implementation_ref<C, I>() -> Result<ContentRef>
where
    C: EffectCapabilityContract,
    I: EffectImplementation<C>,
{
    Ok(NativeAbi::effect::<C, I>()?.implementation)
}
