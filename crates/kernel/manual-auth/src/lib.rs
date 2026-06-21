#![warn(missing_docs)]
//! Manual saga resolution authorization contracts.
//!
//! This crate owns the durable claim/proof JSON shape and the verifier boundary
//! for signed manual resolution decisions. It does not load signer runtime
//! sources and does not consult live registries during replay.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use alloy_primitives::{Address, PrimitiveSignature, B256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, RunId, SchemaId, SpecHash};
use mfm_spec::v1 as spec;

/// Persisted manual authorization claim contract version.
pub const MANUAL_AUTHORIZATION_CLAIM_VERSION: &str = "mfm.manual_resolution.authorization_claim.v1";
/// Persisted manual authorization proof contract version.
pub const MANUAL_AUTHORIZATION_PROOF_VERSION: &str = "mfm.manual_resolution.authorization_proof.v1";
/// Supported v1 digest-signature scheme for manual authorization proofs.
pub const MANUAL_RESOLUTION_DIGEST_SIGNATURE_SCHEME: &str =
    "mfm.manual_resolution.digest_signature.v1";
/// Stable schema name for manual authorization proof artifacts.
pub const MANUAL_AUTHORIZATION_PROOF_SCHEMA_NAME: &str =
    "mfm.manual_resolution.authorization_proof";
/// Stable schema version for manual authorization proof artifacts.
pub const MANUAL_AUTHORIZATION_PROOF_SCHEMA_VERSION: &str = "1";
const MAX_MANUAL_SIGNATURE_LEN: usize = 4096;

/// Result type for manual authorization contracts.
pub type Result<T> = std::result::Result<T, ManualAuthorizationError>;

/// Returns the schema id for manual authorization proof artifacts.
pub fn manual_authorization_proof_schema_id() -> Result<SchemaId> {
    SchemaId::new(
        MANUAL_AUTHORIZATION_PROOF_SCHEMA_NAME,
        MANUAL_AUTHORIZATION_PROOF_SCHEMA_VERSION,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(MANUAL_AUTHORIZATION_PROOF_SCHEMA_NAME.as_bytes()),
    )
    .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))
}

/// Verifies a proof using the built-in v1 digest-signature verifier.
pub fn verify_builtin_manual_resolution_authorization(
    policy: &spec::ManualResolutionAuthorizationSpec,
    claim: ManualResolutionAuthorizationClaim,
    proof: ManualResolutionAuthorizationProof,
) -> Result<VerifiedManualResolution> {
    let mut registry = ManualAuthorizationVerifierRegistry::new();
    registry.register(policy.verifier_id.clone(), DigestSignatureVerifier)?;
    registry.verify(policy, claim, proof)
}

/// Manual block reason bound into a signed manual authorization claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManualResolutionBlockReason {
    /// Certified run policy requires operator resolution.
    PolicyManualResolution,
    /// Forward ledger evidence was ambiguous at quiescence.
    ForwardAmbiguous,
    /// Remediation failed non-retryably.
    RemediationFailed,
    /// Remediation ledger evidence was ambiguous.
    RemediationAmbiguous,
}

impl ManualResolutionBlockReason {
    /// Returns the persisted reason tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PolicyManualResolution => "policy_manual_resolution",
            Self::ForwardAmbiguous => "forward_ambiguous",
            Self::RemediationFailed => "remediation_failed",
            Self::RemediationAmbiguous => "remediation_ambiguous",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "policy_manual_resolution" => Ok(Self::PolicyManualResolution),
            "forward_ambiguous" => Ok(Self::ForwardAmbiguous),
            "remediation_failed" => Ok(Self::RemediationFailed),
            "remediation_ambiguous" => Ok(Self::RemediationAmbiguous),
            other => Err(ManualAuthorizationError::InvalidShape(format!(
                "unknown manual block reason {other:?}"
            ))),
        }
    }
}

/// Evidence artifact reference bound into a manual authorization claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionEvidenceRef {
    /// Certified schema id for the evidence artifact.
    pub schema_id: SchemaId,
    /// Canonical content hash of the evidence artifact.
    pub content_hash: ContentDigest,
    /// Artifact id of the evidence artifact.
    pub artifact_id: ArtifactId,
}

impl ManualResolutionEvidenceRef {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "artifact_id": self.artifact_id.as_str(),
            "content_hash": self.content_hash.as_str(),
            "schema_id": self.schema_id.as_str(),
        })
    }
}

/// Authority over the manually blocked run prefix that operators are allowed to resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionPrefixAuthority {
    run_id: RunId,
    spec_hash: SpecHash,
    expected_next_seq: u64,
    stream_prefix_digest: ContentDigest,
    manual_block_reason: ManualResolutionBlockReason,
    unresolved_obligations_digest: ContentDigest,
    manual_policy: spec::ManualResolutionEvidenceSpec,
}

