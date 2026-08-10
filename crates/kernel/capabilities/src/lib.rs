#![warn(missing_docs)]
//! Effect and capability contracts for the MFM typed kernel.
//!
//! Effects classify the executable behavior a typed state may use. Their
//! framework-owned marker types are sealed: downstream crates may name them
//! in `StateSpec::Effect`, but cannot add effect classes or implement
//! [`EffectSpec`] manually.
//!
//! Capability descriptors are extensible: domain crates may implement
//! [`CapabilitySpec`] for their own token types. Capability-set evidence is
//! framework-owned and sealed: domain crates compose capabilities with the
//! closed tuple implementations in this crate, but cannot implement
//! [`CapabilitySetFor`] manually to widen an effect's authority.
//!
//! ```compile_fail
//! struct CustomEffect;
//!
//! impl mfm_capabilities::EffectSpec for CustomEffect {
//!     fn kind() -> std::result::Result<mfm_ids::EffectKind, mfm_capabilities::EffectError> {
//!         unimplemented!()
//!     }
//!     fn class() -> mfm_capabilities::EffectClass { unimplemented!() }
//!     fn name() -> &'static str { "custom" }
//! }
//! ```

use std::collections::BTreeSet;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
pub use mfm_ids::{CapabilityKind, CapabilityVersion};
use mfm_ids::{DigestAlgorithm, EffectKind, EffectVersion, NameToken};
use mfm_values::MfmValue;
pub use provider_diagnostic::{
    ProviderDiagnosticCode, ProviderDiagnosticValue, RedactedProviderDiagnostic,
};

mod provider_diagnostic;

#[cfg(test)]
mod tests;

/// Result type for capability descriptor helpers.
pub type Result<T> = std::result::Result<T, CapabilityError>;

/// Boxed process-local future returned by one qualified component invoker.
pub type ComponentFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Stable redaction-safe capability contract fault.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("capability contract fault: {code}")]
pub struct CapabilityContractFault {
    code: mfm_ids::StableId,
}

impl CapabilityContractFault {
    /// Constructs a reviewed stable fault code.
    pub const fn new(code: mfm_ids::StableId) -> Self {
        Self { code }
    }

    /// Returns the reviewed stable fault code.
    pub const fn code(&self) -> &mfm_ids::StableId {
        &self.code
    }
}

/// Kernel-owned uninhabited value used where a process contract has no
/// constructible evidence variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum NoRefreshEvidence {}

impl<'de> serde::Deserialize<'de> for NoRefreshEvidence {
    fn deserialize<D>(_deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "the kernel Never type has no canonical value",
        ))
    }
}

/// Stable redaction-safe access fault code returned by a qualified adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessFaultCode {
    code: mfm_ids::StableId,
}

impl AccessFaultCode {
    /// Constructs one reviewed stable access-fault code.
    pub const fn new(code: mfm_ids::StableId) -> Self {
        Self { code }
    }

    /// Returns the reviewed stable integrity code.
    pub const fn code(&self) -> &mfm_ids::StableId {
        &self.code
    }
}

/// Typed application-protocol contract for one external Read capability.
pub trait ReadCapabilityContract: Send + Sync + 'static {
    /// Immutable typed request authored by a state callback.
    type Request: MfmValue;
    /// Schema-valid typed returned value.
    type Returned: MfmValue;
    /// Reviewed redaction-safe definite failure visible to state settlement.
    type SafeFailure: MfmValue;
}

/// Sealed Effect supersession-evidence mode.
pub trait EffectRefreshMode: private::EffectRefreshModeSealed + Send + Sync + 'static {
    /// Exact evidence carried when entry was proved not to have happened.
    type Evidence: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static;
}

/// An Effect whose entry authority cannot be refreshed.
pub enum NoRefresh {}

impl private::EffectRefreshModeSealed for NoRefresh {}

impl EffectRefreshMode for NoRefresh {
    type Evidence = NoRefreshEvidence;
}

/// An Effect whose resource contract can prove supersession before entry.
pub struct Refreshable<E>(PhantomData<fn() -> E>);

impl<E> private::EffectRefreshModeSealed for Refreshable<E> where E: MfmValue {}

impl<E> EffectRefreshMode for Refreshable<E>
where
    E: MfmValue,
{
    type Evidence = E;
}

