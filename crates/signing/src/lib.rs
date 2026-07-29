#![warn(missing_docs)]
//! Generic signer capability contracts.
//!
//! This crate defines both the reusable unqualified signer-facing boundary and
//! the separate generation-guarded deterministic wallet boundary. Qualified
//! wallet providers fix one complete public binding and cannot be used through
//! the general direct-sign trait. Their deployment guard is inseparable from
//! each signing call. Private keys, passwords, endpoint paths, signatures,
//! signed payloads, and provider runtime resolution remain outside persisted
//! workflow config and values.
//!
//! ```rust
//! use mfm_ids::DigestBytes;
//! use mfm_signing::{
//!     SignerRef, SigningAlgorithmId, SigningDomainId, SigningProfileId, SigningPurposeId,
//!     SigningRequest,
//! };
//!
//! let request = SigningRequest::from_digest(
//!     SignerRef::new("deployer")?,
//!     SigningAlgorithmId::new("secp256k1.keccak256.recoverable")?,
//!     SigningProfileId::new("secp256k1.rfc6979.recoverable.low_s.v1")?,
//!     SigningDomainId::new("example.transaction")?,
//!     SigningPurposeId::new("example.transaction.submit")?,
//!     DigestBytes::from_array([0x42; 32]),
//! );
//! assert_eq!(request.signer_ref().as_str(), "deployer");
//! # Ok::<(), mfm_signing::SigningError>(())
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, CanonicalBytes, CanonicalJsonBytes, CanonicalValue};
use mfm_capabilities::{CapabilityError, CapabilitySpec, SupportRole};
use mfm_ids::{
    CapabilityKind, CapabilityVersion, CheckedStringError, CheckedStringErrorReason, ContentDigest,
    DigestAlgorithm, DigestBytes, LocalPublicId, SchemaId,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub use mfm_ids::ContentRef;

/// Result type for signer contracts.
pub type Result<T> = std::result::Result<T, SigningError>;

/// Boxed future returned by signer providers.
pub type SigningFuture<'a> = Pin<Box<dyn Future<Output = Result<SigningResult>> + Send + 'a>>;

/// Future returned by a guarded deterministic signing-provider binder.
pub type GenerationGuardedDeterministicSigningProviderBindFuture = Pin<
    Box<
        dyn Future<Output = Result<Arc<dyn GenerationGuardedDeterministicSigningProvider>>>
            + Send
            + 'static,
    >,
>;

/// Future returned by a deployment-supplied signer-generation guard.
pub type SigningGenerationGuardFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), SigningGenerationGuardError>> + Send + 'a>>;

const MAX_PUBLIC_KEY_LEN: usize = 4096;
const MAX_SIGNATURE_LEN: usize = 4096;
const GENERATION_GUARDED_SIGNER_DESCRIPTOR_SCHEMA_NAME: &str =
    "mfm.signing.generation-guarded-signer-descriptor";
const GENERATION_GUARDED_SIGNER_DESCRIPTOR_VERSION: &str =
    "mfm.signing.generation-guarded-signer-descriptor.v1";

/// Recoverable secp256k1 signature over a Keccak-256 digest.
pub const SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID: &str = "secp256k1.keccak256.recoverable";
/// Deterministic RFC 6979, recoverable, low-s secp256k1 signing profile.
pub const SECP256K1_RFC6979_LOW_S_PROFILE_ID: &str = "secp256k1.rfc6979.recoverable.low_s.v1";

/// Stable capability descriptor for signer providers.
pub struct SigningCapability;

impl CapabilitySpec for SigningCapability {
    type Role = SupportRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.signing",
            "sign",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.signing.capability:sign"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.signing.sign.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.signing.sign"
    }
}

/// Provider interface for reusable signer implementations.
pub trait SigningProvider: Send + Sync {
    /// Signs the supplied request and returns a signature plus public metadata.
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a>;
}

/// Provider contract for profiles whose repeated signing result is byte-identical.
///
/// Implementors certify that the same signer identity, algorithm, profile, and digest always
/// produce the same canonical signature bytes, including after provider reconstruction. This
/// stronger contract is required when recovery must regenerate bearer material that is purposely
/// not persisted.
pub trait DeterministicSigningProvider: SigningProvider {
    /// Returns the non-secret implementation identity of this concrete provider.
    fn implementation_id(&self) -> &'static str;

    /// Returns the one deterministic profile certified by this provider binding.
    fn deterministic_profile_id(&self) -> &'static str;
}

/// Deterministic signer whose wallet generation and direct-sign exclusion are
/// checked inseparably from every signing call.
///
/// This is a separate contract from [`SigningProvider`]. A qualified wallet
/// implementation must not expose its key through that unguarded trait.
pub trait GenerationGuardedDeterministicSigningProvider: Send + Sync {
    /// Returns the one exact public wallet/provider binding.
    fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding;

