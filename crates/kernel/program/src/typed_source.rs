//! Typed source construction and sealed fresh/discovery traversal.

use std::marker::PhantomData;

use crate::construction::{Discover, Draft, Inventory, Walk};
use crate::{EffectState, Handler, PureState, ReadState, Result, Stop};
use mfm_capabilities::{EffectCapabilityContract, ReadCapabilityContract};
use mfm_values::MfmValue;

mod sealed {
    pub trait Sealed {}
    pub trait Targets {
        fn targets() -> super::Result<Vec<(std::any::TypeId, mfm_ids::ContentRef)>>;
    }
}

/// A finite typed source whose outer endpoints include capability injection.
pub trait AuthoringSource: sealed::Sealed {
    /// Complete input of the expanded sequence.
    type Input: MfmValue;
    /// Complete success output of the expanded sequence.
    type Output: MfmValue;
}

/// The fixed outer endpoints of an injected Effect selection.
pub trait EffectSelection<C>: EffectState<C>
where
    C: EffectCapabilityContract,
{
    /// Input before the native prefix.
    type ExpandedInput: MfmValue;
    /// Output after the native suffix.
    type ExpandedOutput: MfmValue;
}

/// The fixed outer endpoints of an injected Read selection.
pub trait ReadSelection<C>: ReadState<C>
where
    C: ReadCapabilityContract,
{
    /// Input before the native prefix.
    type ExpandedInput: MfmValue;
    /// Output after the native suffix.
    type ExpandedOutput: MfmValue;
}

macro_rules! marker {
    ($name:ident<$($parameter:ident),+>) => {
        /// Typed source selection; construction does not instantiate its executable State.
        pub struct $name<$($parameter),+>(PhantomData<fn() -> ($($parameter,)+)>);
        impl<$($parameter),+> Copy for $name<$($parameter),+> {}
        impl<$($parameter),+> Clone for $name<$($parameter),+> {
            fn clone(&self) -> Self { *self }
        }
        impl<$($parameter),+> Default for $name<$($parameter),+> {
            fn default() -> Self { Self(PhantomData) }
        }
    };
}

marker!(Pure<S>);
marker!(Read<S, C>);
marker!(Effect<S, C>);
marker!(Identity<T>);
marker!(Checkpoint<M>);

/// A semantic checkpoint marker with one exact input context.
pub trait CheckpointMarker: 'static {
    /// Context restored when this checkpoint is selected.
    type Context: MfmValue;
}
/// Sealed tuple of checkpoint marker types selected as one handler unit.
pub trait CheckpointTargets: sealed::Targets {}
impl sealed::Targets for () {
    fn targets() -> Result<Vec<(std::any::TypeId, mfm_ids::ContentRef)>> {
        Ok(Vec::new())
    }
}
impl CheckpointTargets for () {}
impl<M: CheckpointMarker> sealed::Sealed for Checkpoint<M> {}
impl<M: CheckpointMarker> AuthoringSource for Checkpoint<M> {
    type Input = M::Context;
    type Output = M::Context;
}
impl<C: ?Sized, R, M: CheckpointMarker> Walk<C, R> for Checkpoint<M> {
    fn walk(&self, _: &C, _: &R, draft: &mut Draft) -> Result<()> {
        draft.checkpoint::<M>()
    }
}
impl<R, M: CheckpointMarker> Discover<R> for Checkpoint<M> {
    fn discover(inventory: &mut Inventory<R>) -> Result<()> {
        inventory.value::<M::Context>()
    }
}

impl<S: PureState> sealed::Sealed for Pure<S> {}
impl<S: PureState> AuthoringSource for Pure<S> {
    type Input = S::Input;
    type Output = S::Output;
}

impl<S, C> sealed::Sealed for Read<S, C>
where
    C: ReadCapabilityContract,
    S: ReadSelection<C>,
{
}
impl<S, C> AuthoringSource for Read<S, C>
where
    C: ReadCapabilityContract,
    S: ReadSelection<C>,
{
    type Input = S::ExpandedInput;
    type Output = S::ExpandedOutput;
}

impl<S, C> sealed::Sealed for Effect<S, C>
where
    C: EffectCapabilityContract,
    S: EffectSelection<C>,
{
}
impl<S, C> AuthoringSource for Effect<S, C>
where
    C: EffectCapabilityContract,
    S: EffectSelection<C>,
{
    type Input = S::ExpandedInput;
    type Output = S::ExpandedOutput;
}

impl<T: MfmValue> sealed::Sealed for Identity<T> {}
impl<T: MfmValue> AuthoringSource for Identity<T> {
    type Input = T;
    type Output = T;
}

