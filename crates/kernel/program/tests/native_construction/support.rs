//! Native construction fixtures exercise the public compiler, not a maintained product workflow.
#[allow(dead_code)]
#[path = "../phase_a/contracts.rs"]
mod contracts;
pub(crate) use contracts::*;
use mfm_capabilities::*;
use mfm_ids::{ContentRef, StableId};
use mfm_program::*;
use mfm_values::InvocationDiagnostic;
use std::result::Result;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

pub(crate) struct Primary;
impl ReadImplementation<Observation> for Primary {
    type Binding = NoParams;
    type NativeIntent = Prepared;
    type NativeEvidence = Deployed;
    type OperationalError = PrimaryFailure;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("proof.primary@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        intent: &Configured,
    ) -> Result<Prepared, mfm_capabilities::CallbackFailure> {
        Ok(Prepared {
            value: intent.value,
        })
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &Configured,
        _: &ContentRef,
        _: &Prepared,
        evidence: &Deployed,
        _: &mfm_values::Object,
    ) -> Result<Observed, mfm_capabilities::CallbackFailure> {
        Ok(Observed {
            value: evidence.value,
        })
    }
}
pub(crate) struct Alternate;
impl ReadImplementation<Observation> for Alternate {
    type Binding = Configured;
    type NativeIntent = Configured;
    type NativeEvidence = Observed;
    type OperationalError = AlternateFailure;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("proof.alternate@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        _: &Configured,
        intent: &Configured,
    ) -> Result<Configured, mfm_capabilities::CallbackFailure> {
        Ok(Configured {
            value: intent.value,
        })
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &Configured,
        _: &ContentRef,
        _: &Configured,
        _: &ContentRef,
        _: &Configured,
        evidence: &Observed,
        _: &mfm_values::Object,
    ) -> Result<Observed, mfm_capabilities::CallbackFailure> {
        Ok(Observed {
            value: evidence.value,
        })
    }
}
pub(crate) struct Support;
impl State for Support {
    type Input = Configured;
    type Output = Configured;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("proof.support@1")?)
    }
}
impl ReadState<Observation> for Support {
    fn prepare(input: &Configured) -> Result<Configured, InvocationDiagnostic> {
        Ok(Configured { value: input.value })
    }
    fn interpret(
        _: Configured,
        evidence: &Observed,
    ) -> Result<ProposedStateOutcome<Configured, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success {
            output: Configured {
                value: evidence.value,
            },
        })
    }
}
impl ReadSelection<Observation> for Support {
    type ExpandedInput = Configured;
    type ExpandedOutput = Configured;
}
impl InjectRead<Support, Observation> for Alternate {
    type Prefix = Identity<Configured>;
    type Suffix = Identity<Configured>;
    fn surround(_: &Configured) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
impl InjectRead<Observe, Observation> for Primary {
    type Prefix = ResolvedRead<Support, Observation, Alternate>;
    type Suffix = Identity<Observed>;
    fn surround(_: &NoParams) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((
            ResolvedRead::new(Configured { value: 0 }),
            Identity::default(),
        ))
    }
}
impl InjectRead<Observe, Observation> for Alternate {
    type Prefix = ResolvedRead<Support, Observation, Alternate>;
    type Suffix = Identity<Observed>;
    fn surround(_: &Configured) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((
            ResolvedRead::new(Configured { value: 0 }),
            Identity::default(),
        ))
    }
}
impl ResolveReadBinding<Configured, Observation> for Primary {
    fn binding(_: &Configured) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
}
impl ResolveReadBinding<Configured, Observation> for Alternate {
    fn binding(input: &Configured) -> mfm_program::Result<Configured> {
        Ok(Configured { value: input.value })
    }
}
pub(crate) struct Resources<const COLD: bool, Family = (Primary, Alternate)> {
    pub(crate) family: std::marker::PhantomData<Family>,
    pub(crate) bindings: Arc<Mutex<Vec<u64>>>,
    pub(crate) primary_available: bool,
    pub(crate) execution: ProviderScript,
}
impl<const COLD: bool, Family> ProgramEnvironment for Resources<COLD, Family> {
    type Sources = (
        Read<Observe, Observation>,
        Pure<Add>,
        Read<Observe, Observation>,
    );
}
impl<const COLD: bool, Family> CapabilityFamily<Observation> for Resources<COLD, Family> {
    type Implementations = Family;
}
impl<Family> Resolve<Configured, Observation> for Resources<false, Family> {
    fn implementation(input: &Configured) -> mfm_program::Result<StableId> {
        Ok(match input.value {
            1 => Primary::implementation_id()?,
            2 => Alternate::implementation_id()?,
            _ => StableId::new("proof.uninstalled@1")?,
        })
    }
}
#[derive(Clone)]
pub(crate) enum ProviderScript {
    Forbidden,
    Success(Arc<std::sync::atomic::AtomicUsize>),
    Failure(Arc<std::sync::atomic::AtomicUsize>),
}
impl ProviderScript {
    fn invoke(&self) -> bool {
        let (counter, failure) = match self {
            Self::Forbidden => panic!("construction must not invoke the provider"),
            Self::Success(counter) => (counter, false),
            Self::Failure(counter) => (counter, true),
        };
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        failure
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrimaryFailure {
    pub(crate) code: u64,
}
impl ClassifyError for PrimaryFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlternateFailure {
    pub(crate) code: String,
}
impl ClassifyError for AlternateFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}
pub(crate) struct PrimaryAdapter(ProviderScript);
impl ReadAdapter<Prepared, Deployed, PrimaryFailure> for PrimaryAdapter {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        intent: &'a Prepared,
    ) -> Pin<Box<dyn Future<Output = Result<Deployed, AdapterError<PrimaryFailure>>> + Send + 'a>>
    {
        Box::pin(async move {
            if self.0.invoke() {
                Err(AdapterError::Operational(PrimaryFailure { code: 73 }))
            } else {
                Ok(Deployed {
                    value: intent.value,
                })
            }
        })
    }
}
pub(crate) struct AlternateAdapter {
    execution: ProviderScript,
    support: bool,
}
impl ReadAdapter<Configured, Observed, AlternateFailure> for AlternateAdapter {
    fn invoke<'a>(
        &'a self,
        _: &'a ContentRef,
        _: &'a ContentRef,
        intent: &'a Configured,
    ) -> Pin<Box<dyn Future<Output = Result<Observed, AdapterError<AlternateFailure>>> + Send + 'a>>
    {
        Box::pin(async move {
            if self.execution.invoke() && !self.support {
                Err(AdapterError::Operational(AlternateFailure {
                    code: "alternate-91".into(),
                }))
            } else {
                Ok(Observed {
                    value: intent.value,
                })
            }
        })
    }
}
impl<const COLD: bool, Family> BindRead<Observation, Primary> for Resources<COLD, Family> {
    type Adapter = PrimaryAdapter;
    fn bind_read(&self, _: &NoParams) -> Result<PrimaryAdapter, InvocationDiagnostic> {
        assert!(
            self.primary_available,
            "unselected implementation requested unavailable resources"
        );
        self.bindings.lock().unwrap().push(1);
        Ok(PrimaryAdapter(self.execution.clone()))
    }
}
impl<const COLD: bool, Family> BindRead<Observation, Alternate> for Resources<COLD, Family> {
    type Adapter = AlternateAdapter;
    fn bind_read(&self, binding: &Configured) -> Result<AlternateAdapter, InvocationDiagnostic> {
        self.bindings.lock().unwrap().push(binding.value);
        Ok(AlternateAdapter {
            execution: self.execution.clone(),
            support: binding.value == 0,
        })
    }
}