/// A request that carries the exact value the external system keys this effect on.
///
/// Two properties of this signature are load-bearing and must not be relaxed:
///
/// - `&self` is the only input, so no ambient IO and no process state can enter
///   the derivation. The key is a pure projection of committed request bytes and
///   outlives the process that authored them.
/// - It is total. A request cannot carry `key: Option<K>` and implement this
///   honestly, so declaring absorption forces the field to be mandatory in the
///   request shape. Making it fallible would let a capability declare absorption
///   it cannot always name, and strand at runtime — which is the autoincrement
///   case, and it must fail to compile rather than park.
pub trait EntryKeyed: MfmValue {
    /// Exact value the external system keys this effect on.
    type EntryKey: MfmValue + Eq;

    /// Projects the exact retained entry key of this request.
    fn entry_key(&self) -> Self::EntryKey;
}

/// Sealed declaration of whether a parked attempt may be re-entered.
pub trait EffectEntryMode: private::EffectEntryModeSealed + Send + Sync + 'static {
    /// Maximum invocations of one occurrence, including the first.
    const MAX_ENTRIES: u16;
}

/// A repeat is not absorbed. A parked attempt is terminal.
///
/// This is not a legacy mode. It is the honest current declaration for a
/// capability whose external system does not absorb a repeat — no key exists, or
/// the response is once-only — and it is where the residual recovery gap lives
/// by design.
pub enum EntryOnce {}

impl private::EffectEntryModeSealed for EntryOnce {}

impl EffectEntryMode for EntryOnce {
    const MAX_ENTRIES: u16 = 1;
}

/// The external system absorbs a repeat of the byte-identical committed request.
///
/// Naming this asserts four things the kernel cannot verify: that the external
/// system absorbs a repeat, that the adapter transmits the entry key, that
/// absorption is retained long enough, and that `Returned` is a function of the
/// external system's post-state rather than of one exchange.
pub struct EntryAbsorbing<const MAX: u16>;

impl<const MAX: u16> private::EffectEntryModeSealed for EntryAbsorbing<MAX> {}

impl<const MAX: u16> EffectEntryMode for EntryAbsorbing<MAX> {
    const MAX_ENTRIES: u16 = MAX;
}

/// Framework-owned evidence that an entry mode is declarable for a request type.
///
/// Sealing the *parameterized* marker is what makes this an obligation rather
/// than a suggestion. Sealing only [`EffectEntryMode`] and taking the request as
/// a trait parameter would compile, look sealed, and not be: orphan rules permit
/// `impl ForeignTrait<LocalType> for ForeignType`, so a downstream crate could
/// declare absorption over an unkeyed request and it would build clean.
pub trait EffectEntryModeFor<Req>:
    EffectEntryMode + private::EffectEntryModeForSealed<Req>
{
}

impl<Req, M> EffectEntryModeFor<Req> for M where
    M: EffectEntryMode + private::EffectEntryModeForSealed<Req>
{
}

/// Typed application-protocol contract for one external Effect capability.
pub trait EffectCapabilityContract: Send + Sync + 'static {
    /// Immutable typed request authored by a state callback.
    type Request: MfmValue;
    /// Schema-valid typed returned value.
    type Returned: MfmValue;
    /// Reviewed redaction-safe definite failure visible to state settlement.
    type SafeFailure: MfmValue;
    /// Exact resource-refresh contract for this effect.
    type Refresh: EffectRefreshMode;
    /// Re-entry discipline, declarable as absorbing only over a keyed request.
    type Entry: EffectEntryModeFor<Self::Request>;
}

/// Closed completion produced by one qualified Read adapter invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadAdapterCompletion<Returned, SafeFailure> {
    /// One schema-valid typed value returned.
    Returned(Returned),
    /// One reviewed definite state-facing failure.
    SafeFailure(SafeFailure),
    /// One integrity-blocking disposition.
    IntegrityFault(AccessFaultCode),
}

