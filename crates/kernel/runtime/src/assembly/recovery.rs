use super::*;
use mfm_program::{
    Assessment, Classifier, ClassifierAbi, ClassifierBinding, Handler, HandlerAbi, HandlerBinding,
    Incident, IncidentContract, MapAbi, MapBinding, PolicyParams, RecoveryContext, RecoveryRequest,
    ValueMap,
};

#[cfg(test)]
mod tests;

type ErasedIncident = Box<dyn Any + Send + Sync>;
type MapCallback = fn(&QualifiedValue, QualifiedValue) -> Result<QualifiedValue>;
type ClassifyCallback = fn(
    &QualifiedValue,
    &QualifiedValue,
    &QualifiedValue,
    QualifiedIncident,
    &RecoveryContext<'_>,
) -> Result<(Assessment, ErasedIncident)>;
type HandleCallback = fn(
    &QualifiedValue,
    &ErasedIncident,
    Assessment,
    &RecoveryContext<'_>,
) -> Result<RecoveryRequest>;

struct Registration<F> {
    implementation_type: TypeId,
    callback: F,
}

#[derive(Default)]
pub(super) struct Registrations {
    maps: BTreeMap<MapAbi, Registration<MapCallback>>,
    classifiers: BTreeMap<ClassifierAbi, Registration<ClassifyCallback>>,
    handlers: BTreeMap<HandlerAbi, Registration<HandleCallback>>,
}

pub(crate) enum QualifiedIncident {
    Domain(QualifiedValue),
    Adapter {
        original: QualifiedValue,
        context: Box<QualifiedValue>,
    },
}

pub(crate) struct AssociatedRecovery {
    classify: ClassifyCallback,
    handle: HandleCallback,
    domain_params: QualifiedValue,
    context_params: QualifiedValue,
    classifier_params: QualifiedValue,
    handler_params: QualifiedValue,
}

pub(crate) struct AssociatedRootMap {
    input: ContentRef,
    steps: Vec<(MapCallback, QualifiedValue)>,
}

impl AssociatedRootMap {
    pub(crate) fn apply(&self, mut original: QualifiedValue) -> Result<QualifiedValue> {
        if original.contract_ref != self.input {
            return Err(RuntimeError::Internal);
        }
        for (callback, params) in &self.steps {
            original = callback(params, original)?;
        }
        Ok(original)
    }
}

impl AssociatedRecovery {
    pub(crate) fn request(
        &self,
        incident: QualifiedIncident,
        context: &RecoveryContext<'_>,
    ) -> Result<(Assessment, RecoveryRequest)> {
        let (assessment, mapped) = (self.classify)(
            &self.domain_params,
            &self.context_params,
            &self.classifier_params,
            incident,
            context,
        )?;
        let request = (self.handle)(&self.handler_params, &mapped, assessment, context)?;
        Ok((assessment, request))
    }
}

impl RuntimeAssemblyBuilder {
    /// Registers one exact typed conversion and its real value/parameter codecs.
    pub fn register_map<M: ValueMap>(&mut self) -> Result<()> {
        self.ensure_value::<M::Input>()?;
        self.ensure_value::<M::Output>()?;
        self.ensure_value::<M::Params>()?;
        let abi = MapAbi::of::<M>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        insert(
            &mut self.recovery.maps,
            abi,
            TypeId::of::<M>(),
            map::<M> as MapCallback,
        )
    }

    /// Registers the finite exact source, mapped incident, maps, classifier and parameter ABI.
    pub fn register_classifier<E, DM, XM, K>(&mut self) -> Result<()>
    where
        E: MfmValue,
        DM: ValueMap,
        XM: ValueMap,
        K: Classifier<Incident<DM::Output, E, XM::Output>>,
    {
        self.register_map::<DM>()?;
        self.register_map::<XM>()?;
        self.ensure_value::<E>()?;
        self.ensure_value::<K::Params>()?;
        self.register_handler::<Incident<DM::Output, E, XM::Output>, mfm_program::Stop>()?;
        let abi =
            ClassifierAbi::of::<E, DM, XM, K>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        insert(
            &mut self.recovery.classifiers,
            abi,
            TypeId::of::<(E, DM, XM, K)>(),
            classify::<E, DM, XM, K> as ClassifyCallback,
        )
    }

    /// Registers one exact incident/handler association and its checked parameter codec.
    pub fn register_handler<I: IncidentContract, H: Handler<I>>(&mut self) -> Result<()> {
        self.ensure_value::<I::Domain>()?;
        self.ensure_value::<I::Error>()?;
        self.ensure_value::<I::Context>()?;
        self.ensure_value::<H::Params>()?;
        let abi = HandlerAbi::of::<I, H>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        insert(
            &mut self.recovery.handlers,
            abi,
            TypeId::of::<(I, H)>(),
            handle::<I, H> as HandleCallback,
        )
    }
}