    /// Checks the mandatory deployment guard and signs the transient request.
    ///
    /// The implementation must reject an unexpected generation before
    /// consulting the guard. The guard must then complete successfully
    /// immediately before any key file, key handle, or private-key access.
    fn sign_guarded<'a>(
        &'a self,
        expected_generation_ref: &'a ContentRef,
        request: &'a SigningRequest,
    ) -> SigningFuture<'a>;
}

/// Deployment guard for one exact wallet generation and access-control binding.
///
/// This contract does not grant signing authority by itself. It is injected
/// into a concrete guarded provider, which invokes it inside
/// [`GenerationGuardedDeterministicSigningProvider::sign_guarded`].
pub trait SigningGenerationGuard: Send + Sync {
    /// Proves that the bound generation is current, stale/sibling writers are
    /// fenced, and the wallet key is excluded from general direct signing.
    fn verify_current_and_exclusive<'a>(
        &'a self,
        binding: &'a VerifiedGenerationGuardedSignerBinding,
    ) -> SigningGenerationGuardFuture<'a>;
}

type BindGenerationGuardedDeterministicSigningProvider =
    dyn Fn() -> GenerationGuardedDeterministicSigningProviderBindFuture + Send + Sync;

/// Process-local binder for one exact qualified wallet signer.
///
/// The binder owns the binding rather than accepting caller-selected signer
/// material. It rejects a provider that returns any different public binding.
#[derive(Clone)]
pub struct GenerationGuardedDeterministicSigningProviderBinder {
    binding: VerifiedGenerationGuardedSignerBinding,
    bind: Arc<BindGenerationGuardedDeterministicSigningProvider>,
}

impl GenerationGuardedDeterministicSigningProviderBinder {
    /// Creates a binder for one exact verified wallet binding.
    pub fn new<B>(binding: VerifiedGenerationGuardedSignerBinding, bind: B) -> Self
    where
        B: Fn() -> GenerationGuardedDeterministicSigningProviderBindFuture + Send + Sync + 'static,
    {
        Self {
            binding,
            bind: Arc::new(bind),
        }
    }

    /// Returns the exact wallet/provider binding fixed by this binder.
    pub const fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        &self.binding
    }

    /// Resolves the exact guarded provider and rechecks its complete binding.
    pub fn bind(&self) -> GenerationGuardedDeterministicSigningProviderBindFuture {
        let expected = self.binding.clone();
        let provider = (self.bind)();
        Box::pin(async move {
            let provider = provider.await?;
            if provider.binding() != &expected {
                return Err(SigningError::Provider {
                    reason: SigningProviderError::BindingMismatch,
                });
            }
            Ok(provider)
        })
    }
}

/// Process-local signer reference used by workflow config and runtime binding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SignerRef(LocalPublicId);

impl SignerRef {
    /// Creates a checked signer reference.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = checked_local_public_id(SigningIdentifierKind::SignerRef, value)?;
        Ok(Self(value))
    }

    /// Returns the canonical signer reference string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for SignerRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SignerRef {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for SignerRef {
    type Error = SigningError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<SignerRef> for String {
    fn from(value: SignerRef) -> Self {
        value.0.into_string()
    }
}

/// Signing algorithm identifier, such as an EVM or Ed25519 signing algorithm.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SigningAlgorithmId(LocalPublicId);

impl SigningAlgorithmId {
    /// Creates a checked signing algorithm identifier.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = checked_local_public_id(SigningIdentifierKind::Algorithm, value)?;
        Ok(Self(value))
    }

    /// Returns the canonical identifier string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for SigningAlgorithmId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SigningAlgorithmId {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for SigningAlgorithmId {
    type Error = SigningError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<SigningAlgorithmId> for String {
    fn from(value: SigningAlgorithmId) -> Self {
        value.0.into_string()
    }
}

/// Signing-profile identifier that makes provider behavior an explicit contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SigningProfileId(LocalPublicId);

impl SigningProfileId {
    /// Creates a checked signing-profile identifier.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = checked_local_public_id(SigningIdentifierKind::Profile, value)?;
        Ok(Self(value))
    }

    /// Returns the canonical identifier string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for SigningProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SigningProfileId {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for SigningProfileId {
    type Error = SigningError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<SigningProfileId> for String {
    fn from(value: SigningProfileId) -> Self {
        value.0.into_string()
    }
}

/// Signing domain identifier that scopes signed bytes to a protocol/domain.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SigningDomainId(LocalPublicId);

impl SigningDomainId {
    /// Creates a checked signing domain identifier.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = checked_local_public_id(SigningIdentifierKind::Domain, value)?;
        Ok(Self(value))
    }

    /// Returns the canonical identifier string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for SigningDomainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SigningDomainId {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for SigningDomainId {
    type Error = SigningError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<SigningDomainId> for String {
    fn from(value: SigningDomainId) -> Self {
        value.0.into_string()
    }
}