impl ManualResolutionPrefixAuthority {
    /// Creates prefix authority from store/replay-derived prefix facts and certified policy.
    pub fn new(
        run_id: RunId,
        spec_hash: SpecHash,
        expected_next_seq: u64,
        stream_prefix_digest: ContentDigest,
        manual_block_reason: ManualResolutionBlockReason,
        unresolved_obligations_digest: ContentDigest,
        manual_policy: spec::ManualResolutionEvidenceSpec,
    ) -> Result<Self> {
        if expected_next_seq == 0 {
            return Err(ManualAuthorizationError::InvalidShape(
                "manual prefix expected_next_seq must be non-zero".to_owned(),
            ));
        }
        Ok(Self {
            run_id,
            spec_hash,
            expected_next_seq,
            stream_prefix_digest,
            manual_block_reason,
            unresolved_obligations_digest,
            manual_policy,
        })
    }

    /// Returns the run id bound into this prefix.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the certified spec hash bound into this prefix.
    pub const fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Returns the expected sequence for the manual-resolution append.
    pub const fn expected_next_seq(&self) -> u64 {
        self.expected_next_seq
    }

    /// Returns the digest of the event prefix being resolved.
    pub const fn stream_prefix_digest(&self) -> &ContentDigest {
        &self.stream_prefix_digest
    }

    /// Returns the manual block reason derived from the prefix projection.
    pub const fn manual_block_reason(&self) -> ManualResolutionBlockReason {
        self.manual_block_reason
    }

    /// Returns the digest of unresolved obligations at the manual block.
    pub const fn unresolved_obligations_digest(&self) -> &ContentDigest {
        &self.unresolved_obligations_digest
    }

    /// Returns the certified manual policy bound into this prefix.
    pub const fn manual_policy(&self) -> &spec::ManualResolutionEvidenceSpec {
        &self.manual_policy
    }

    /// Builds the unique authorization claim for this prefix, outcome, and evidence artifact.
    pub fn authorization_claim(
        &self,
        outcome: events::ManualResolutionOutcome,
        evidence: ManualResolutionEvidenceRef,
    ) -> Result<ManualResolutionAuthorizationClaim> {
        if evidence.schema_id != self.manual_policy.evidence_schema {
            return Err(ManualAuthorizationError::PolicyMismatch("evidence_schema"));
        }
        Ok(ManualResolutionAuthorizationClaim {
            run_id: self.run_id.clone(),
            spec_hash: self.spec_hash.clone(),
            expected_next_seq: self.expected_next_seq,
            stream_prefix_digest: self.stream_prefix_digest.clone(),
            manual_block_reason: self.manual_block_reason,
            unresolved_obligations_digest: self.unresolved_obligations_digest.clone(),
            outcome,
            evidence,
        })
    }
}

/// Authority over proof bytes and artifact refs for one manual resolution decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionProofAuthority {
    prefix: ManualResolutionPrefixAuthority,
    outcome: events::ManualResolutionOutcome,
    evidence: ManualResolutionEvidenceRef,
    authorization: ManualResolutionEvidenceRef,
    proof_bytes: Vec<u8>,
}

impl ManualResolutionProofAuthority {
    /// Creates proof authority after binding proof bytes to their authorization artifact ref.
    pub fn new(
        prefix: ManualResolutionPrefixAuthority,
        outcome: events::ManualResolutionOutcome,
        evidence: ManualResolutionEvidenceRef,
        authorization: ManualResolutionEvidenceRef,
        proof_bytes: Vec<u8>,
    ) -> Result<Self> {
        verify_authorization_artifact_bytes(&authorization, &proof_bytes)?;
        Ok(Self {
            prefix,
            outcome,
            evidence,
            authorization,
            proof_bytes,
        })
    }

    /// Verifies the proof against the certified prefix authority.
    pub fn verify(self) -> Result<VerifiedManualResolutionForPrefix> {
        let expected_claim = self
            .prefix
            .authorization_claim(self.outcome, self.evidence.clone())?;
        let proof = ManualResolutionAuthorizationProof::from_json_slice(&self.proof_bytes)?;
        let verified = verify_builtin_manual_resolution_authorization(
            &self.prefix.manual_policy.authorization,
            expected_claim,
            proof,
        )?;
        Ok(VerifiedManualResolutionForPrefix {
            prefix: self.prefix,
            outcome: self.outcome,
            evidence: self.evidence,
            authorization: self.authorization,
            proof_bytes: self.proof_bytes,
            verified,
            _seal: sealed::VerifiedManualResolutionForPrefixSeal,
        })
    }
}