fn insert<K: Ord, F>(
    registry: &mut BTreeMap<K, Registration<F>>,
    key: K,
    implementation_type: TypeId,
    callback: F,
) -> Result<()> {
    if let Some(previous) = registry.get(&key) {
        return (previous.implementation_type == implementation_type)
            .then_some(())
            .ok_or(RuntimeError::IncompatibleAssembly);
    }
    registry.insert(
        key,
        Registration {
            implementation_type,
            callback,
        },
    );
    Ok(())
}

impl AssemblyInner {
    pub(crate) fn associate_root_map(
        &self,
        input: &ContentRef,
        output: &ContentRef,
        path: &[MapBinding],
    ) -> Result<AssociatedRootMap> {
        if !self.values.contains_key(input) || !self.values.contains_key(output) {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let mut current = input;
        let mut steps = Vec::with_capacity(path.len());
        for binding in path {
            if binding.abi().input() != current {
                return Err(RuntimeError::IncompatibleAssembly);
            }
            let registration = self
                .recovery
                .maps
                .get(binding.abi())
                .ok_or(RuntimeError::IncompatibleAssembly)?;
            let params = self.policy_params(binding.params(), binding.abi().params())?;
            steps.push((registration.callback, params));
            current = binding.abi().output();
        }
        if current != output {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        Ok(AssociatedRootMap {
            input: input.clone(),
            steps,
        })
    }

    pub(crate) fn associate_recovery(
        &self,
        classifier: &ClassifierBinding,
        handler: &HandlerBinding,
    ) -> Result<AssociatedRecovery> {
        if &classifier.abi().mapped() != handler.abi().input() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let classify = self
            .recovery
            .classifiers
            .get(classifier.abi())
            .ok_or(RuntimeError::IncompatibleAssembly)?
            .callback;
        let handle = self
            .recovery
            .handlers
            .get(handler.abi())
            .ok_or(RuntimeError::IncompatibleAssembly)?
            .callback;
        Ok(AssociatedRecovery {
            classify,
            handle,
            domain_params: self.policy_params(
                classifier.domain_params(),
                classifier.abi().domain_map().params(),
            )?,
            context_params: self.policy_params(
                classifier.context_params(),
                classifier.abi().context_map().params(),
            )?,
            classifier_params: self
                .policy_params(classifier.params(), classifier.abi().params())?,
            handler_params: self.policy_params(handler.params(), handler.abi().params())?,
        })
    }

    fn policy_params(
        &self,
        params: &PolicyParams,
        expected: &ContentRef,
    ) -> Result<QualifiedValue> {
        self.values
            .get(expected)
            .ok_or(RuntimeError::IncompatibleAssembly)?
            .qualify(params.value_ref(), params.canonical_bytes())
            .map_err(|_| RuntimeError::IncompatibleAssembly)
    }
}

fn take<T: MfmValue>(value: QualifiedValue) -> Result<T> {
    value
        .typed
        .downcast::<T>()
        .map(|value| *value)
        .map_err(|_| RuntimeError::Internal)
}

fn borrow<T: MfmValue>(value: &QualifiedValue) -> Result<&T> {
    value
        .typed
        .downcast_ref::<T>()
        .ok_or(RuntimeError::Internal)
}

fn map<M: ValueMap>(params: &QualifiedValue, input: QualifiedValue) -> Result<QualifiedValue> {
    let output = M::apply(borrow::<M::Params>(params)?, take::<M::Input>(input)?)
        .map_err(|_| RuntimeError::Internal)?;
    qualify_hot(output).map_err(RuntimeError::from)
}

fn classify<E, DM, XM, K>(
    domain_params: &QualifiedValue,
    context_params: &QualifiedValue,
    params: &QualifiedValue,
    incident: QualifiedIncident,
    context: &RecoveryContext<'_>,
) -> Result<(Assessment, ErasedIncident)>
where
    E: MfmValue,
    DM: ValueMap,
    XM: ValueMap,
    K: Classifier<Incident<DM::Output, E, XM::Output>>,
{
    let mapped = match incident {
        QualifiedIncident::Domain(value) => {
            Incident::Domain(take::<DM::Output>(map::<DM>(domain_params, value)?)?)
        }
        QualifiedIncident::Adapter { original, context } => Incident::Adapter {
            original: take::<E>(original)?,
            context: take::<XM::Output>(map::<XM>(context_params, *context)?)?,
        },
    };
    let assessment = K::classify(borrow::<K::Params>(params)?, &mapped, context)
        .map_err(|_| RuntimeError::Internal)?;
    Ok((assessment, Box::new(mapped)))
}

fn handle<I: IncidentContract, H: Handler<I>>(
    params: &QualifiedValue,
    incident: &ErasedIncident,
    assessment: Assessment,
    context: &RecoveryContext<'_>,
) -> Result<RecoveryRequest> {
    H::handle(
        borrow::<H::Params>(params)?,
        incident.downcast_ref::<I>().ok_or(RuntimeError::Internal)?,
        assessment,
        context,
    )
    .map_err(|_| RuntimeError::Internal)
}
