#![warn(missing_docs)]
//! Capability contracts for the MFM typed kernel.
//!
//! Capability descriptors are extensible: domain crates may implement
//! [`CapabilitySpec`] for their own token types. Capability-set evidence is
//! framework-owned and sealed: domain crates compose capabilities with the
//! closed tuple implementations in this crate, but cannot implement
//! [`CapabilitySetFor`] manually to widen an effect's authority.

use std::collections::BTreeSet;

pub use mfm_effects::{
    ApplySideEffect, EffectClass, EffectSpec, ManagedPlatformWrite, Pure, ReadExternal,
};
use mfm_ids::NameToken;
pub use mfm_ids::{CapabilityKind, CapabilityVersion};
pub use provider_diagnostic::{
    ProviderDiagnosticCode, ProviderDiagnosticValue, RedactedProviderDiagnostic,
};

mod provider_diagnostic;

#[cfg(test)]
mod tests;

/// Result type for capability descriptor helpers.
pub type Result<T> = std::result::Result<T, CapabilityError>;

/// Error returned by capability descriptor and role validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    /// Capability descriptor construction failed.
    #[error("capability descriptor error: {0}")]
    Descriptor(String),
    /// Capability identity or version construction failed.
    #[error("capability identity error: {0}")]
    Identity(String),
    /// A capability set contains the same capability kind/version more than once.
    #[error("duplicate capability descriptor for kind {kind} version {version}")]
    DuplicateCapability {
        /// Duplicate capability kind.
        kind: String,
        /// Duplicate capability version.
        version: String,
    },
    /// A descriptor set violates the v1 effect-role rules.
    #[error("invalid capability set for effect {effect}: {message}")]
    InvalidCapabilitySet {
        /// Effect class being validated.
        effect: &'static str,
        /// Stable diagnostic.
        message: String,
    },
}

/// Capability role marker contract.
pub trait CapabilityRoleSpec: private::RoleSealed + Send + Sync + 'static {
    /// Stable role enum carried by persisted descriptors.
    const ROLE: CapabilityRole;
}

/// External read capability role.
pub enum ReadExternalRole {}

/// MFM-managed platform persistence/output capability role.
pub enum ManagedPlatformWriteRole {}

/// Support capability role that cannot mutate external systems.
pub enum SupportRole {}

/// External mutation authority role.
pub enum ExternalMutationAuthorityRole {}

macro_rules! impl_role_spec {
    ($ty:ty, $role:expr) => {
        impl private::RoleSealed for $ty {}

        impl CapabilityRoleSpec for $ty {
            const ROLE: CapabilityRole = $role;
        }
    };
}

impl_role_spec!(ReadExternalRole, CapabilityRole::ReadExternal);
impl_role_spec!(
    ManagedPlatformWriteRole,
    CapabilityRole::ManagedPlatformWrite
);
impl_role_spec!(SupportRole, CapabilityRole::Support);
impl_role_spec!(
    ExternalMutationAuthorityRole,
    CapabilityRole::ExternalMutationAuthority
);

/// Stable capability role persisted in descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CapabilityRole {
    /// Read-only access to an external system.
    ReadExternal,
    /// MFM-managed platform persistence/output authority.
    ManagedPlatformWrite,
    /// Non-mutating support authority.
    Support,
    /// Authority to mutate one external domain system.
    ExternalMutationAuthority,
}

impl CapabilityRole {
    /// Returns the stable descriptor string for this capability role.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadExternal => "read_external",
            Self::ManagedPlatformWrite => "managed_platform_write",
            Self::Support => "support",
            Self::ExternalMutationAuthority => "external_mutation_authority",
        }
    }
}

/// Extensible descriptor contract for a capability token type.
pub trait CapabilitySpec: Send + Sync + 'static {
    /// Framework role marker for this capability.
    type Role: CapabilityRoleSpec;

    /// Returns the stable capability kind id.
    fn kind() -> Result<CapabilityKind>;

    /// Returns the capability descriptor version.
    fn version() -> Result<CapabilityVersion>;

    /// Returns the stable capability name.
    fn name() -> &'static str;

    /// Returns the stable capability descriptor.
    fn descriptor() -> Result<CapabilityDescriptor> {
        CapabilityDescriptor::new(
            Self::kind()?,
            Self::version()?,
            <Self::Role as CapabilityRoleSpec>::ROLE,
            Self::name(),
        )
    }
}

/// Stable capability descriptor used by state descriptors and certification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    /// Stable capability kind id.
    pub kind: CapabilityKind,
    /// Capability descriptor contract version.
    pub version: CapabilityVersion,
    /// Capability role.
    pub role: CapabilityRole,
    /// Stable capability name.
    pub name: NameToken,
}