/// Verified manual resolution proof for one concrete run prefix.
#[derive(Debug, Clone)]
pub struct VerifiedManualResolutionForPrefix {
    prefix: ManualResolutionPrefixAuthority,
    outcome: events::ManualResolutionOutcome,
    evidence: ManualResolutionEvidenceRef,
    authorization: ManualResolutionEvidenceRef,
    proof_bytes: Vec<u8>,
    verified: VerifiedManualResolution,
    _seal: sealed::VerifiedManualResolutionForPrefixSeal,
}

impl VerifiedManualResolutionForPrefix {
    /// Returns the verified prefix authority.
    pub const fn prefix(&self) -> &ManualResolutionPrefixAuthority {
        &self.prefix
    }

    /// Returns the authorized outcome.
    pub const fn outcome(&self) -> events::ManualResolutionOutcome {
        self.outcome
    }

    /// Returns the manual evidence artifact ref.
    pub const fn evidence(&self) -> &ManualResolutionEvidenceRef {
        &self.evidence
    }

    /// Returns the authorization proof artifact ref.
    pub const fn authorization(&self) -> &ManualResolutionEvidenceRef {
        &self.authorization
    }

    /// Returns the canonical proof bytes that were verified.
    pub fn proof_bytes(&self) -> &[u8] {
        &self.proof_bytes
    }

    /// Returns the verified authorization claim.
    pub const fn claim(&self) -> &ManualResolutionAuthorizationClaim {
        self.verified.claim()
    }

    /// Returns the verified authorization proof.
    pub const fn proof(&self) -> &ManualResolutionAuthorizationProof {
        self.verified.proof()
    }
}

/// Canonical manual-resolution authorization claim signed by operators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionAuthorizationClaim {
    /// Run being resolved.
    pub run_id: RunId,
    /// Certified spec hash for the run.
    pub spec_hash: SpecHash,
    /// Store sequence expected for the manual resolution append.
    pub expected_next_seq: u64,
    /// Digest of the stream prefix ending in `ManualBlocked`.
    pub stream_prefix_digest: ContentDigest,
    /// Derived manual block reason.
    pub manual_block_reason: ManualResolutionBlockReason,
    /// Digest of unresolved obligations at the manual block.
    pub unresolved_obligations_digest: ContentDigest,
    /// Authorized manual resolution outcome.
    pub outcome: events::ManualResolutionOutcome,
    /// Evidence artifact covered by this authorization.
    pub evidence: ManualResolutionEvidenceRef,
}

impl ManualResolutionAuthorizationClaim {
    /// Returns canonical JSON bytes for the claim.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(self.json())
    }

    /// Returns the canonical claim digest operators sign.
    pub fn digest(&self) -> Result<ContentDigest> {
        Ok(self.canonical_json()?.content_digest())
    }

    /// Parses canonical claim JSON bytes.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|error| ManualAuthorizationError::Canonical(error.to_string()))?;
        let value: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
            .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?;
        let parsed = parse_claim(&value)?;
        if parsed.canonical_json()?.as_bytes() != canonical.as_bytes() {
            return Err(ManualAuthorizationError::InvalidShape(
                "manual authorization claim contains unknown fields".to_owned(),
            ));
        }
        Ok(parsed)
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "claim_version": MANUAL_AUTHORIZATION_CLAIM_VERSION,
            "evidence": self.evidence.json(),
            "expected_next_seq": self.expected_next_seq,
            "manual_block_reason": self.manual_block_reason.as_str(),
            "outcome": self.outcome.as_str(),
            "run_id": self.run_id.as_str(),
            "spec_hash": self.spec_hash.as_str(),
            "stream_prefix_digest": self.stream_prefix_digest.as_str(),
            "unresolved_obligations_digest": self.unresolved_obligations_digest.as_str(),
        })
    }
}

/// One operator signature over a manual authorization claim digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionAuthorizationSignature {
    /// Stable operator id from the certified authority snapshot.
    pub operator_id: spec::OperatorId,
    /// Public operator identity from the certified authority snapshot.
    pub public_identity: spec::OperatorPublicIdentity,
    /// Signature bytes over the canonical claim digest.
    pub signature: ManualAuthorizationSignatureBytes,
}

impl ManualResolutionAuthorizationSignature {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "operator_id": self.operator_id.as_str(),
            "public_identity": self.public_identity.as_str(),
            "signature_hex": hex::encode(self.signature.as_bytes()),
        })
    }
}

/// Signature bytes persisted in a manual authorization proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAuthorizationSignatureBytes(Vec<u8>);