/// Closed completion produced by one qualified Effect adapter invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectAdapterCompletion<Returned, SafeFailure, RefreshEvidence> {
    /// One schema-valid typed value returned.
    Returned(Returned),
    /// One reviewed definite state-facing failure.
    SafeFailure(SafeFailure),
    /// Qualified evidence that the old authority was superseded before entry.
    SupersededBeforeEntry(RefreshEvidence),
    /// Entry may have happened and the effect must remain parked.
    EntryUnknown(AccessFaultCode),
    /// One integrity-blocking disposition.
    IntegrityFault(AccessFaultCode),
}

/// Closed completion for one exact Effect capability contract.
pub type EffectContractCompletion<C> = EffectAdapterCompletion<
    <C as EffectCapabilityContract>::Returned,
    <C as EffectCapabilityContract>::SafeFailure,
    <<C as EffectCapabilityContract>::Refresh as EffectRefreshMode>::Evidence,
>;

/// Qualified callback-free validator for one Read capability.
pub trait ReadCapabilityImplementation<C: ReadCapabilityContract>: Send + Sync + 'static {
    /// Validates one exact typed request before authorization.
    fn validate_request(
        &self,
        request: &C::Request,
    ) -> std::result::Result<(), CapabilityContractFault>;

    /// Validates one returned value without changing its disposition.
    fn validate_returned(
        &self,
        returned: &C::Returned,
    ) -> std::result::Result<(), CapabilityContractFault>;

    /// Validates one reviewed safe failure without changing its disposition.
    fn validate_safe_failure(
        &self,
        failure: &C::SafeFailure,
    ) -> std::result::Result<(), CapabilityContractFault>;
}

/// Qualified callback-free validator for one Effect capability.
pub trait EffectCapabilityImplementation<C: EffectCapabilityContract>:
    Send + Sync + 'static
{
    /// Validates one exact typed request before authorization.
    fn validate_request(
        &self,
        request: &C::Request,
    ) -> std::result::Result<(), CapabilityContractFault>;

    /// Validates one returned value without changing its disposition.
    fn validate_returned(
        &self,
        returned: &C::Returned,
    ) -> std::result::Result<(), CapabilityContractFault>;

    /// Validates one reviewed safe failure without changing its disposition.
    fn validate_safe_failure(
        &self,
        failure: &C::SafeFailure,
    ) -> std::result::Result<(), CapabilityContractFault>;

    /// Validates exact supersession evidence without changing its disposition.
    fn validate_superseded_before_entry(
        &self,
        evidence: &<C::Refresh as EffectRefreshMode>::Evidence,
    ) -> std::result::Result<(), CapabilityContractFault>;

    /// Validates an entry-unknown fault without changing its disposition.
    fn validate_entry_unknown(
        &self,
        fault: &AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault>;

    /// Validates an integrity fault without changing its disposition.
    fn validate_integrity_fault(
        &self,
        fault: &AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault>;
}

/// Qualified process-private live Read adapter invoker.
pub trait ReadAdapterInvoker<C: ReadCapabilityContract>: Send + Sync + 'static {
    /// Performs exactly one bounded invocation and returns a closed completion.
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>>;
}

/// Qualified process-private live Effect adapter invoker.
pub trait EffectAdapterInvoker<C: EffectCapabilityContract>: Send + Sync + 'static {
    /// Performs exactly one bounded invocation and returns a closed completion.
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, EffectContractCompletion<C>>;
}

/// Typed contract for one bounded signer or resource-authority operation.
pub trait BoundedComponentContract: Send + Sync + 'static {
    /// Exact process-local request type.
    type Request: Send + Sync + 'static;
    /// Closed process-local completion type.
    type Completion: Send + Sync + 'static;
}

/// Shared qualified invoker contract for bounded signer/resource operations.
pub trait BoundedComponentInvoker<C: BoundedComponentContract>: Send + Sync + 'static {
    /// Performs exactly one bounded invocation.
    fn invoke<'a>(&'a self, request: &'a C::Request) -> ComponentFuture<'a, C::Completion>;
}

/// Nominal bounded signer contract.
pub trait SignerContract: BoundedComponentContract {}

/// Nominal bounded cross-run resource-authority contract.
pub trait ResourceAuthorityContract: BoundedComponentContract {}

/// Error returned by effect descriptor construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EffectError {
    /// Effect identity or version construction failed.
    #[error("effect identity error: {0}")]
    Identity(String),
}

/// Pure deterministic computation with no external capability access.
pub enum Pure {}