/// Signing purpose identifier that explains why a request is being signed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SigningPurposeId(LocalPublicId);

impl SigningPurposeId {
    /// Creates a checked signing purpose identifier.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = checked_local_public_id(SigningIdentifierKind::Purpose, value)?;
        Ok(Self(value))
    }

    /// Returns the canonical identifier string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for SigningPurposeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SigningPurposeId {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<String> for SigningPurposeId {
    type Error = SigningError;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<SigningPurposeId> for String {
    fn from(value: SigningPurposeId) -> Self {
        value.0.into_string()
    }
}

/// Transient signing request.
///
/// The request is intentionally neither serializable nor cloneable, and its
/// signing digest is zeroized when the request is dropped.
///
/// ```compile_fail
/// use mfm_signing::SigningRequest;
///
/// fn require_clone<T: Clone>() {}
/// require_clone::<SigningRequest>();
/// ```
///
/// ```compile_fail
/// use mfm_signing::SigningRequest;
///
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<SigningRequest>();
/// ```
#[derive(PartialEq, Eq)]
pub struct SigningRequest {
    signer_ref: SignerRef,
    algorithm: SigningAlgorithmId,
    profile: SigningProfileId,
    domain: SigningDomainId,
    purpose: SigningPurposeId,
    digest: Zeroizing<[u8; 32]>,
    expected_identity: Option<ExpectedSignerIdentity>,
}

impl SigningRequest {
    /// Creates a request for providers that sign an already-domain-separated digest.
    pub fn from_digest(
        signer_ref: SignerRef,
        algorithm: SigningAlgorithmId,
        profile: SigningProfileId,
        domain: SigningDomainId,
        purpose: SigningPurposeId,
        digest: DigestBytes,
    ) -> Self {
        Self {
            signer_ref,
            algorithm,
            profile,
            domain,
            purpose,
            digest: Zeroizing::new(*digest.as_bytes()),
            expected_identity: None,
        }
    }

    /// Requires the provider result to match the expected public identity.
    pub fn require_public_identity(mut self, expected_identity: ExpectedSignerIdentity) -> Self {
        self.expected_identity = Some(expected_identity);
        self
    }

    /// Returns the signer reference.
    pub fn signer_ref(&self) -> &SignerRef {
        &self.signer_ref
    }

    /// Returns the signing algorithm.
    pub fn algorithm(&self) -> &SigningAlgorithmId {
        &self.algorithm
    }

    /// Returns the required signing behavior profile.
    pub fn profile(&self) -> &SigningProfileId {
        &self.profile
    }

    /// Returns the signing domain.
    pub fn domain(&self) -> &SigningDomainId {
        &self.domain
    }

    /// Returns the signing purpose.
    pub fn purpose(&self) -> &SigningPurposeId {
        &self.purpose
    }

    /// Returns the domain-separated digest providers must sign.
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Returns the expected public identity, if one was supplied.
    pub fn expected_identity(&self) -> Option<&ExpectedSignerIdentity> {
        self.expected_identity.as_ref()
    }
}

impl fmt::Debug for SigningRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningRequest")
            .field("signer_ref", &self.signer_ref)
            .field("algorithm", &self.algorithm)
            .field("profile", &self.profile)
            .field("domain", &self.domain)
            .field("purpose", &self.purpose)
            .field("digest", &"digest")
            .field("expected_identity", &self.expected_identity)
            .finish()
    }
}

/// Public signer identity returned by providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicSigningIdentity {
    algorithm: SigningAlgorithmId,
    public_key: Option<PublicKeyBytes>,
    account_id: Option<LocalPublicId>,
}

impl PublicSigningIdentity {
    /// Creates public identity metadata.
    pub fn new(
        algorithm: SigningAlgorithmId,
        public_key: Option<PublicKeyBytes>,
        account_id: Option<String>,
    ) -> Result<Self> {
        if public_key.is_none() && account_id.is_none() {
            return Err(SigningError::InvalidRequest {
                reason: SigningRequestError::MissingPublicIdentity,
            });
        }
        let account_id = account_id
            .map(|account_id| checked_local_public_id(SigningIdentifierKind::Account, account_id))
            .transpose()?;
        Ok(Self {
            algorithm,
            public_key,
            account_id,
        })
    }

    /// Returns the algorithm this identity belongs to.
    pub fn algorithm(&self) -> &SigningAlgorithmId {
        &self.algorithm
    }

    /// Returns the public key bytes, when available.
    pub fn public_key(&self) -> Option<&PublicKeyBytes> {
        self.public_key.as_ref()
    }

    /// Returns the public account identifier, when available.
    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_ref().map(LocalPublicId::as_str)
    }
}

