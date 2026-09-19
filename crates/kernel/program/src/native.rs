//! Typed installed native support, deterministic selection, and explicit live binding.

use crate::{AuthoringSource, EffectSelection, ReadSelection, Result};
use mfm_capabilities::{
    EffectAdapter, EffectCapabilityContract, EffectImplementation, ReadAdapter,
    ReadCapabilityContract, ReadImplementation,
};
use mfm_ids::StableId;
use mfm_values::InvocationDiagnostic;

/// One installed tuple of native implementations for a semantic capability.
pub trait CapabilityFamily<C> {
    /// Concrete implementations visited by both fresh selection and cold discovery.
    type Implementations;
}

/// Selects installed native code from the current checked configuration without IO.
pub trait Resolve<Config: ?Sized, C>: CapabilityFamily<C> {
    /// Returns the unique supported implementation identity.
    fn implementation(config: &Config) -> Result<StableId>;
}

/// Constructs an exact public Read binding from checked configuration.
pub trait ResolveReadBinding<Config: ?Sized, C>: ReadImplementation<C>
where
    C: ReadCapabilityContract,
{
    /// Derives the binding without live handles or IO.
    fn binding(config: &Config) -> Result<Self::Binding>;
}

/// Constructs an exact public Effect binding from checked configuration.
pub trait ResolveEffectBinding<Config: ?Sized, C>: EffectImplementation<C>
where
    C: EffectCapabilityContract,
{
    /// Derives the binding without live handles or IO.
    fn binding(config: &Config) -> Result<Self::Binding>;
}

/// Binds explicit resources to an already-selected native Read implementation.
pub trait BindRead<C, I>
where
    C: ReadCapabilityContract,
    I: ReadImplementation<C>,
{
    /// Bound observational adapter; no invocation occurs during construction.
    type Adapter: ReadAdapter<I::NativeIntent, I::NativeEvidence, I::OperationalError>;
    /// Attaches supplied handles without changing the selected public binding.
    fn bind_read(
        &self,
        binding: &I::Binding,
    ) -> std::result::Result<Self::Adapter, InvocationDiagnostic>;
}

/// Binds explicit resources to an already-selected native Effect implementation.
pub trait BindEffect<C, I>
where
    C: EffectCapabilityContract,
    I: EffectImplementation<C>,
{
    /// Bound mutating adapter; no invocation occurs during construction.
    type Adapter: EffectAdapter<I::NativeCommand, I::NativeEvidence, I::OperationalError>;
    /// Attaches supplied handles without changing the selected public binding.
    fn bind_effect(
        &self,
        binding: &I::Binding,
    ) -> std::result::Result<Self::Adapter, InvocationDiagnostic>;
}

/// Native-owned typed support surrounding one designated Read State.
pub trait InjectRead<S, C>: ReadImplementation<C>
where
    C: ReadCapabilityContract,
    S: ReadSelection<C>,
{
    /// Successful prefix connecting the expanded input to the State input.
    type Prefix: AuthoringSource<Input = S::ExpandedInput, Output = S::Input>;
    /// Successful suffix connecting the State output to the expanded output.
    type Suffix: AuthoringSource<Input = S::Output, Output = S::ExpandedOutput>;
    /// Constructs supporting sources from the exact selected binding without IO.
    fn surround(binding: &Self::Binding) -> Result<(Self::Prefix, Self::Suffix)>;
}

/// Native-owned typed support surrounding one designated Effect State.
pub trait InjectEffect<S, C>: EffectImplementation<C>
where
    C: EffectCapabilityContract,
    S: EffectSelection<C>,
{
    /// Successful prefix connecting the expanded input to the State input.
    type Prefix: AuthoringSource<Input = S::ExpandedInput, Output = S::Input>;
    /// Successful suffix connecting the State output to the expanded output.
    type Suffix: AuthoringSource<Input = S::Output, Output = S::ExpandedOutput>;
    /// Constructs supporting sources from the exact selected binding without IO.
    fn surround(binding: &Self::Binding) -> Result<(Self::Prefix, Self::Suffix)>;
}