impl CapabilityDescriptor {
    /// Creates a checked capability descriptor.
    pub fn new(
        kind: CapabilityKind,
        version: CapabilityVersion,
        role: CapabilityRole,
        name: impl Into<String>,
    ) -> Result<Self> {
        let raw_name = name.into();
        let name = NameToken::new(&raw_name).map_err(|_| {
            CapabilityError::Descriptor(format!("invalid capability descriptor name {raw_name:?}"))
        })?;

        Ok(Self {
            kind,
            version,
            role,
            name,
        })
    }
}

/// No-capability set for pure states.
pub enum NoCaps {}

/// Sealed descriptor contract for framework-owned capability-set shapes.
pub trait CapabilitySet: private::CapabilitySetSealed + Send + Sync + 'static {
    /// Returns the capability-set descriptor.
    fn descriptor() -> Result<CapabilitySetDescriptor>;
}

/// Framework-owned evidence that a capability set is valid for an effect.
pub trait CapabilitySetFor<E: EffectSpec>:
    CapabilitySet + private::CapabilitySetForSealed<E>
{
}

impl<E, C> CapabilitySetFor<E> for C
where
    E: EffectSpec,
    C: CapabilitySet + private::CapabilitySetForSealed<E>,
{
}

/// Stable descriptor for a capability set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySetDescriptor {
    /// Capabilities in the authored set order.
    pub capabilities: Vec<CapabilityDescriptor>,
}

impl CapabilitySetDescriptor {
    /// Creates a descriptor and rejects duplicate capability kind/version pairs.
    pub fn new(capabilities: Vec<CapabilityDescriptor>) -> Result<Self> {
        let mut seen = BTreeSet::new();
        for capability in &capabilities {
            let key = (
                capability.kind.as_str().to_owned(),
                capability.version.as_str().to_owned(),
            );
            if !seen.insert(key) {
                return Err(CapabilityError::DuplicateCapability {
                    kind: capability.kind.as_str().to_owned(),
                    version: capability.version.as_str().to_owned(),
                });
            }
        }
        Ok(Self { capabilities })
    }

    /// Returns true when this descriptor contains no capabilities.
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Returns the number of capability descriptors.
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Validates this persisted descriptor against the v1 role rules for an effect.
    pub fn validate_for_effect<E: EffectSpec>(&self) -> Result<()> {
        self.validate_for_effect_class(E::class(), E::name())
    }

    /// Validates this descriptor against an effect class.
    pub fn validate_for_effect_class(
        &self,
        effect: EffectClass,
        effect_name: &'static str,
    ) -> Result<()> {
        match effect {
            EffectClass::Pure => {
                if self.capabilities.is_empty() {
                    Ok(())
                } else {
                    Err(invalid_set(
                        effect_name,
                        "pure effects must declare NoCaps only",
                    ))
                }
            }
            EffectClass::ReadExternal => {
                if self.capabilities.iter().all(|capability| {
                    matches!(
                        capability.role,
                        CapabilityRole::ReadExternal | CapabilityRole::Support
                    )
                }) {
                    Ok(())
                } else {
                    Err(invalid_set(
                        effect_name,
                        "read effects may declare only read or support capabilities",
                    ))
                }
            }
            EffectClass::ManagedPlatformWrite => {
                if self
                    .capabilities
                    .iter()
                    .all(|capability| capability.role == CapabilityRole::ManagedPlatformWrite)
                {
                    Ok(())
                } else {
                    Err(invalid_set(
                        effect_name,
                        "managed platform write effects may declare only managed platform capabilities",
                    ))
                }
            }
            EffectClass::ApplySideEffect => {
                let mutation_authorities = self
                    .capabilities
                    .iter()
                    .filter(|capability| {
                        capability.role == CapabilityRole::ExternalMutationAuthority
                    })
                    .count();
                let only_allowed_aux = self.capabilities.iter().all(|capability| {
                    matches!(
                        capability.role,
                        CapabilityRole::ExternalMutationAuthority
                            | CapabilityRole::ReadExternal
                            | CapabilityRole::Support
                    )
                });

                if mutation_authorities == 1 && only_allowed_aux {
                    Ok(())
                } else {
                    Err(invalid_set(
                        effect_name,
                        "side effects must declare exactly one mutation authority plus optional read/support capabilities",
                    ))
                }
            }
        }
    }
}

impl private::CapabilitySetSealed for NoCaps {}

impl CapabilitySet for NoCaps {
    fn descriptor() -> Result<CapabilitySetDescriptor> {
        CapabilitySetDescriptor::new(Vec::new())
    }
}