/// Expected public signer identity for request/result verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSignerIdentity {
    public_key: Option<PublicKeyBytes>,
    account_id: Option<LocalPublicId>,
}

impl ExpectedSignerIdentity {
    /// Expects a specific public key.
    pub fn public_key(public_key: PublicKeyBytes) -> Self {
        Self {
            public_key: Some(public_key),
            account_id: None,
        }
    }

    /// Expects a specific public account identifier.
    pub fn account_id(account_id: impl AsRef<str>) -> Result<Self> {
        let account_id = checked_local_public_id(SigningIdentifierKind::Account, account_id)?;
        Ok(Self {
            public_key: None,
            account_id: Some(account_id),
        })
    }

    /// Expects both public key and public account identifier.
    pub fn public_key_and_account_id(
        public_key: PublicKeyBytes,
        account_id: impl AsRef<str>,
    ) -> Result<Self> {
        let account_id = checked_local_public_id(SigningIdentifierKind::Account, account_id)?;
        Ok(Self {
            public_key: Some(public_key),
            account_id: Some(account_id),
        })
    }

    /// Returns the expected public key, when one is fixed.
    pub fn public_key_ref(&self) -> Option<&PublicKeyBytes> {
        self.public_key.as_ref()
    }

    /// Returns the expected public account identifier, when one is fixed.
    pub fn account_id_ref(&self) -> Option<&str> {
        self.account_id.as_ref().map(LocalPublicId::as_str)
    }

    /// Verifies public provider identity metadata.
    pub fn verify(&self, signer_ref: &SignerRef, actual: &PublicSigningIdentity) -> Result<()> {
        if let Some(expected) = &self.public_key {
            if actual.public_key() != Some(expected) {
                return Err(SigningError::PublicIdentityMismatch {
                    signer_ref: signer_ref.clone(),
                    field: "public_key",
                });
            }
        }
        if let Some(expected) = &self.account_id {
            if actual.account_id() != Some(expected.as_str()) {
                return Err(SigningError::PublicIdentityMismatch {
                    signer_ref: signer_ref.clone(),
                    field: "account_id",
                });
            }
        }
        Ok(())
    }

    fn matches_exactly(&self, actual: &PublicSigningIdentity) -> bool {
        self.public_key_ref() == actual.public_key() && self.account_id_ref() == actual.account_id()
    }
}

/// Fully validated public identity and generation binding for one guarded signer.
///
/// The binding contains no private key, unlock source, endpoint, credential,
/// signature, or signed payload. Its three content references fix the wallet's
/// durable generation, deployment fence evidence, and provider/ACL proof that
/// excludes the key from general direct-sign access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGenerationGuardedSignerBinding {
    signer_ref: SignerRef,
    provider_implementation_id: LocalPublicId,
    algorithm: SigningAlgorithmId,
    profile: SigningProfileId,
    expected_public_identity: PublicSigningIdentity,
    durable_generation_ref: ContentRef,
    fence_attestation_ref: ContentRef,
    direct_sign_exclusion_ref: ContentRef,
}

impl VerifiedGenerationGuardedSignerBinding {
    /// Validates and fixes one complete guarded signer binding.
    #[allow(clippy::too_many_arguments)]
    pub fn verify(
        signer_ref: SignerRef,
        provider_implementation_id: impl AsRef<str>,
        algorithm: SigningAlgorithmId,
        profile: SigningProfileId,
        expected_public_identity: PublicSigningIdentity,
        durable_generation_ref: ContentRef,
        fence_attestation_ref: ContentRef,
        direct_sign_exclusion_ref: ContentRef,
    ) -> Result<Self> {
        if expected_public_identity.algorithm() != &algorithm {
            return Err(SigningError::PublicIdentityMismatch {
                signer_ref,
                field: "algorithm",
            });
        }
        if expected_public_identity.account_id().is_none() {
            return Err(SigningError::InvalidRequest {
                reason: SigningRequestError::MissingAccountIdentity,
            });
        }
        Ok(Self {
            signer_ref,
            provider_implementation_id: checked_local_public_id(
                SigningIdentifierKind::Implementation,
                provider_implementation_id,
            )?,
            algorithm,
            profile,
            expected_public_identity,
            durable_generation_ref,
            fence_attestation_ref,
            direct_sign_exclusion_ref,
        })
    }

    /// Returns the exact process-local signer reference.
    pub const fn signer_ref(&self) -> &SignerRef {
        &self.signer_ref
    }

    /// Returns the concrete provider implementation identity.
    pub const fn provider_implementation_id(&self) -> &LocalPublicId {
        &self.provider_implementation_id
    }

    /// Returns the exact signing algorithm.
    pub const fn algorithm(&self) -> &SigningAlgorithmId {
        &self.algorithm
    }