/// An Operation's statically discoverable body, independent of planning configuration.
pub trait OperationDefinition {
    /// Source structure traversed by both fresh construction and cold discovery.
    type Body: AuthoringSource;
    /// Optional stable public name and non-semantic description for inspection.
    fn metadata() -> Option<(&'static str, &'static str)> {
        None
    }
}

/// Fresh planning with a borrowed configuration local to this Operation.
pub trait Plan<Parent: ?Sized>: OperationDefinition {
    /// Known planning facts used by descendants; never a simulated future State input.
    type Config: ?Sized;
    /// Returns the local configuration and finite typed source structure.
    fn plan<'a>(&'a self, parent: &'a Parent) -> Result<(&'a Self::Config, Self::Body)>;
}

/// Inherit the enclosing policy and allowances.
pub struct Inherit;

/// Statically declared recovery handler and target types for one maintained Operation.
pub trait OperationDefaults {
    /// Handler installed when resolution supplies its parameters.
    type Handler: Handler;
    /// Statically declared checkpoint target markers.
    type Targets: CheckpointTargets;
}
/// Resolution from this Operation's local configuration.
pub trait ResolveDefaults<C: ?Sized>: OperationDefaults {
    /// Resolves independently inherited handler and allowance values.
    fn resolve(config: &C) -> Result<PolicyValues<Self::Handler>>;
}
/// Values selected by maintained defaults, with None preserving the parent scope.
pub struct PolicyValues<H: Handler> {
    /// Replaces the handler and its target unit when present.
    pub handler: Option<H::Params>,
    /// Replaces the retry allowance, including explicit zero.
    pub retries: Option<u32>,
    /// Replaces the restart allowance, including explicit zero.
    pub restarts: Option<u32>,
}
impl OperationDefaults for Inherit {
    type Handler = Stop;
    type Targets = ();
}
impl<C: ?Sized> ResolveDefaults<C> for Inherit {
    fn resolve(_: &C) -> Result<PolicyValues<Stop>> {
        Ok(PolicyValues {
            handler: None,
            retries: None,
            restarts: None,
        })
    }
}

/// A source scope with one definition and a maintained defaults type.
pub struct Operation<Definition, Defaults = Inherit> {
    pub(super) definition: Definition,
    defaults: PhantomData<fn() -> Defaults>,
}
impl<D> Operation<D, Inherit> {
    /// Constructs source values without executing or binding a State.
    pub fn new(definition: D) -> Self {
        Self::from(definition)
    }
}
impl<D, P> From<D> for Operation<D, P> {
    fn from(definition: D) -> Self {
        Self {
            definition,
            defaults: PhantomData,
        }
    }
}
impl<D: Default, P> Default for Operation<D, P> {
    fn default() -> Self {
        Self::from(D::default())
    }
}
impl<D: Clone, P> Clone for Operation<D, P> {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            defaults: PhantomData,
        }
    }
}
impl<D: OperationDefinition, P> sealed::Sealed for Operation<D, P> {}
impl<D: OperationDefinition, P> AuthoringSource for Operation<D, P> {
    type Input = <D::Body as AuthoringSource>::Input;
    type Output = <D::Body as AuthoringSource>::Output;
}

impl<T: MfmValue, B: AuthoringSource<Input = T, Output = T>> sealed::Sealed for Vec<B> {}
impl<T: MfmValue, B: AuthoringSource<Input = T, Output = T>> AuthoringSource for Vec<B> {
    type Input = T;
    type Output = T;
}

