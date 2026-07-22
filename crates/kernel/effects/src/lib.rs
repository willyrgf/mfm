#![warn(missing_docs)]
//! Effect marker contracts for the MFM typed kernel.
//!
//! Effects classify the executable behavior a typed state may use. The marker
//! types in this crate are framework-owned and sealed; downstream crates may
//! name them in `StateSpec::Effect`, but cannot add new effect classes or
//! implement [`EffectSpec`] manually.
//!
//! ```compile_fail
//! struct CustomEffect;
//!
//! impl mfm_effects::EffectSpec for CustomEffect {
//!     fn kind() -> mfm_effects::Result<mfm_ids::EffectKind> { unimplemented!() }
//!     fn class() -> mfm_effects::EffectClass { unimplemented!() }
//!     fn name() -> &'static str { "custom" }
//! }
//! ```

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{DigestAlgorithm, EffectKind, EffectVersion};

#[cfg(test)]
mod tests;

/// Result type for effect descriptor helpers.
pub type Result<T> = std::result::Result<T, EffectError>;

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
pub trait EffectSpec: private::Sealed + Send + Sync + 'static {
    /// Returns the stable effect kind id.
    fn kind() -> Result<EffectKind>;

    /// Returns the effect descriptor version.
    fn version() -> Result<EffectVersion> {
        effect_version()
    }

    /// Returns the semantic effect class.
    fn class() -> EffectClass;

    /// Returns the stable framework-owned effect name.
    fn name() -> &'static str;

    /// Returns the stable effect descriptor.
    fn descriptor() -> Result<EffectDescriptor> {
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
        impl private::Sealed for $ty {}

        impl EffectSpec for $ty {
            fn kind() -> Result<EffectKind> {
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

fn effect_kind(name: &'static str) -> Result<EffectKind> {
    let digest = sha256_digest_bytes(format!("effect-kind:mfm.kernel.effect:{name}").as_bytes());
    EffectKind::new(
        "mfm.kernel.effect",
        name,
        DigestAlgorithm::Sha256JcsV1,
        digest,
    )
    .map_err(|error| EffectError::Identity(error.to_string()))
}

fn effect_version() -> Result<EffectVersion> {
    EffectVersion::new("mfm.effect.v1").map_err(|error| EffectError::Identity(error.to_string()))
}

mod private {
    pub trait Sealed {}
}
