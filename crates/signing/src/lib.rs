#![warn(missing_docs)]
//! Generic signer capability contracts.
//!
//! This crate defines the reusable signer-facing boundary for MFM runtime
//! providers. Providers accept typed signing requests and return signatures
//! plus public metadata. Private keys, passwords, endpoint paths, and provider
//! runtime resolution remain outside persisted workflow config and values.
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

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{CapabilityError, CapabilitySpec, SupportRole};
use mfm_ids::{
    CapabilityKind, CapabilityVersion, CheckedStringError, CheckedStringErrorReason,
    DigestAlgorithm, DigestBytes, LocalPublicId,
};
use serde::{Deserialize, Serialize};

/// Result type for signer contracts.
pub type Result<T> = std::result::Result<T, SigningError>;

/// Boxed future returned by signer providers.
pub type SigningFuture<'a> = Pin<Box<dyn Future<Output = Result<SigningResult>> + Send + 'a>>;

const MAX_PUBLIC_KEY_LEN: usize = 4096;
const MAX_SIGNATURE_LEN: usize = 4096;

/// Recoverable secp256k1 signature over a Keccak-256 digest.
pub const SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID: &str = "secp256k1.keccak256.recoverable";
/// Deterministic RFC 6979, recoverable, low-s secp256k1 signing profile.
pub const SECP256K1_RFC6979_LOW_S_PROFILE_ID: &str = "secp256k1.rfc6979.recoverable.low_s.v1";

/// Manual-resolution signing domain id.
pub const MANUAL_RESOLUTION_SIGNING_DOMAIN_ID: &str = "mfm.manual_resolution";
/// Manual-resolution authorization signing purpose id.
pub const MANUAL_RESOLUTION_SIGNING_PURPOSE_ID: &str = "mfm.manual_resolution.authorization.v1";

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

/// Returns the manual-resolution signing domain id.
pub fn manual_resolution_signing_domain_id() -> Result<SigningDomainId> {
    SigningDomainId::new(MANUAL_RESOLUTION_SIGNING_DOMAIN_ID)
}

/// Returns the manual-resolution authorization signing purpose id.
pub fn manual_resolution_signing_purpose_id() -> Result<SigningPurposeId> {
    SigningPurposeId::new(MANUAL_RESOLUTION_SIGNING_PURPOSE_ID)
}

/// Builds a manual-resolution signing request over an already canonical claim digest.
pub fn manual_resolution_signing_request(
    signer_ref: SignerRef,
    algorithm: SigningAlgorithmId,
    profile: SigningProfileId,
    digest: DigestBytes,
) -> Result<SigningRequest> {
    Ok(SigningRequest::from_digest(
        signer_ref,
        algorithm,
        profile,
        manual_resolution_signing_domain_id()?,
        manual_resolution_signing_purpose_id()?,
        digest,
    ))
}

/// Transient signing request. This type is intentionally not serializable.
#[derive(Clone, PartialEq, Eq)]
pub struct SigningRequest {
    signer_ref: SignerRef,
    algorithm: SigningAlgorithmId,
    profile: SigningProfileId,
    domain: SigningDomainId,
    purpose: SigningPurposeId,
    digest: DigestBytes,
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
            digest,
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
    pub fn digest(&self) -> &DigestBytes {
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
#[derive(Clone, PartialEq, Eq)]
pub struct SignatureBytes(Vec<u8>);

impl SignatureBytes {
    /// Creates checked signature bytes.
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
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
#[derive(Clone, PartialEq, Eq)]
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
    /// Public key bytes were empty or too large.
    InvalidPublicKey,
    /// Signature bytes were empty or too large.
    InvalidSignature,
}

/// Closed redaction-safe provider failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningProviderError {
    /// Provider failed without exposing path, endpoint, or secret details.
    Failed,
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
        SigningRequestError::InvalidPublicKey => "public key bytes are invalid",
        SigningRequestError::InvalidSignature => "signature bytes are invalid",
    }
}