    /// Returns the exact deterministic signing profile.
    pub const fn profile(&self) -> &SigningProfileId {
        &self.profile
    }

    /// Returns the expected public signer identity, including its account id.
    pub const fn expected_public_identity(&self) -> &PublicSigningIdentity {
        &self.expected_public_identity
    }

    /// Returns the independently durable wallet/executor generation.
    pub const fn durable_generation_ref(&self) -> &ContentRef {
        &self.durable_generation_ref
    }

    /// Returns the deployment fence-attestation identity.
    pub const fn fence_attestation_ref(&self) -> &ContentRef {
        &self.fence_attestation_ref
    }

    /// Returns the provider/ACL direct-sign-exclusion identity.
    pub const fn direct_sign_exclusion_ref(&self) -> &ContentRef {
        &self.direct_sign_exclusion_ref
    }

    /// Derives the complete secret-free descriptor of this guarded signer.
    ///
    /// ```
    /// use mfm_signing::{
    ///     GenerationGuardedSignerDescriptor, VerifiedGenerationGuardedSignerBinding,
    /// };
    ///
    /// fn retained_descriptor(
    ///     binding: &VerifiedGenerationGuardedSignerBinding,
    /// ) -> mfm_signing::Result<GenerationGuardedSignerDescriptor> {
    ///     binding.public_descriptor()
    /// }
    /// ```
    pub fn public_descriptor(&self) -> Result<GenerationGuardedSignerDescriptor> {
        GenerationGuardedSignerDescriptor::from_verified(self)
    }

    /// Verifies that a transient request exactly matches this wallet binding.
    pub fn verify_request(&self, request: &SigningRequest) -> Result<()> {
        if request.signer_ref() != self.signer_ref()
            || request.algorithm() != self.algorithm()
            || request.profile() != self.profile()
            || !request
                .expected_identity()
                .is_some_and(|expected| expected.matches_exactly(&self.expected_public_identity))
        {
            return Err(SigningError::Provider {
                reason: SigningProviderError::BindingMismatch,
            });
        }
        Ok(())
    }
}

/// Canonical secret-free descriptor derived from one verified guarded signer.
///
/// This value has no caller-selected field constructor. It is derived from
/// [`VerifiedGenerationGuardedSignerBinding`] so its content reference fixes
/// every public signer, provider, identity, generation, fence, and
/// direct-sign-exclusion field together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationGuardedSignerDescriptor {
    signer_ref: SignerRef,
    provider_implementation_id: LocalPublicId,
    algorithm: SigningAlgorithmId,
    profile: SigningProfileId,
    expected_public_identity: PublicSigningIdentity,
    durable_generation_ref: ContentRef,
    fence_attestation_ref: ContentRef,
    direct_sign_exclusion_ref: ContentRef,
    canonical: CanonicalJsonBytes,
    reference: ContentRef,
}

