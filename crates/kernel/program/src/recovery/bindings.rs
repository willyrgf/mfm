use std::collections::{btree_map::Entry, BTreeMap};

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::ContentRef;
use mfm_values::{canonicalize_mfm_value, MfmValue};

use super::{
    scope::{ScopeId, ScopedBoundary},
    Checkpoint,
};
use super::{Classifier, Handler, Incident, IncidentContract, ValueMap};
use crate::{implementation_ref, nominal_contract_ref, ProgramError, Result};

/// Exact component contracts for a transient incident; the wrapper has no persisted codec.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncidentAbi {
    domain: ContentRef,
    error: ContentRef,
    context: ContentRef,
}

impl IncidentAbi {
    /// Derives the three exact component schema contracts.
    pub fn of<I: IncidentContract>() -> Result<Self> {
        Ok(Self {
            domain: nominal_contract_ref::<I::Domain>()?,
            error: nominal_contract_ref::<I::Error>()?,
            context: nominal_contract_ref::<I::Context>()?,
        })
    }

    /// Domain failure contract.
    pub const fn domain(&self) -> &ContentRef {
        &self.domain
    }
    /// Operational error contract.
    pub const fn error(&self) -> &ContentRef {
        &self.error
    }
    /// State context contract.
    pub const fn context(&self) -> &ContentRef {
        &self.context
    }
}

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyParams {
    value: ContentRef,
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

impl serde::Serialize for PolicyParams {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let value: &serde_json::value::RawValue =
            serde_json::from_slice(self.canonical_bytes()).map_err(serde::ser::Error::custom)?;
        let mut fields = serializer.serialize_struct("PolicyParams", 2)?;
        fields.serialize_field("value", &self.value)?;
        fields.serialize_field("canonical", &value)?;
        fields.end()
    }
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

/// Exact mapping and classifier association selected for an original incident contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierAbi {
    error: ContentRef,
    domain_map: MapAbi,
    context_map: MapAbi,
    implementation: ContentRef,
    params: ContentRef,
}

impl ClassifierAbi {
    /// Derives the complete finite monomorphized mapping/classification ABI.
    pub fn of<E, DM, XM, K>() -> Result<Self>
    where
        E: MfmValue,
        DM: ValueMap,
        XM: ValueMap,
        K: Classifier<Incident<DM::Output, E, XM::Output>>,
    {
        Ok(Self {
            error: nominal_contract_ref::<E>()?,
            domain_map: MapAbi::of::<DM>()?,
            context_map: MapAbi::of::<XM>()?,
            implementation: implementation_ref("mfm.classifier", K::implementation_id()?)?,
            params: nominal_contract_ref::<K::Params>()?,
        })
    }
    /// Original incident contracts.
    pub fn source(&self) -> IncidentAbi {
        IncidentAbi {
            domain: self.domain_map.input.clone(),
            error: self.error.clone(),
            context: self.context_map.input.clone(),
        }
    }
    /// Derives the mapped incident contract used by classifier and handler.
    pub fn mapped(&self) -> IncidentAbi {
        IncidentAbi {
            domain: self.domain_map.output.clone(),
            error: self.error.clone(),
            context: self.context_map.output.clone(),
        }
    }
    /// Domain conversion ABI.
    pub const fn domain_map(&self) -> &MapAbi {
        &self.domain_map
    }
    /// Context conversion ABI; it never receives the operational cause.
    pub const fn context_map(&self) -> &MapAbi {
        &self.context_map
    }
    /// Classifier implementation identity.
    pub const fn implementation(&self) -> &ContentRef {
        &self.implementation
    }
    /// Classifier parameter contract.
    pub const fn params(&self) -> &ContentRef {
        &self.params
    }
}

/// Exact typed handler association, independent of classifier selection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerAbi {
    input: IncidentAbi,
    implementation: ContentRef,
    params: ContentRef,
}

impl HandlerAbi {
    /// Derives an exact incident/implementation/parameter association.
    pub fn of<I: IncidentContract, H: Handler<I>>() -> Result<Self> {
        Ok(Self {
            input: IncidentAbi::of::<I>()?,
            implementation: implementation_ref("mfm.handler", H::implementation_id()?)?,
            params: nominal_contract_ref::<H::Params>()?,
        })
    }
    /// Handler's policy-facing incident contract.
    pub const fn input(&self) -> &IncidentAbi {
        &self.input
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

/// Immutable classifier descriptor; no Runtime callback is stored in authoring.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierBinding {
    abi: ClassifierAbi,
    domain_params: PolicyParams,
    context_params: PolicyParams,
    params: PolicyParams,
}

impl ClassifierBinding {
    pub(crate) fn new<E, DM, XM, K>(
        domain_params: DM::Params,
        context_params: XM::Params,
        classifier_params: K::Params,
    ) -> Result<Self>
    where
        E: MfmValue,
        DM: ValueMap,
        XM: ValueMap,
        K: Classifier<Incident<DM::Output, E, XM::Output>>,
    {
        Ok(Self {
            abi: ClassifierAbi::of::<E, DM, XM, K>()?,
            domain_params: PolicyParams::new(&domain_params)?,
            context_params: PolicyParams::new(&context_params)?,
            params: PolicyParams::new(&classifier_params)?,
        })
    }