impl private::CapabilitySetForSealed<Pure> for NoCaps {}

macro_rules! impl_capability_tuple {
    ($($name:ident),+) => {
        impl<$($name),+> private::CapabilitySetSealed for ($($name,)+)
        where
            $($name: CapabilitySpec),+
        {
        }

        impl<$($name),+> CapabilitySet for ($($name,)+)
        where
            $($name: CapabilitySpec),+
        {
            fn descriptor() -> Result<CapabilitySetDescriptor> {
                CapabilitySetDescriptor::new(vec![$($name::descriptor()?),+])
            }
        }
    };
}

macro_rules! impl_all_roles_for_effect {
    ($effect:ty, $allowed:path, $($name:ident),+) => {
        impl<$($name),+> private::CapabilitySetForSealed<$effect> for ($($name,)+)
        where
            $(
                $name: CapabilitySpec,
                <$name as CapabilitySpec>::Role: $allowed,
            )+
        {
        }
    };
}

macro_rules! impl_capability_tuple_family {
    ($($name:ident),+) => {
        impl_capability_tuple!($($name),+);
        impl_all_roles_for_effect!(ReadExternal, private::ReadEffectRole, $($name),+);
        impl_all_roles_for_effect!(
            ManagedPlatformWrite,
            private::ManagedPlatformWriteEffectRole,
            $($name),+
        );
        impl<$($name),+> private::CapabilitySetForSealed<ApplySideEffect> for ($($name,)+)
        where
            $(
                $name: CapabilitySpec,
                <$name as CapabilitySpec>::Role: private::RoleMutationAuthorityCount,
            )+
            ($(<$name as CapabilitySpec>::Role,)+): private::ExactlyOneMutationAuthority,
        {
        }
    };
}

impl_capability_tuple_family!(A);
impl_capability_tuple_family!(A, B);
impl_capability_tuple_family!(A, B, C);
impl_capability_tuple_family!(A, B, C, D);
impl_capability_tuple_family!(A, B, C, D, E);
impl_capability_tuple_family!(A, B, C, D, E, F);
impl_capability_tuple_family!(A, B, C, D, E, F, G);
impl_capability_tuple_family!(A, B, C, D, E, F, G, H);

fn invalid_set(effect: &'static str, message: &'static str) -> CapabilityError {
    CapabilityError::InvalidCapabilitySet {
        effect,
        message: message.to_owned(),
    }
}

mod private {
    use super::{
        CapabilityRoleSpec, ExternalMutationAuthorityRole, ManagedPlatformWriteRole,
        ReadExternalRole, SupportRole,
    };
    use mfm_effects::EffectSpec;

    pub trait RoleSealed {}

    pub trait CapabilitySetSealed {}

    pub trait CapabilitySetForSealed<E: EffectSpec> {}

    pub trait ReadEffectRole {}

    impl ReadEffectRole for ReadExternalRole {}

    impl ReadEffectRole for SupportRole {}

    pub trait ManagedPlatformWriteEffectRole {}

    impl ManagedPlatformWriteEffectRole for ManagedPlatformWriteRole {}

    pub enum ZeroMutationAuthorities {}

    pub enum OneMutationAuthority {}

    pub enum InvalidMutationAuthorities {}

    pub trait MutationAuthorityCount {}

    impl MutationAuthorityCount for ZeroMutationAuthorities {}

    impl MutationAuthorityCount for OneMutationAuthority {}

    impl MutationAuthorityCount for InvalidMutationAuthorities {}

    pub trait AddMutationAuthorityCount<Rhs: MutationAuthorityCount>:
        MutationAuthorityCount
    {
        type Output: MutationAuthorityCount;
    }

    impl AddMutationAuthorityCount<ZeroMutationAuthorities> for ZeroMutationAuthorities {
        type Output = ZeroMutationAuthorities;
    }

    impl AddMutationAuthorityCount<OneMutationAuthority> for ZeroMutationAuthorities {
        type Output = OneMutationAuthority;
    }

    impl AddMutationAuthorityCount<InvalidMutationAuthorities> for ZeroMutationAuthorities {
        type Output = InvalidMutationAuthorities;
    }

    impl AddMutationAuthorityCount<ZeroMutationAuthorities> for OneMutationAuthority {
        type Output = OneMutationAuthority;
    }

    impl AddMutationAuthorityCount<OneMutationAuthority> for OneMutationAuthority {
        type Output = InvalidMutationAuthorities;
    }

    impl AddMutationAuthorityCount<InvalidMutationAuthorities> for OneMutationAuthority {
        type Output = InvalidMutationAuthorities;
    }