impl<C: ?Sized, R, S: PureState> Walk<C, R> for Pure<S> {
    fn walk(&self, _: &C, _: &R, draft: &mut Draft) -> Result<()> {
        draft.pure::<S>()
    }
}
impl<R, S: PureState> Discover<R> for Pure<S> {
    fn discover(inventory: &mut Inventory<R>) -> Result<()> {
        inventory.pure::<S>()
    }
}
impl<C: ?Sized, R, T: MfmValue> Walk<C, R> for Identity<T> {
    fn walk(&self, _: &C, _: &R, draft: &mut Draft) -> Result<()> {
        draft.contracts.insert::<T>().map(|_| ())
    }
}
impl<R, T: MfmValue> Discover<R> for Identity<T> {
    fn discover(inventory: &mut Inventory<R>) -> Result<()> {
        inventory.value::<T>()
    }
}
impl<C: ?Sized, R, D, P> Walk<C, R> for Operation<D, P>
where
    D: Plan<C>,
    D::Body: Walk<D::Config, R>,
    P: ResolveDefaults<D::Config>,
{
    fn walk(&self, config: &C, resources: &R, draft: &mut Draft) -> Result<()> {
        draft.nested(|draft| {
            let (local, body) = self.definition.plan(config)?;
            draft.scope::<P, _, _>(local, |draft| body.walk(local, resources, draft))
        })
    }
}
impl<R, D, P> Discover<R> for Operation<D, P>
where
    D: OperationDefinition,
    D::Body: Discover<R>,
    P: OperationDefaults,
{
    fn discover(inventory: &mut Inventory<R>) -> Result<()> {
        inventory.operation::<D>()?;
        inventory.handler::<P::Handler>()?;
        D::Body::discover(inventory)
    }
}
impl<C: ?Sized, R, T: MfmValue, B: Walk<C, R, Input = T, Output = T>> Walk<C, R> for Vec<B> {
    fn walk(&self, config: &C, resources: &R, draft: &mut Draft) -> Result<()> {
        for child in self {
            child.walk(config, resources, draft)?;
        }
        Ok(())
    }
}
impl<R, B: Discover<R>> Discover<R> for Vec<B> {
    fn discover(inventory: &mut Inventory<R>) -> Result<()> {
        B::discover(inventory)
    }
}
impl<R> Discover<R> for () {
    fn discover(_: &mut Inventory<R>) -> Result<()> {
        Ok(())
    }
}