    /// Complete exact association contract.
    pub const fn abi(&self) -> &ClassifierAbi {
        &self.abi
    }
    /// Domain mapping parameters.
    pub const fn domain_params(&self) -> &PolicyParams {
        &self.domain_params
    }
    /// Context mapping parameters.
    pub const fn context_params(&self) -> &PolicyParams {
        &self.context_params
    }
    /// Classifier parameters.
    pub const fn params(&self) -> &PolicyParams {
        &self.params
    }
}

/// Immutable handler descriptor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerBinding {
    abi: HandlerAbi,
    params: PolicyParams,
}

impl HandlerBinding {
    pub(crate) fn stop(input: IncidentAbi) -> Result<Self> {
        Ok(Self {
            abi: HandlerAbi {
                input,
                implementation: implementation_ref("mfm.handler", super::Stop::id()?)?,
                params: nominal_contract_ref::<super::NoParams>()?,
            },
            params: PolicyParams::new(&super::NoParams)?,
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

/// Finite exact classifier family; a missing source is an error, never an outer fallback.
#[derive(Debug, Clone, Default)]
pub struct Classifiers(BTreeMap<IncidentAbi, ClassifierBinding>);

impl Classifiers {
    /// Creates an empty explicit family.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds one original incident contract to explicit typed maps and classifier.
    pub fn bind<E, DM, XM, K>(
        &mut self,
        domain_params: DM::Params,
        context_params: XM::Params,
        classifier_params: K::Params,
    ) -> Result<()>
    where
        E: MfmValue,
        DM: ValueMap,
        XM: ValueMap,
        K: Classifier<Incident<DM::Output, E, XM::Output>>,
    {
        let source = IncidentAbi::of::<Incident<DM::Input, E, XM::Input>>()?;
        let Entry::Vacant(slot) = self.0.entry(source) else {
            return Err(ProgramError::InvalidContract);
        };
        slot.insert(ClassifierBinding::new::<E, DM, XM, K>(
            domain_params,
            context_params,
            classifier_params,
        )?);
        Ok(())
    }

    /// Selects an exact source binding without conversion search.
    pub fn binding(&self, source: &IncidentAbi) -> Result<&ClassifierBinding> {
        self.0.get(source).ok_or(ProgramError::InvalidContract)
    }
}

/// Finite independently selected handler family.
#[derive(Debug, Clone, Default)]
pub struct Handlers(BTreeMap<IncidentAbi, HandlerSetting>);

#[derive(Debug, Clone)]
pub(super) struct HandlerSetting {
    pub(super) binding: HandlerBinding,
    pub(super) checkpoints: Vec<ScopedBoundary>,
}

impl Handlers {
    /// Creates an empty explicit family.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds one exact mapped incident; duplicate bindings fail construction.
    pub fn bind<I: IncidentContract, H: Handler<I>>(&mut self, params: H::Params) -> Result<()> {
        let abi = HandlerAbi::of::<I, H>()?;
        let Entry::Vacant(slot) = self.0.entry(abi.input.clone()) else {
            return Err(ProgramError::InvalidContract);
        };
        slot.insert(HandlerSetting {
            binding: HandlerBinding {
                abi,
                params: PolicyParams::new(&params)?,
            },
            checkpoints: Vec::new(),
        });
        Ok(())
    }

    /// Attaches a typed checkpoint to an existing exact incident binding.
    /// Installation in a compiler scope separately checks token ownership.
    pub fn checkpoint<I: IncidentContract, T: MfmValue>(
        &mut self,
        checkpoint: &Checkpoint<T>,
    ) -> Result<()> {
        let setting = self
            .0
            .get_mut(&IncidentAbi::of::<I>()?)
            .ok_or(ProgramError::InvalidContract)?;
        if setting.checkpoints.contains(&checkpoint.boundary) {
            return Err(ProgramError::InvalidContract);
        }
        setting.checkpoints.push(checkpoint.boundary.clone());
        Ok(())
    }

    pub(crate) fn require_scope(&self, owner: &ScopeId) -> Result<()> {
        for setting in self.0.values() {
            for checkpoint in &setting.checkpoints {
                checkpoint.require_owner(owner)?;
            }
        }
        Ok(())
    }

    pub(super) fn setting(&self, input: &IncidentAbi) -> Result<&HandlerSetting> {
        self.0.get(input).ok_or(ProgramError::InvalidContract)
    }

    /// Selects an exact policy input binding without an outer-family fallback.
    pub fn binding(&self, input: &IncidentAbi) -> Result<&HandlerBinding> {
        self.setting(input).map(|setting| &setting.binding)
    }
}