impl GenerationGuardedSignerDescriptor {
    fn from_verified(binding: &VerifiedGenerationGuardedSignerBinding) -> Result<Self> {
        let identity = binding.expected_public_identity();
        let public_key = identity
            .public_key()
            .map(|value| CanonicalValue::Bytes(CanonicalBytes::new(value.as_bytes().to_vec())))
            .unwrap_or(CanonicalValue::Null);
        let account_id = identity
            .account_id()
            .map(|value| CanonicalValue::String(value.to_owned()))
            .unwrap_or(CanonicalValue::Null);
        let public_identity =
            CanonicalValue::object([("account_id", account_id), ("public_key", public_key)])
                .map_err(|_| invalid_public_descriptor())?;
        let canonical = CanonicalJsonBytes::from_value(
            &CanonicalValue::object([
                (
                    "algorithm",
                    CanonicalValue::String(binding.algorithm().as_str().to_owned()),
                ),
                (
                    "direct_sign_exclusion_ref",
                    content_ref_value(binding.direct_sign_exclusion_ref())?,
                ),
                (
                    "durable_generation_ref",
                    content_ref_value(binding.durable_generation_ref())?,
                ),
                ("expected_public_identity", public_identity),
                (
                    "fence_attestation_ref",
                    content_ref_value(binding.fence_attestation_ref())?,
                ),
                (
                    "profile",
                    CanonicalValue::String(binding.profile().as_str().to_owned()),
                ),
                (
                    "provider_implementation_id",
                    CanonicalValue::String(
                        binding.provider_implementation_id().as_str().to_owned(),
                    ),
                ),
                (
                    "signer_ref",
                    CanonicalValue::String(binding.signer_ref().as_str().to_owned()),
                ),
                (
                    "version",
                    CanonicalValue::String(GENERATION_GUARDED_SIGNER_DESCRIPTOR_VERSION.to_owned()),
                ),
            ])
            .map_err(|_| invalid_public_descriptor())?,
        );
        let schema_id = SchemaId::new(
            GENERATION_GUARDED_SIGNER_DESCRIPTOR_SCHEMA_NAME,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(
                format!("schema:{GENERATION_GUARDED_SIGNER_DESCRIPTOR_SCHEMA_NAME}:1").as_bytes(),
            ),
        )
        .map_err(|_| invalid_public_descriptor())?;
        let reference = ContentRef::new(
            schema_id,
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, canonical.digest_bytes()),
        )
        .map_err(|_| invalid_public_descriptor())?;
        Ok(Self {
            signer_ref: binding.signer_ref().clone(),
            provider_implementation_id: binding.provider_implementation_id().clone(),
            algorithm: binding.algorithm().clone(),
            profile: binding.profile().clone(),
            expected_public_identity: binding.expected_public_identity().clone(),
            durable_generation_ref: binding.durable_generation_ref().clone(),
            fence_attestation_ref: binding.fence_attestation_ref().clone(),
            direct_sign_exclusion_ref: binding.direct_sign_exclusion_ref().clone(),
            canonical,
            reference,
        })
    }

    /// Returns the exact process-local signer reference.
    pub const fn signer_ref(&self) -> &SignerRef {
        &self.signer_ref
    }

    /// Returns the concrete provider implementation identity.
    pub const fn provider_implementation_id(&self) -> &LocalPublicId {
        &self.provider_implementation_id
    }

    /// Returns the exact signing algorithm.
    pub const fn algorithm(&self) -> &SigningAlgorithmId {
        &self.algorithm
    }

    /// Returns the exact deterministic signing profile.
    pub const fn profile(&self) -> &SigningProfileId {
        &self.profile
    }

    /// Returns the expected public signer identity.
    pub const fn expected_public_identity(&self) -> &PublicSigningIdentity {
        &self.expected_public_identity
    }

    /// Returns the independently durable wallet/executor generation.
    pub const fn durable_generation_ref(&self) -> &ContentRef {
        &self.durable_generation_ref
    }

    /// Returns the destination fence-attestation identity.
    pub const fn fence_attestation_ref(&self) -> &ContentRef {
        &self.fence_attestation_ref
    }

    /// Returns the provider/ACL direct-sign-exclusion identity.
    pub const fn direct_sign_exclusion_ref(&self) -> &ContentRef {
        &self.direct_sign_exclusion_ref
    }

    /// Returns exact canonical descriptor bytes.
    pub const fn canonical(&self) -> &CanonicalJsonBytes {
        &self.canonical
    }

    /// Returns the exact content identity of the canonical descriptor.
    pub const fn reference(&self) -> &ContentRef {
        &self.reference
    }
}

/// Public key bytes. This is public metadata, not private signing material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKeyBytes(Vec<u8>);

impl PublicKeyBytes {
    /// Creates checked public key bytes.
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        validate_bytes_len(
            bytes.len(),
            MAX_PUBLIC_KEY_LEN,
            SigningRequestError::InvalidPublicKey,
        )?;
        Ok(Self(bytes))
    }

    /// Returns the public key bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Signature bytes returned by a signer provider.
///
/// Signature bytes are transient bearer material. They are intentionally
/// neither serializable nor cloneable and are zeroized when dropped.
///
/// ```compile_fail
/// use mfm_signing::SignatureBytes;
///
/// fn require_clone<T: Clone>() {}
/// require_clone::<SignatureBytes>();
/// ```
///
/// ```compile_fail
/// use mfm_signing::SignatureBytes;
///
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<SignatureBytes>();
/// ```
#[derive(PartialEq, Eq)]
pub struct SignatureBytes(Zeroizing<Vec<u8>>);

impl SignatureBytes {
    /// Creates checked signature bytes.
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let bytes = Zeroizing::new(bytes);
        validate_bytes_len(
            bytes.len(),
            MAX_SIGNATURE_LEN,
            SigningRequestError::InvalidSignature,
        )?;
        Ok(Self(bytes))
    }

    /// Returns the signature bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SignatureBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SignatureBytes(<redacted>)")
    }
}

/// Signing result returned by a provider.
///
/// The result owns transient signature bearer material and is intentionally
/// neither serializable nor cloneable.
///
/// ```compile_fail
/// use mfm_signing::SigningResult;
///
/// fn require_clone<T: Clone>() {}
/// require_clone::<SigningResult>();
/// ```
///
/// ```compile_fail
/// use mfm_signing::SigningResult;
///
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<SigningResult>();
/// ```
#[derive(PartialEq, Eq)]
pub struct SigningResult {
    signer_ref: SignerRef,
    algorithm: SigningAlgorithmId,
    profile: SigningProfileId,
    public_identity: PublicSigningIdentity,
    signature: SignatureBytes,
}