trait ReadFamily<S, C, R> {
    fn discover(inventory: &mut Inventory<R>) -> Result<()>;
}
trait EffectFamily<S, C, R> {
    fn discover(inventory: &mut Inventory<R>) -> Result<()>;
}
trait WalkReadFamily<Config: ?Sized, S, C, R> {
    fn walk(config: &Config, resources: &R, draft: &mut Draft) -> Result<()>;
}
trait WalkEffectFamily<Config: ?Sized, S, C, R> {
    fn walk(config: &Config, resources: &R, draft: &mut Draft) -> Result<()>;
}
fn unique_implementations<'a>(
    capability: &mfm_ids::StableId,
    ids: impl IntoIterator<Item = &'a mfm_ids::StableId>,
    position: Option<usize>,
) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(crate::construction::rejection(
                "select_native",
                &serde_json::json!({
                    "reason": "duplicate_implementation", "capability": capability,
                    "implementation": id, "position": position,
                }),
            ));
        }
    }
    Ok(())
}
macro_rules! family_tuple {
    ($family:ident, $walk:ident, $capability:ident, $selection:ident, $implementation:ident,
     $resolve:ident, $resolved:ident; $($entry:ident),+) => {
        impl<SelectedState, Capability, Resources, $($entry,)+>
            $family<SelectedState, Capability, Resources> for ($($entry,)+)
        where Capability: mfm_capabilities::$capability, SelectedState: $selection<Capability>,
              $($entry: mfm_capabilities::$implementation<Capability>,
                $resolved<SelectedState, Capability, $entry>: Discover<Resources>,)+
        {
            fn discover(inventory: &mut Inventory<Resources>) -> Result<()> {
                unique_implementations(&Capability::contract_id()?, &[$($entry::implementation_id()?,)+], None)?;
                $(<$resolved<SelectedState, Capability, $entry> as Discover<Resources>>::discover(inventory)?;)+
                Ok(())
            }
        }
        impl<Config: ?Sized, SelectedState, Capability, Resources, $($entry,)+>
            $walk<Config, SelectedState, Capability, Resources> for ($($entry,)+)
        where Capability: mfm_capabilities::$capability, SelectedState: $selection<Capability>,
              Resources: crate::Resolve<Config, Capability>,
              $($entry: crate::$resolve<Config, Capability>,
                $resolved<SelectedState, Capability, $entry>: Walk<Config, Resources>,)+
        {
            fn walk(config: &Config, resources: &Resources, draft: &mut Draft) -> Result<()> {
                #[allow(non_snake_case)]
                let ($($entry,)+) = ($($entry::implementation_id()?,)+);
                unique_implementations(&Capability::contract_id()?, [$(&$entry,)+], Some(draft.position()))?;
                let selected = <Resources as crate::Resolve<Config, Capability>>::implementation(config)?;
                $(if selected == $entry {
                    let source = $resolved::<SelectedState, Capability, $entry>::new(
                        <$entry as crate::$resolve<Config, Capability>>::binding(config)?,
                    );
                    return source.walk(config, resources, draft);
                })+
                Err(crate::construction::rejection("select_native", &serde_json::json!({
                    "reason": "unsupported_implementation", "capability": Capability::contract_id()?,
                    "selected": selected, "available": [$($entry,)+], "position": draft.position(),
                })))
            }
        }
    };
}
macro_rules! tuple {
    ($first:ident $(, $previous:ident => $next:ident)* ; $last:ident) => {
        family_tuple!(ReadFamily, WalkReadFamily, ReadCapabilityContract, ReadSelection,
            ReadImplementation, ResolveReadBinding, ResolvedRead; $first $(, $next)*);
        family_tuple!(EffectFamily, WalkEffectFamily, EffectCapabilityContract, EffectSelection,
            EffectImplementation, ResolveEffectBinding, ResolvedEffect; $first $(, $next)*);
        impl<$first, $($next,)*> sealed::Sealed for ($first, $($next,)*)
        where
            $first: AuthoringSource,
            $($next: AuthoringSource<Input = <$previous as AuthoringSource>::Output>,)*
        {}
        impl<$first, $($next,)*> AuthoringSource for ($first, $($next,)*)
        where
            $first: AuthoringSource,
            $($next: AuthoringSource<Input = <$previous as AuthoringSource>::Output>,)*
        {
            type Input = $first::Input;
            type Output = $last::Output;
        }
        impl<Config: ?Sized, Resources, $first, $($next,)*> Walk<Config, Resources> for ($first, $($next,)*)
        where
            $first: Walk<Config, Resources>,
            $($next: Walk<Config, Resources, Input = <$previous as AuthoringSource>::Output>,)*
        {
            fn walk(&self, config: &Config, resources: &Resources, draft: &mut Draft) -> Result<()> {
                #[allow(non_snake_case)]
                let ($first, $($next,)*) = self;
                $first.walk(config, resources, draft)?;
                $($next.walk(config, resources, draft)?;)*
                Ok(())
            }
        }
        impl<Resources, $first, $($next,)*> Discover<Resources> for ($first, $($next,)*)
        where $first: Discover<Resources>, $($next: Discover<Resources>,)*
        {
            fn discover(inventory: &mut Inventory<Resources>) -> Result<()> {
                $first::discover(inventory)?;
                $($next::discover(inventory)?;)*
                Ok(())
            }
        }
        impl<$first, $($next,)*> sealed::Targets for ($first, $($next,)*)
        where $first: CheckpointMarker, $($next: CheckpointMarker,)*
        {
            fn targets() -> Result<Vec<(std::any::TypeId, mfm_ids::ContentRef)>> {
                Ok(vec![(std::any::TypeId::of::<$first>(), crate::nominal_contract_ref::<$first::Context>()?),
                    $((std::any::TypeId::of::<$next>(), crate::nominal_contract_ref::<$next::Context>()?),)*])
            }
        }
        impl<$first, $($next,)*> CheckpointTargets for ($first, $($next,)*)
        where $first: CheckpointMarker, $($next: CheckpointMarker,)* {}
        impl<$first, $($next,)*> OperationDefinition for ($first, $($next,)*)
        where
            Self: AuthoringSource,
        {
            type Body = Self;
        }
        impl<Parent: ?Sized, $first, $($next,)*> Plan<Parent> for ($first, $($next,)*)
        where
            Self: AuthoringSource + Clone,
        {
            type Config = Parent;
            fn plan<'a>(&'a self, parent: &'a Parent) -> Result<(&'a Parent, Self)> {
                Ok((parent, self.clone()))
            }
        }
    };
}

tuple!(A; A);
tuple!(A, A => B; B);
tuple!(A, A => B, B => C; C);
tuple!(A, A => B, B => C, C => D; D);
tuple!(A, A => B, B => C, C => D, D => E; E);
tuple!(A, A => B, B => C, C => D, D => E, E => F; F);
tuple!(A, A => B, B => C, C => D, D => E, E => F, F => G; G);
tuple!(A, A => B, B => C, C => D, D => E, E => F, F => G, G => H; H);
tuple!(A, A => B, B => C, C => D, D => E, E => F, F => G, G => H, H => I; I);
tuple!(A, A => B, B => C, C => D, D => E, E => F, F => G, G => H, H => I, I => J; J);
tuple!(A, A => B, B => C, C => D, D => E, E => F, F => G, G => H, H => I, I => J, J => K; K);
tuple!(A, A => B, B => C, C => D, D => E, E => F, F => G, G => H, H => I, I => J, J => K, K => L; L);