impl ManualAuthorizationSignatureBytes {
    /// Creates checked persisted manual authorization signature bytes.
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_MANUAL_SIGNATURE_LEN {
            return Err(ManualAuthorizationError::InvalidShape(
                "manual authorization signature length is invalid".to_owned(),
            ));
        }
        Ok(Self(bytes))
    }

    /// Returns the raw signature bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Persisted manual authorization proof artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionAuthorizationProof {
    /// Verifier id that must validate this proof.
    pub verifier_id: spec::ManualAuthorizationVerifierId,
    /// Signing scheme used by the proof signatures.
    pub signing_scheme: spec::ManualSigningSchemeSpec,
    /// Claim signed by the listed operators.
    pub claim: ManualResolutionAuthorizationClaim,
    /// Operator signatures over [`ManualResolutionAuthorizationClaim::digest`].
    pub signatures: Vec<ManualResolutionAuthorizationSignature>,
}

impl ManualResolutionAuthorizationProof {
    /// Returns canonical JSON bytes for the proof artifact.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(self.json())
    }

    /// Returns the canonical proof artifact digest.
    pub fn content_digest(&self) -> Result<ContentDigest> {
        Ok(self.canonical_json()?.content_digest())
    }

    /// Parses canonical proof JSON bytes.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|error| ManualAuthorizationError::Canonical(error.to_string()))?;
        let value: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
            .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?;
        let parsed = parse_proof(&value)?;
        if parsed.canonical_json()?.as_bytes() != canonical.as_bytes() {
            return Err(ManualAuthorizationError::InvalidShape(
                "manual authorization proof contains unknown fields".to_owned(),
            ));
        }
        Ok(parsed)
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "claim": self.claim.json(),
            "proof_version": MANUAL_AUTHORIZATION_PROOF_VERSION,
            "signatures": self
                .signatures
                .iter()
                .map(ManualResolutionAuthorizationSignature::json)
                .collect::<Vec<_>>(),
            "signing_scheme": self.signing_scheme.as_str(),
            "verifier_id": self.verifier_id.as_str(),
        })
    }
}

/// Manual authorization verifier input.
pub struct ManualAuthorizationVerification<'a> {
    /// Certified authorization policy from the spec/certificate.
    pub policy: &'a spec::ManualResolutionAuthorizationSpec,
    /// Canonical claim expected by runtime or replay.
    pub claim: &'a ManualResolutionAuthorizationClaim,
    /// Persisted authorization proof artifact.
    pub proof: &'a ManualResolutionAuthorizationProof,
}