impl SigningResult {
    /// Creates a signing result and verifies expected identity from the request.
    pub fn for_request(
        request: &SigningRequest,
        public_identity: PublicSigningIdentity,
        signature: SignatureBytes,
    ) -> Result<Self> {
        if &public_identity.algorithm != request.algorithm() {
            return Err(SigningError::PublicIdentityMismatch {
                signer_ref: request.signer_ref().clone(),
                field: "algorithm",
            });
        }
        if let Some(expected) = request.expected_identity() {
            expected.verify(request.signer_ref(), &public_identity)?;
        }
        Ok(Self {
            signer_ref: request.signer_ref().clone(),
            algorithm: request.algorithm().clone(),
            profile: request.profile().clone(),
            public_identity,
            signature,
        })
    }

    /// Returns the signer reference.
    pub fn signer_ref(&self) -> &SignerRef {
        &self.signer_ref
    }

    /// Returns the signing algorithm.
    pub fn algorithm(&self) -> &SigningAlgorithmId {
        &self.algorithm
    }

    /// Returns the signing behavior profile.
    pub fn profile(&self) -> &SigningProfileId {
        &self.profile
    }

    /// Returns public signer metadata.
    pub fn public_identity(&self) -> &PublicSigningIdentity {
        &self.public_identity
    }

    /// Returns signature bytes.
    pub fn signature(&self) -> &SignatureBytes {
        &self.signature
    }
}

impl fmt::Debug for SigningResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningResult")
            .field("signer_ref", &self.signer_ref)
            .field("algorithm", &self.algorithm)
            .field("profile", &self.profile)
            .field("public_identity", &self.public_identity)
            .field("signature", &"<redacted>")
            .finish()
    }
}

/// Closed identifier categories for signing errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningIdentifierKind {
    /// Signer reference.
    SignerRef,
    /// Signing algorithm id.
    Algorithm,
    /// Signing behavior profile id.
    Profile,
    /// Signing domain id.
    Domain,
    /// Signing purpose id.
    Purpose,
    /// Public account id.
    Account,
    /// Concrete signing-provider implementation id.
    Implementation,
}

impl SigningIdentifierKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::SignerRef => "signer_ref",
            Self::Algorithm => "algorithm",
            Self::Profile => "profile",
            Self::Domain => "domain",
            Self::Purpose => "purpose",
            Self::Account => "account",
            Self::Implementation => "implementation",
        }
    }
}

/// Closed validation reasons for public signing identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningValidationError {
    /// Identifier was empty.
    Empty,
    /// Identifier exceeded the maximum length.
    TooLong,
    /// Identifier started with an unsupported character.
    InvalidStart,
    /// Identifier ended with an unsupported character.
    InvalidEnd,
    /// Identifier contained an unsupported character.
    InvalidCharacter,
}

/// Closed signing request validation reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningRequestError {
    /// Public identity did not contain any public identifier.
    MissingPublicIdentity,
    /// A guarded wallet binding omitted its required public account id.
    MissingAccountIdentity,
    /// Public key bytes were empty or too large.
    InvalidPublicKey,
    /// Signature bytes were empty or too large.
    InvalidSignature,
    /// A guarded signer's public descriptor could not be encoded exactly.
    InvalidPublicDescriptor,
}

/// Closed redaction-safe provider failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningProviderError {
    /// Provider failed without exposing path, endpoint, or secret details.
    Failed,
    /// The provider, request, or binder used a different public wallet binding.
    BindingMismatch,
    /// The caller expected a different durable wallet generation.
    GenerationMismatch,
    /// The deployment generation guard could not establish its verdict.
    GenerationGuardUnavailable,
    /// The deployment guard rejected a stale or sibling wallet generation.
    GenerationFenced,
    /// The deployment guard found another direct-sign path to the wallet key.
    DirectSigningOverlap,
}

/// Closed deployment-generation guard failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SigningGenerationGuardError {
    /// The guard could not establish an authoritative current verdict.
    #[error("signer generation guard is unavailable")]
    Unavailable,
    /// The wallet generation was stale or conflicted with a sibling writer.
    #[error("signer generation is fenced")]
    Fenced,
    /// The wallet key was also reachable through a direct-sign path.
    #[error("signer direct-sign exclusion failed")]
    DirectSigningOverlap,
}

/// Redaction-safe signing contract error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    /// A signing identifier failed validation.
    #[error("invalid signing {}: {}", kind.as_str(), validation_reason(*reason))]
    InvalidIdentifier {
        /// Identifier category.
        kind: SigningIdentifierKind,
        /// Closed validation reason.
        reason: SigningValidationError,
    },
    /// A signing request failed validation.
    #[error("{}", request_reason(*reason))]
    InvalidRequest {
        /// Closed request validation reason.
        reason: SigningRequestError,
    },
    /// Returned public identity did not match the request expectation.
    #[error("signer {signer_ref} public identity mismatch for {field}")]
    PublicIdentityMismatch {
        /// Signer reference whose identity did not match.
        signer_ref: SignerRef,
        /// Public metadata field that did not match.
        field: &'static str,
    },
    /// Provider failed without exposing source details.
    #[error("signing provider failed")]
    Provider {
        /// Closed provider failure reason.
        reason: SigningProviderError,
    },
}