pub(crate) fn targets<T: CheckpointTargets>() -> Result<Vec<(std::any::TypeId, mfm_ids::ContentRef)>>
{
    <T as sealed::Targets>::targets()
}

macro_rules! resolved {
    ($name:ident, $selection:ident, $capability:ident, $implementation:ident, $inject:ident, $bind:ident, $emit:ident) => {
        /// Implementation-owned supporting selection carrying its already-selected public binding.
        pub struct $name<S, C, I>
        where
            C: mfm_capabilities::$capability,
            I: mfm_capabilities::$implementation<C>,
        {
            binding: I::Binding,
            marker: PhantomData<fn() -> (S, C)>,
        }
        impl<S, C, I> $name<S, C, I>
        where
            C: mfm_capabilities::$capability,
            I: mfm_capabilities::$implementation<C>,
        {
            /// Supplies an exact binding without querying parent configuration.
            pub fn new(binding: I::Binding) -> Self {
                Self {
                    binding,
                    marker: PhantomData,
                }
            }
        }
        impl<S, C, I> sealed::Sealed for $name<S, C, I>
        where
            S: $selection<C>,
            C: mfm_capabilities::$capability,
            I: mfm_capabilities::$implementation<C>,
        {
        }
        impl<S, C, I> AuthoringSource for $name<S, C, I>
        where
            S: $selection<C>,
            C: mfm_capabilities::$capability,
            I: mfm_capabilities::$implementation<C>,
        {
            type Input = S::ExpandedInput;
            type Output = S::ExpandedOutput;
        }
        impl<Config: ?Sized, R, S, C, I> Walk<Config, R> for $name<S, C, I>
        where
            S: $selection<C>,
            C: mfm_capabilities::$capability,
            I: crate::$inject<S, C>,
            I::OperationalError: crate::ClassifyError,
            I::Prefix: Walk<Config, R>,
            I::Suffix: Walk<Config, R>,
            R: crate::$bind<C, I>,
        {
            fn walk(&self, config: &Config, resources: &R, draft: &mut Draft) -> Result<()> {
                draft.nested(|draft| {
                    let (prefix, suffix) = I::surround(&self.binding)?;
                    prefix.walk(config, resources, draft)?;
                    draft.$emit::<S, C, I, R>(&self.binding, resources)?;
                    suffix.walk(config, resources, draft)
                })
            }
        }
        impl<R, S, C, I> Discover<R> for $name<S, C, I>
        where
            S: $selection<C>,
            C: mfm_capabilities::$capability,
            I: crate::$inject<S, C>,
            I::OperationalError: crate::ClassifyError,
            I::Prefix: Discover<R>,
            I::Suffix: Discover<R>,
            R: crate::$bind<C, I>,
        {
            fn discover(inventory: &mut Inventory<R>) -> Result<()> {
                I::Prefix::discover(inventory)?;
                inventory.$emit::<S, C, I>()?;
                I::Suffix::discover(inventory)
            }
        }
    };
}
resolved!(
    ResolvedRead,
    ReadSelection,
    ReadCapabilityContract,
    ReadImplementation,
    InjectRead,
    BindRead,
    read
);
resolved!(
    ResolvedEffect,
    EffectSelection,
    EffectCapabilityContract,
    EffectImplementation,
    InjectEffect,
    BindEffect,
    effect
);

macro_rules! unresolved {
    ($source:ident, $selection:ident, $capability:ident, $family:ident, $walk:ident) => {
        impl<Config: ?Sized, R, S, C> Walk<Config, R> for $source<S, C>
        where
            C: mfm_capabilities::$capability,
            S: $selection<C>,
            R: crate::Resolve<Config, C>,
            R::Implementations: $walk<Config, S, C, R>,
        {
            fn walk(&self, config: &Config, resources: &R, draft: &mut Draft) -> Result<()> {
                <R::Implementations as $walk<Config, S, C, R>>::walk(config, resources, draft)
            }
        }
        impl<R, S, C> Discover<R> for $source<S, C>
        where
            C: mfm_capabilities::$capability,
            S: $selection<C>,
            R: crate::CapabilityFamily<C>,
            R::Implementations: $family<S, C, R>,
        {
            fn discover(inventory: &mut Inventory<R>) -> Result<()> {
                <R::Implementations as $family<S, C, R>>::discover(inventory)
            }
        }
    };
}
unresolved!(
    Read,
    ReadSelection,
    ReadCapabilityContract,
    ReadFamily,
    WalkReadFamily
);
unresolved!(
    Effect,
    EffectSelection,
    EffectCapabilityContract,
    EffectFamily,
    WalkEffectFamily
);
