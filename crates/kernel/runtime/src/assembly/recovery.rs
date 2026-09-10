use super::*;
use mfm_program::{
    ClassifyError, Handler, HandlerAbi, HandlerBinding, IncidentSource, IncidentSummary, MapAbi,
    MapBinding, PolicyParams, RecoveryContext, RecoveryRequest, ValueMap,
};

#[cfg(test)]
mod tests;

type MapCallback = fn(&QualifiedValue, QualifiedValue) -> Result<QualifiedValue>;
pub(super) type ClassifyCallback = fn(&QualifiedIncident<'_>) -> Result<IncidentSummary>;
type HandleCallback =
    fn(&QualifiedValue, &IncidentSummary, &RecoveryContext<'_>) -> Result<RecoveryRequest>;

struct Registration<F> {
    implementation_type: TypeId,
    callback: F,
}

#[derive(Default)]
pub(super) struct Registrations {
    maps: BTreeMap<MapAbi, Registration<MapCallback>>,
    handlers: BTreeMap<HandlerAbi, Registration<HandleCallback>>,
}

pub(crate) enum QualifiedIncident<'a> {
    Domain(&'a QualifiedValue),
    Adapter { original: &'a QualifiedValue },
}

pub(crate) struct AssociatedRecovery {
    classify: ClassifyCallback,
    handle: HandleCallback,
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
        incident: QualifiedIncident<'_>,
        context: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest> {
        let summary = (self.classify)(&incident)?;
        (self.handle)(&self.handler_params, &summary, context)
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

    /// Registers one handler implementation and its exact parameter codec.
    pub fn register_handler<H: Handler>(&mut self) -> Result<()> {
        self.ensure_value::<H::Params>()?;
        let abi = HandlerAbi::of::<H>().map_err(|_| RuntimeError::IncompatibleAssembly)?;
        insert(
            &mut self.recovery.handlers,
            abi,
            TypeId::of::<H>(),
            handle::<H> as HandleCallback,
        )
    }
}

fn insert<K: Ord, F>(
    registry: &mut BTreeMap<K, Registration<F>>,
    key: K,
    implementation_type: TypeId,
    callback: F,
) -> Result<()> {
    match registry.entry(key) {
        std::collections::btree_map::Entry::Occupied(slot) => (slot.get().implementation_type
            == implementation_type)
            .then_some(())
            .ok_or(RuntimeError::IncompatibleAssembly),
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(Registration {
                implementation_type,
                callback,
            });
            Ok(())
        }
    }
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
        classify: ClassifyCallback,
        handler: &HandlerBinding,
    ) -> Result<AssociatedRecovery> {
        let handle = self
            .recovery
            .handlers
            .get(handler.abi())
            .ok_or(RuntimeError::IncompatibleAssembly)?
            .callback;
        Ok(AssociatedRecovery {
            classify,
            handle,
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

pub(super) fn classify<D: ClassifyError, E: ClassifyError>(
    incident: &QualifiedIncident<'_>,
) -> Result<IncidentSummary> {
    Ok(match incident {
        QualifiedIncident::Domain(value) => IncidentSummary {
            source: IncidentSource::State,
            classification: borrow::<D>(value)?.classify(),
        },
        QualifiedIncident::Adapter { original, .. } => IncidentSummary {
            source: IncidentSource::Adapter,
            classification: borrow::<E>(original)?.classify(),
        },
    })
}

fn handle<H: Handler>(
    params: &QualifiedValue,
    incident: &IncidentSummary,
    context: &RecoveryContext<'_>,
) -> Result<RecoveryRequest> {
    H::handle(borrow::<H::Params>(params)?, incident, context).map_err(|_| RuntimeError::Internal)
}