    impl AddMutationAuthorityCount<ZeroMutationAuthorities> for InvalidMutationAuthorities {
        type Output = InvalidMutationAuthorities;
    }

    impl AddMutationAuthorityCount<OneMutationAuthority> for InvalidMutationAuthorities {
        type Output = InvalidMutationAuthorities;
    }

    impl AddMutationAuthorityCount<InvalidMutationAuthorities> for InvalidMutationAuthorities {
        type Output = InvalidMutationAuthorities;
    }

    pub trait RoleMutationAuthorityCount {
        type Count: MutationAuthorityCount;
    }

    impl RoleMutationAuthorityCount for ReadExternalRole {
        type Count = ZeroMutationAuthorities;
    }

    impl RoleMutationAuthorityCount for SupportRole {
        type Count = ZeroMutationAuthorities;
    }

    impl RoleMutationAuthorityCount for ExternalMutationAuthorityRole {
        type Count = OneMutationAuthority;
    }

    impl RoleMutationAuthorityCount for ManagedPlatformWriteRole {
        type Count = InvalidMutationAuthorities;
    }

    pub trait ExactlyOneMutationAuthorityCount {}

    impl ExactlyOneMutationAuthorityCount for OneMutationAuthority {}

    pub trait TupleMutationAuthorityCount {
        type Count: MutationAuthorityCount;
    }

    pub trait ExactlyOneMutationAuthorityTuple {}

    impl<T> ExactlyOneMutationAuthorityTuple for T
    where
        T: TupleMutationAuthorityCount,
        T::Count: ExactlyOneMutationAuthorityCount,
    {
    }

    pub trait ExactlyOneMutationAuthority: ExactlyOneMutationAuthorityTuple {}

    impl<T> ExactlyOneMutationAuthority for T where T: ExactlyOneMutationAuthorityTuple {}

    macro_rules! count2 {
        ($a:ty, $b:ty) => {
            <<$a as RoleMutationAuthorityCount>::Count as AddMutationAuthorityCount<
                <$b as RoleMutationAuthorityCount>::Count,
            >>::Output
        };
    }

    macro_rules! count3 {
        ($a:ty, $b:ty, $c:ty) => {
            <count2!($a, $b) as AddMutationAuthorityCount<
                <$c as RoleMutationAuthorityCount>::Count,
            >>::Output
        };
    }

    macro_rules! count4 {
        ($a:ty, $b:ty, $c:ty, $d:ty) => {
            <count3!($a, $b, $c) as AddMutationAuthorityCount<
                <$d as RoleMutationAuthorityCount>::Count,
            >>::Output
        };
    }

    macro_rules! count5 {
        ($a:ty, $b:ty, $c:ty, $d:ty, $e:ty) => {
            <count4!($a, $b, $c, $d) as AddMutationAuthorityCount<
                <$e as RoleMutationAuthorityCount>::Count,
            >>::Output
        };
    }

    macro_rules! count6 {
        ($a:ty, $b:ty, $c:ty, $d:ty, $e:ty, $f:ty) => {
            <count5!($a, $b, $c, $d, $e) as AddMutationAuthorityCount<
                <$f as RoleMutationAuthorityCount>::Count,
            >>::Output
        };
    }

    macro_rules! count7 {
        ($a:ty, $b:ty, $c:ty, $d:ty, $e:ty, $f:ty, $g:ty) => {
            <count6!($a, $b, $c, $d, $e, $f) as AddMutationAuthorityCount<
                <$g as RoleMutationAuthorityCount>::Count,
            >>::Output
        };
    }

    macro_rules! count8 {
        ($a:ty, $b:ty, $c:ty, $d:ty, $e:ty, $f:ty, $g:ty, $h:ty) => {
            <count7!($a, $b, $c, $d, $e, $f, $g) as AddMutationAuthorityCount<
                <$h as RoleMutationAuthorityCount>::Count,
            >>::Output
        };
    }

    impl<A> TupleMutationAuthorityCount for (A,)
    where
        A: RoleMutationAuthorityCount,
    {
        type Count = A::Count;
    }

    impl<A, B> TupleMutationAuthorityCount for (A, B)
    where
        A: RoleMutationAuthorityCount,
        B: RoleMutationAuthorityCount,
        A::Count: AddMutationAuthorityCount<B::Count>,
    {
        type Count = count2!(A, B);
    }