/// External read effect; observes outside systems only through declared read
/// capabilities and replayable facts.
pub enum ReadExternal {}

/// External mutation effect governed by the side-effect ledger protocol.
pub enum ApplySideEffect {}

/// Sealed effect marker descriptor contract.
pub trait EffectSpec: private::EffectSealed + Send + Sync + 'static {
    /// Returns the stable effect kind id.
    fn kind() -> std::result::Result<EffectKind, EffectError>;

    /// Returns the effect descriptor version.
    fn version() -> std::result::Result<EffectVersion, EffectError> {
        effect_version()
    }

    /// Returns the semantic effect class.
    fn class() -> EffectClass;

    /// Returns the stable framework-owned effect name.
    fn name() -> &'static str;

    /// Returns the stable effect descriptor.
    fn descriptor() -> std::result::Result<EffectDescriptor, EffectError> {
        Ok(EffectDescriptor {
            kind: Self::kind()?,
            version: Self::version()?,
            class: Self::class(),
            name: Self::name(),
        })
    }
}

/// Stable effect descriptor used by state descriptors and certification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDescriptor {
    /// Stable effect kind id.
    pub kind: EffectKind,
    /// Effect descriptor contract version.
    pub version: EffectVersion,
    /// Semantic effect class.
    pub class: EffectClass,
    /// Stable framework-owned effect name.
    pub name: &'static str,
}

/// Framework-owned effect classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectClass {
    /// Pure deterministic computation.
    Pure,
    /// External read through declared read/support capabilities.
    ReadExternal,
    /// External mutation with side-effect ledger authority.
    ApplySideEffect,
}

impl EffectClass {
    /// Returns the stable descriptor string for this effect class.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::ReadExternal => "read_external",
            Self::ApplySideEffect => "apply_side_effect",
        }
    }
}

macro_rules! impl_effect_spec {
    ($ty:ty, $class:expr, $name:literal) => {
        impl private::EffectSealed for $ty {}

        impl EffectSpec for $ty {
            fn kind() -> std::result::Result<EffectKind, EffectError> {
                effect_kind($name)
            }

            fn class() -> EffectClass {
                $class
            }

            fn name() -> &'static str {
                $name
            }
        }
    };
}

impl_effect_spec!(Pure, EffectClass::Pure, "pure");
impl_effect_spec!(ReadExternal, EffectClass::ReadExternal, "read_external");
impl_effect_spec!(
    ApplySideEffect,
    EffectClass::ApplySideEffect,
    "apply_side_effect"
);

fn effect_kind(name: &'static str) -> std::result::Result<EffectKind, EffectError> {
    let digest = sha256_digest_bytes(format!("effect-kind:mfm.kernel.effect:{name}").as_bytes());
    EffectKind::new(
        "mfm.kernel.effect",
        name,
        DigestAlgorithm::Sha256JcsV1,
        digest,
    )
    .map_err(|error| EffectError::Identity(error.to_string()))
}

fn effect_version() -> std::result::Result<EffectVersion, EffectError> {
    EffectVersion::new("mfm.effect.v1").map_err(|error| EffectError::Identity(error.to_string()))
}

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
    use super::EffectSpec;
    use super::{EntryAbsorbing, EntryKeyed, EntryOnce};
    use super::{ExternalMutationAuthorityRole, ReadExternalRole, SupportRole};

    pub trait EffectSealed {}

    pub trait EffectRefreshModeSealed {}

    pub trait EffectEntryModeSealed {}

    pub trait EffectEntryModeForSealed<Req> {}

    impl<Req> EffectEntryModeForSealed<Req> for EntryOnce {}

    // The obligation. Absorption is unavailable without an entry key, so a
    // capability over a target with no such value cannot declare it at all.
    impl<Req, const MAX: u16> EffectEntryModeForSealed<Req> for EntryAbsorbing<MAX> where Req: EntryKeyed
    {}

    pub trait RoleSealed {}

    pub trait CapabilitySetSealed {}

    pub trait CapabilitySetForSealed<E: EffectSpec> {}

    pub trait ReadEffectRole {}

    impl ReadEffectRole for ReadExternalRole {}

    impl ReadEffectRole for SupportRole {}

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
}