/// Verifier for one manual authorization proof scheme.
pub trait ManualAuthorizationVerifier: Send + Sync {
    /// Verifies proof signatures and scheme-specific constraints.
    fn verify(&self, verification: ManualAuthorizationVerification<'_>) -> Result<()>;
}

/// Built-in verifier for `mfm.manual_resolution.digest_signature.v1`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DigestSignatureVerifier;

impl ManualAuthorizationVerifier for DigestSignatureVerifier {
    fn verify(&self, verification: ManualAuthorizationVerification<'_>) -> Result<()> {
        if verification.proof.signing_scheme.as_str() != MANUAL_RESOLUTION_DIGEST_SIGNATURE_SCHEME {
            return Err(ManualAuthorizationError::VerificationFailed(
                "unsupported manual digest-signature scheme".to_owned(),
            ));
        }
        let claim_digest = verification.claim.digest()?;
        let signing_hash = B256::from(*claim_digest.digest().as_bytes());
        for signature in &verification.proof.signatures {
            let primitive = primitive_signature_from_bytes(&signature.signature)?;
            let recovered = recover_signing_address(signing_hash, primitive)?;
            let recovered_identity = format!("{recovered:?}");
            if recovered_identity != signature.public_identity.as_str() {
                return Err(ManualAuthorizationError::VerificationFailed(
                    "manual signature signer identity mismatch".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// Registry of manual authorization verifiers.
#[derive(Default)]
pub struct ManualAuthorizationVerifierRegistry {
    verifiers: BTreeMap<String, Arc<dyn ManualAuthorizationVerifier>>,
}

impl ManualAuthorizationVerifierRegistry {
    /// Creates an empty verifier registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a verifier implementation for a certified verifier id.
    pub fn register<V>(
        &mut self,
        verifier_id: spec::ManualAuthorizationVerifierId,
        verifier: V,
    ) -> Result<()>
    where
        V: ManualAuthorizationVerifier + 'static,
    {
        let key = verifier_id.as_str().to_owned();
        if self.verifiers.contains_key(&key) {
            return Err(ManualAuthorizationError::VerifierAlreadyRegistered(
                verifier_id,
            ));
        }
        self.verifiers.insert(key, Arc::new(verifier));
        Ok(())
    }

    /// Verifies a proof against a certified policy and expected claim.
    pub fn verify(
        &self,
        policy: &spec::ManualResolutionAuthorizationSpec,
        claim: ManualResolutionAuthorizationClaim,
        proof: ManualResolutionAuthorizationProof,
    ) -> Result<VerifiedManualResolution> {
        validate_policy_claim_proof(policy, &claim, &proof)?;
        let Some(verifier) = self.verifiers.get(policy.verifier_id.as_str()) else {
            return Err(ManualAuthorizationError::UnknownVerifier(
                policy.verifier_id.clone(),
            ));
        };
        verifier.verify(ManualAuthorizationVerification {
            policy,
            claim: &claim,
            proof: &proof,
        })?;
        Ok(VerifiedManualResolution {
            claim,
            proof,
            _seal: sealed::VerifiedManualResolutionSeal,
        })
    }
}

/// Verified manual resolution authorization.
#[derive(Debug, Clone)]
pub struct VerifiedManualResolution {
    claim: ManualResolutionAuthorizationClaim,
    proof: ManualResolutionAuthorizationProof,
    _seal: sealed::VerifiedManualResolutionSeal,
}

impl VerifiedManualResolution {
    /// Returns the verified claim.
    pub const fn claim(&self) -> &ManualResolutionAuthorizationClaim {
        &self.claim
    }

    /// Returns the verified proof.
    pub const fn proof(&self) -> &ManualResolutionAuthorizationProof {
        &self.proof
    }
}

/// Redaction-safe manual authorization error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManualAuthorizationError {
    /// JSON was not canonical.
    #[error("manual authorization canonical error: {0}")]
    Canonical(String),
    /// Persisted proof or claim shape was invalid.
    #[error("manual authorization shape error: {0}")]
    InvalidShape(String),
    /// Proof does not match the certified policy or expected claim.
    #[error("manual authorization proof does not match certified {0}")]
    PolicyMismatch(&'static str),
    /// Proof did not satisfy certified quorum.
    #[error("manual authorization quorum unsatisfied: required {required}, accepted {accepted}")]
    QuorumUnsatisfied {
        /// Required unique operator signatures.
        required: u32,
        /// Accepted unique operator signatures.
        accepted: usize,
    },
    /// Verifier id is not registered in the process-local verifier registry.
    #[error("manual authorization verifier {0} is not registered")]
    UnknownVerifier(spec::ManualAuthorizationVerifierId),
    /// Verifier id was registered more than once.
    #[error("manual authorization verifier {0} is already registered")]
    VerifierAlreadyRegistered(spec::ManualAuthorizationVerifierId),
    /// Scheme-specific verifier rejected the proof.
    #[error("manual authorization verifier rejected proof: {0}")]
    VerificationFailed(String),
}

fn validate_policy_claim_proof(
    policy: &spec::ManualResolutionAuthorizationSpec,
    claim: &ManualResolutionAuthorizationClaim,
    proof: &ManualResolutionAuthorizationProof,
) -> Result<()> {
    if proof.verifier_id != policy.verifier_id {
        return Err(ManualAuthorizationError::PolicyMismatch("verifier_id"));
    }
    if proof.signing_scheme != policy.signing_scheme {
        return Err(ManualAuthorizationError::PolicyMismatch("signing_scheme"));
    }
    if &proof.claim != claim {
        return Err(ManualAuthorizationError::PolicyMismatch("claim"));
    }
    let authority = policy
        .authority
        .operators
        .iter()
        .map(|operator| {
            (
                operator.operator_id.as_str().to_owned(),
                operator.public_identity.as_str().to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut accepted = BTreeSet::new();
    for signature in &proof.signatures {
        let key = (
            signature.operator_id.as_str().to_owned(),
            signature.public_identity.as_str().to_owned(),
        );
        if !authority.contains(&key) {
            return Err(ManualAuthorizationError::PolicyMismatch("authority"));
        }
        accepted.insert(key);
    }
    let required = policy.quorum.required_signatures();
    if accepted.len() < required as usize {
        return Err(ManualAuthorizationError::QuorumUnsatisfied {
            required,
            accepted: accepted.len(),
        });
    }
    Ok(())
}

fn verify_authorization_artifact_bytes(
    authorization: &ManualResolutionEvidenceRef,
    proof_bytes: &[u8],
) -> Result<()> {
    let authorization_schema_id = manual_authorization_proof_schema_id()?;
    if authorization.schema_id != authorization_schema_id {
        return Err(ManualAuthorizationError::PolicyMismatch(
            "authorization_schema",
        ));
    }
    let content_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(proof_bytes),
    );
    let artifact_id = ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest());
    if authorization.content_hash != content_hash || authorization.artifact_id != artifact_id {
        return Err(ManualAuthorizationError::InvalidShape(
            "manual authorization proof bytes do not match artifact ref".to_owned(),
        ));
    }
    Ok(())
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(&value)
        .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| ManualAuthorizationError::Canonical(error.to_string()))
}

fn parse_claim(value: &serde_json::Value) -> Result<ManualResolutionAuthorizationClaim> {
    let object = json_object(value, "manual authorization claim")?;
    let claim_version = required_str(object, "claim_version")?;
    if claim_version != MANUAL_AUTHORIZATION_CLAIM_VERSION {
        return Err(ManualAuthorizationError::InvalidShape(format!(
            "unsupported manual authorization claim version {claim_version:?}"
        )));
    }
    let expected_next_seq = required_u64(object, "expected_next_seq")?;
    if expected_next_seq == 0 {
        return Err(ManualAuthorizationError::InvalidShape(
            "expected_next_seq must be non-zero".to_owned(),
        ));
    }
    Ok(ManualResolutionAuthorizationClaim {
        run_id: parse_identity(required_str(object, "run_id")?)?,
        spec_hash: parse_identity(required_str(object, "spec_hash")?)?,
        expected_next_seq,
        stream_prefix_digest: parse_identity(required_str(object, "stream_prefix_digest")?)?,
        manual_block_reason: ManualResolutionBlockReason::parse(required_str(
            object,
            "manual_block_reason",
        )?)?,
        unresolved_obligations_digest: parse_identity(required_str(
            object,
            "unresolved_obligations_digest",
        )?)?,
        outcome: parse_outcome(required_str(object, "outcome")?)?,
        evidence: parse_evidence_ref(required(object, "evidence")?)?,
    })
}

fn parse_evidence_ref(value: &serde_json::Value) -> Result<ManualResolutionEvidenceRef> {
    let object = json_object(value, "manual evidence ref")?;
    Ok(ManualResolutionEvidenceRef {
        schema_id: parse_identity(required_str(object, "schema_id")?)?,
        content_hash: parse_identity(required_str(object, "content_hash")?)?,
        artifact_id: parse_identity(required_str(object, "artifact_id")?)?,
    })
}

fn parse_proof(value: &serde_json::Value) -> Result<ManualResolutionAuthorizationProof> {
    let object = json_object(value, "manual authorization proof")?;
    let proof_version = required_str(object, "proof_version")?;
    if proof_version != MANUAL_AUTHORIZATION_PROOF_VERSION {
        return Err(ManualAuthorizationError::InvalidShape(format!(
            "unsupported manual authorization proof version {proof_version:?}"
        )));
    }
    Ok(ManualResolutionAuthorizationProof {
        verifier_id: spec::ManualAuthorizationVerifierId::new(required_str(object, "verifier_id")?)
            .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?,
        signing_scheme: spec::ManualSigningSchemeSpec::new(required_str(object, "signing_scheme")?)
            .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?,
        claim: parse_claim(required(object, "claim")?)?,
        signatures: parse_array(required(object, "signatures")?, parse_signature)?,
    })
}

fn parse_signature(value: &serde_json::Value) -> Result<ManualResolutionAuthorizationSignature> {
    let object = json_object(value, "manual authorization signature")?;
    let signature = hex::decode(required_str(object, "signature_hex")?)
        .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?;
    Ok(ManualResolutionAuthorizationSignature {
        operator_id: spec::OperatorId::new(required_str(object, "operator_id")?)
            .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?,
        public_identity: spec::OperatorPublicIdentity::new(required_str(
            object,
            "public_identity",
        )?)
        .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?,
        signature: ManualAuthorizationSignatureBytes::new(signature)?,
    })
}

fn primitive_signature_from_bytes(
    signature: &ManualAuthorizationSignatureBytes,
) -> Result<PrimitiveSignature> {
    let bytes = signature.as_bytes();
    let raw: &[u8; 65] = bytes.try_into().map_err(|_| {
        ManualAuthorizationError::VerificationFailed(
            "manual authorization signature must be 65 bytes".to_owned(),
        )
    })?;
    PrimitiveSignature::from_raw_array(raw)
        .map(PrimitiveSignature::normalized_s)
        .map_err(|_| {
            ManualAuthorizationError::VerificationFailed(
                "manual authorization signature parity is invalid".to_owned(),
            )
        })
}

fn recover_signing_address(signing_hash: B256, signature: PrimitiveSignature) -> Result<Address> {
    signature
        .recover_address_from_prehash(&signing_hash)
        .map_err(|_| {
            ManualAuthorizationError::VerificationFailed(
                "manual authorization signature recovery failed".to_owned(),
            )
        })
}

fn parse_outcome(value: &str) -> Result<events::ManualResolutionOutcome> {
    match value {
        "confirm_remediated" => Ok(events::ManualResolutionOutcome::ConfirmRemediated),
        "fail_without_acdc_claim" => Ok(events::ManualResolutionOutcome::FailWithoutAcdcClaim),
        other => Err(ManualAuthorizationError::InvalidShape(format!(
            "unknown manual resolution outcome {other:?}"
        ))),
    }
}

fn parse_array<T>(
    value: &serde_json::Value,
    parser: fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    json_array(value, "array")?.iter().map(parser).collect()
}

fn parse_identity<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    value
        .parse::<T>()
        .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))
}

fn json_object<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value.as_object().ok_or_else(|| {
        ManualAuthorizationError::InvalidShape(format!("{context} must be a JSON object"))
    })
}

fn json_array<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a Vec<serde_json::Value>> {
    value.as_array().ok_or_else(|| {
        ManualAuthorizationError::InvalidShape(format!("{context} must be a JSON array"))
    })
}

fn required<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a serde_json::Value> {
    object.get(field).ok_or_else(|| {
        ManualAuthorizationError::InvalidShape(format!("missing required field {field}"))
    })
}

fn required_str<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a str> {
    required(object, field)?
        .as_str()
        .ok_or_else(|| ManualAuthorizationError::InvalidShape(format!("{field} must be a string")))
}

fn required_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u64> {
    required(object, field)?.as_u64().ok_or_else(|| {
        ManualAuthorizationError::InvalidShape(format!("{field} must be an unsigned integer"))
    })
}

mod sealed {
    #[derive(Debug, Clone)]
    pub(super) struct VerifiedManualResolutionSeal;
    #[derive(Debug, Clone)]
    pub(super) struct VerifiedManualResolutionForPrefixSeal;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_ids::{DigestAlgorithm, DigestBytes};

    struct AcceptingVerifier;

    impl ManualAuthorizationVerifier for AcceptingVerifier {
        fn verify(&self, _verification: ManualAuthorizationVerification<'_>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn claim_digest_is_canonical_and_stable() {
        let claim = claim();

        assert_eq!(
            claim.digest().expect("digest").as_str(),
            "content:sha256-jcs-v1:3e2a6ff2fb3b393f9530d4239e223181b863a58839c962629f62027eefcff611"
        );
    }

    #[test]
    fn proof_round_trips_only_canonical_json() {
        let proof = proof();
        let canonical = proof.canonical_json().expect("canonical");

        let parsed = ManualResolutionAuthorizationProof::from_json_slice(canonical.as_bytes())
            .expect("parse");
        assert_eq!(parsed, proof);

        let mut value: serde_json::Value =
            serde_json::from_slice(canonical.as_bytes()).expect("json value");
        value["extra"] = serde_json::json!("field");
        let raw = serde_json::to_vec(&value).expect("json");
        let error = ManualResolutionAuthorizationProof::from_json_slice(&raw)
            .expect_err("non-canonical/unknown field rejects");
        assert!(matches!(
            error,
            ManualAuthorizationError::Canonical(_) | ManualAuthorizationError::InvalidShape(_)
        ));
    }

    #[test]
    fn registry_mints_verified_manual_resolution_after_verifier_accepts() {
        let policy = policy();
        let claim = claim();
        let proof = proof();
        let mut registry = ManualAuthorizationVerifierRegistry::new();
        registry
            .register(policy.verifier_id.clone(), AcceptingVerifier)
            .expect("register verifier");

        let verified = registry
            .verify(&policy, claim.clone(), proof)
            .expect("verified");

        assert_eq!(verified.claim(), &claim);
    }

    #[test]
    fn registry_rejects_policy_and_quorum_mismatches_before_verifier() {
        let policy = policy();
        let mut missing_quorum_proof = proof();
        missing_quorum_proof.signatures.clear();
        let registry = ManualAuthorizationVerifierRegistry::new();

        let error = registry
            .verify(&policy, claim(), missing_quorum_proof)
            .expect_err("quorum rejects");

        assert!(matches!(
            error,
            ManualAuthorizationError::QuorumUnsatisfied { .. }
        ));

        let mut proof = proof();
        proof.verifier_id = spec::ManualAuthorizationVerifierId::new("mfm.manual_auth.test.other")
            .expect("verifier id");
        let error = registry
            .verify(&policy, claim(), proof)
            .expect_err("verifier mismatch rejects");
        assert_eq!(
            error,
            ManualAuthorizationError::PolicyMismatch("verifier_id")
        );
    }

    #[test]
    fn proof_authority_verifies_canonical_proof_bytes_for_prefix() {
        let policy = policy();
        let claim = claim();
        let manual_policy = spec::ManualResolutionEvidenceSpec {
            evidence_schema: claim.evidence.schema_id.clone(),
            authorization: policy.clone(),
        };
        let prefix = ManualResolutionPrefixAuthority::new(
            claim.run_id.clone(),
            claim.spec_hash.clone(),
            claim.expected_next_seq,
            claim.stream_prefix_digest.clone(),
            claim.manual_block_reason,
            claim.unresolved_obligations_digest.clone(),
            manual_policy,
        )
        .expect("prefix authority");
        let proof = signed_proof(&policy, claim.clone());
        let proof_bytes = proof.canonical_json().expect("canonical proof").to_vec();
        let authorization = authorization_ref(&proof_bytes);

        let verified = ManualResolutionProofAuthority::new(
            prefix,
            claim.outcome,
            claim.evidence.clone(),
            authorization,
            proof_bytes,
        )
        .expect("proof authority")
        .verify()
        .expect("verified for prefix");

        assert_eq!(verified.claim(), &claim);
        assert_eq!(verified.proof(), &proof);
    }

    fn proof() -> ManualResolutionAuthorizationProof {
        let policy = policy();
        let operator = policy.authority.operators[0].clone();
        ManualResolutionAuthorizationProof {
            verifier_id: policy.verifier_id,
            signing_scheme: policy.signing_scheme,
            claim: claim(),
            signatures: vec![ManualResolutionAuthorizationSignature {
                operator_id: operator.operator_id,
                public_identity: operator.public_identity,
                signature: ManualAuthorizationSignatureBytes::new(vec![0xab; 64])
                    .expect("signature"),
            }],
        }
    }

    fn policy() -> spec::ManualResolutionAuthorizationSpec {
        spec::ManualResolutionAuthorizationSpec {
            verifier_id: spec::ManualAuthorizationVerifierId::new("mfm.manual_auth.test.verifier")
                .expect("verifier id"),
            signing_scheme: spec::ManualSigningSchemeSpec::new(
                "mfm.manual_resolution.digest_signature.v1",
            )
            .expect("signing scheme"),
            authority: spec::OperatorAuthoritySnapshotSpec {
                authority_id: spec::OperatorAuthorityId::new("mfm.manual_auth.test.authority")
                    .expect("authority id"),
                operators: vec![spec::OperatorAuthorityMemberSpec {
                    operator_id: spec::OperatorId::new("operator.manual-auth")
                        .expect("operator id"),
                    public_identity: spec::OperatorPublicIdentity::new(
                        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                    )
                    .expect("public identity"),
                }],
            },
            quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
        }
    }

    fn claim() -> ManualResolutionAuthorizationClaim {
        ManualResolutionAuthorizationClaim {
            run_id: run_id(0x11),
            spec_hash: SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x12)),
            expected_next_seq: 9,
            stream_prefix_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x13),
            ),
            manual_block_reason: ManualResolutionBlockReason::PolicyManualResolution,
            unresolved_obligations_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x14),
            ),
            outcome: events::ManualResolutionOutcome::ConfirmRemediated,
            evidence: ManualResolutionEvidenceRef {
                schema_id: SchemaId::new(
                    "mfm.manual_auth.test.evidence",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x15),
                )
                .expect("schema"),
                content_hash: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x16),
                ),
                artifact_id: ArtifactId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x16),
                ),
            },
        }
    }

    fn run_id(byte: u8) -> RunId {
        RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(byte))
    }

    fn signed_proof(
        policy: &spec::ManualResolutionAuthorizationSpec,
        claim: ManualResolutionAuthorizationClaim,
    ) -> ManualResolutionAuthorizationProof {
        let operator = policy.authority.operators[0].clone();
        let claim_digest = claim.digest().expect("claim digest");
        ManualResolutionAuthorizationProof {
            verifier_id: policy.verifier_id.clone(),
            signing_scheme: policy.signing_scheme.clone(),
            claim,
            signatures: vec![ManualResolutionAuthorizationSignature {
                operator_id: operator.operator_id,
                public_identity: operator.public_identity,
                signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                    &test_manual_signing_key(),
                    claim_digest.digest().as_bytes(),
                ))
                .expect("signature"),
            }],
        }
    }

    fn authorization_ref(proof_bytes: &[u8]) -> ManualResolutionEvidenceRef {
        let content_hash = ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(proof_bytes),
        );
        ManualResolutionEvidenceRef {
            schema_id: manual_authorization_proof_schema_id().expect("authorization schema"),
            artifact_id: ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest()),
            content_hash,
        }
    }

    fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
        let mut key_bytes = [0u8; 32];
        key_bytes[31] = 1;
        let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
        k256::ecdsa::SigningKey::from(&secret_key)
    }

    fn sign_manual_claim_digest(
        signing_key: &k256::ecdsa::SigningKey,
        digest: &[u8; 32],
    ) -> Vec<u8> {
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(digest)
            .expect("manual signature");
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(u8::from(recovery_id.is_y_odd()));
        signature_bytes
    }

    fn digest_byte(byte: u8) -> DigestBytes {
        DigestBytes::from_array([byte; 32])
    }
}