    impl<A, B, C> TupleMutationAuthorityCount for (A, B, C)
    where
        A: RoleMutationAuthorityCount,
        B: RoleMutationAuthorityCount,
        C: RoleMutationAuthorityCount,
        A::Count: AddMutationAuthorityCount<B::Count>,
        count2!(A, B): AddMutationAuthorityCount<C::Count>,
    {
        type Count = count3!(A, B, C);
    }

    impl<A, B, C, D> TupleMutationAuthorityCount for (A, B, C, D)
    where
        A: RoleMutationAuthorityCount,
        B: RoleMutationAuthorityCount,
        C: RoleMutationAuthorityCount,
        D: RoleMutationAuthorityCount,
        A::Count: AddMutationAuthorityCount<B::Count>,
        count2!(A, B): AddMutationAuthorityCount<C::Count>,
        count3!(A, B, C): AddMutationAuthorityCount<D::Count>,
    {
        type Count = count4!(A, B, C, D);
    }

    impl<A, B, C, D, E> TupleMutationAuthorityCount for (A, B, C, D, E)
    where
        A: RoleMutationAuthorityCount,
        B: RoleMutationAuthorityCount,
        C: RoleMutationAuthorityCount,
        D: RoleMutationAuthorityCount,
        E: RoleMutationAuthorityCount,
        A::Count: AddMutationAuthorityCount<B::Count>,
        count2!(A, B): AddMutationAuthorityCount<C::Count>,
        count3!(A, B, C): AddMutationAuthorityCount<D::Count>,
        count4!(A, B, C, D): AddMutationAuthorityCount<E::Count>,
    {
        type Count = count5!(A, B, C, D, E);
    }

    impl<A, B, C, D, E, F> TupleMutationAuthorityCount for (A, B, C, D, E, F)
    where
        A: RoleMutationAuthorityCount,
        B: RoleMutationAuthorityCount,
        C: RoleMutationAuthorityCount,
        D: RoleMutationAuthorityCount,
        E: RoleMutationAuthorityCount,
        F: RoleMutationAuthorityCount,
        A::Count: AddMutationAuthorityCount<B::Count>,
        count2!(A, B): AddMutationAuthorityCount<C::Count>,
        count3!(A, B, C): AddMutationAuthorityCount<D::Count>,
        count4!(A, B, C, D): AddMutationAuthorityCount<E::Count>,
        count5!(A, B, C, D, E): AddMutationAuthorityCount<F::Count>,
    {
        type Count = count6!(A, B, C, D, E, F);
    }

    impl<A, B, C, D, E, F, G> TupleMutationAuthorityCount for (A, B, C, D, E, F, G)
    where
        A: RoleMutationAuthorityCount,
        B: RoleMutationAuthorityCount,
        C: RoleMutationAuthorityCount,
        D: RoleMutationAuthorityCount,
        E: RoleMutationAuthorityCount,
        F: RoleMutationAuthorityCount,
        G: RoleMutationAuthorityCount,
        A::Count: AddMutationAuthorityCount<B::Count>,
        count2!(A, B): AddMutationAuthorityCount<C::Count>,
        count3!(A, B, C): AddMutationAuthorityCount<D::Count>,
        count4!(A, B, C, D): AddMutationAuthorityCount<E::Count>,
        count5!(A, B, C, D, E): AddMutationAuthorityCount<F::Count>,
        count6!(A, B, C, D, E, F): AddMutationAuthorityCount<G::Count>,
    {
        type Count = count7!(A, B, C, D, E, F, G);
    }

    impl<A, B, C, D, E, F, G, H> TupleMutationAuthorityCount for (A, B, C, D, E, F, G, H)
    where
        A: RoleMutationAuthorityCount,
        B: RoleMutationAuthorityCount,
        C: RoleMutationAuthorityCount,
        D: RoleMutationAuthorityCount,
        E: RoleMutationAuthorityCount,
        F: RoleMutationAuthorityCount,
        G: RoleMutationAuthorityCount,
        H: RoleMutationAuthorityCount,
        A::Count: AddMutationAuthorityCount<B::Count>,
        count2!(A, B): AddMutationAuthorityCount<C::Count>,
        count3!(A, B, C): AddMutationAuthorityCount<D::Count>,
        count4!(A, B, C, D): AddMutationAuthorityCount<E::Count>,
        count5!(A, B, C, D, E): AddMutationAuthorityCount<F::Count>,
        count6!(A, B, C, D, E, F): AddMutationAuthorityCount<G::Count>,
        count7!(A, B, C, D, E, F, G): AddMutationAuthorityCount<H::Count>,
    {
        type Count = count8!(A, B, C, D, E, F, G, H);
    }

    #[allow(dead_code)]
    fn _role_spec_is_closed<R: CapabilityRoleSpec>() {}
}