impl SigningError {
    /// Builds a redacted provider failure, discarding source details.
    pub fn redacted_provider_failure(_source: impl fmt::Display) -> Self {
        Self::Provider {
            reason: SigningProviderError::Failed,
        }
    }
}

fn checked_local_public_id(
    kind: SigningIdentifierKind,
    value: impl AsRef<str>,
) -> Result<LocalPublicId> {
    LocalPublicId::new(value).map_err(|error| signing_identifier_error(kind, error))
}

fn content_ref_value(reference: &ContentRef) -> Result<CanonicalValue> {
    CanonicalValue::object([
        (
            "content_digest",
            CanonicalValue::String(reference.content_digest().as_str().to_owned()),
        ),
        (
            "schema_id",
            CanonicalValue::String(reference.schema_id().as_str().to_owned()),
        ),
    ])
    .map_err(|_| invalid_public_descriptor())
}

fn invalid_public_descriptor() -> SigningError {
    SigningError::InvalidRequest {
        reason: SigningRequestError::InvalidPublicDescriptor,
    }
}

fn signing_identifier_error(
    kind: SigningIdentifierKind,
    error: CheckedStringError,
) -> SigningError {
    let reason = match error.reason() {
        CheckedStringErrorReason::Empty => SigningValidationError::Empty,
        CheckedStringErrorReason::TooLong { .. }
        | CheckedStringErrorReason::SegmentTooLong { .. } => SigningValidationError::TooLong,
        CheckedStringErrorReason::InvalidStart => SigningValidationError::InvalidStart,
        CheckedStringErrorReason::InvalidEnd => SigningValidationError::InvalidEnd,
        CheckedStringErrorReason::InvalidCharacter { .. }
        | CheckedStringErrorReason::MissingSeparator { .. }
        | CheckedStringErrorReason::EmptySegment
        | CheckedStringErrorReason::ReservedPrefix { .. } => {
            SigningValidationError::InvalidCharacter
        }
    };
    SigningError::InvalidIdentifier { kind, reason }
}

fn validate_bytes_len(len: usize, max: usize, reason: SigningRequestError) -> Result<()> {
    if len == 0 || len > max {
        return Err(SigningError::InvalidRequest { reason });
    }
    Ok(())
}

fn validation_reason(reason: SigningValidationError) -> &'static str {
    match reason {
        SigningValidationError::Empty => "empty",
        SigningValidationError::TooLong => "too long",
        SigningValidationError::InvalidStart => "invalid start",
        SigningValidationError::InvalidEnd => "invalid end",
        SigningValidationError::InvalidCharacter => "invalid character",
    }
}

fn request_reason(reason: SigningRequestError) -> &'static str {
    match reason {
        SigningRequestError::MissingPublicIdentity => "public signer identity is missing",
        SigningRequestError::MissingAccountIdentity => {
            "guarded signer public account identity is missing"
        }
        SigningRequestError::InvalidPublicKey => "public key bytes are invalid",
        SigningRequestError::InvalidSignature => "signature bytes are invalid",
        SigningRequestError::InvalidPublicDescriptor => {
            "guarded signer public descriptor is invalid"
        }
    }
}

#[cfg(test)]
mod zeroization_tests {
    use super::*;
    use zeroize::Zeroize;

    #[test]
    fn secret_and_signature_buffers_use_zeroizing_storage() {
        let mut request = SigningRequest::from_digest(
            SignerRef::new("test-signer").expect("signer"),
            SigningAlgorithmId::new("test.algorithm").expect("algorithm"),
            SigningProfileId::new("test.profile").expect("profile"),
            SigningDomainId::new("test.domain").expect("domain"),
            SigningPurposeId::new("test.purpose").expect("purpose"),
            DigestBytes::from_array([0x42; 32]),
        );
        assert_zeroizing_digest(&request.digest);
        request.digest.zeroize();
        assert_eq!(request.digest(), &[0; 32]);

        let mut signature = SignatureBytes::new(vec![0x24; 65]).expect("signature");
        assert_zeroizing_signature(&signature.0);
        signature.0.zeroize();
        assert!(signature.as_bytes().iter().all(|byte| *byte == 0));
    }

    fn assert_zeroizing_digest(_: &Zeroizing<[u8; 32]>) {}

    fn assert_zeroizing_signature(_: &Zeroizing<Vec<u8>>) {}
}
