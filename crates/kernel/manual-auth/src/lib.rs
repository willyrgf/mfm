#![warn(missing_docs)]
//! Manual saga resolution authorization contracts.
//!
//! This crate owns the durable claim/proof JSON shape and the verifier boundary
//! for signed manual resolution decisions. It does not load signer runtime
//! sources and does not consult live registries during replay.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, RunId, SchemaId, SpecHash};
use mfm_signing::SignatureBytes;
use mfm_spec::v1 as spec;

/// Persisted manual authorization claim contract version.
pub const MANUAL_AUTHORIZATION_CLAIM_VERSION: &str = "mfm.manual_resolution.authorization_claim.v1";
/// Persisted manual authorization proof contract version.
pub const MANUAL_AUTHORIZATION_PROOF_VERSION: &str = "mfm.manual_resolution.authorization_proof.v1";

/// Result type for manual authorization contracts.
pub type Result<T> = std::result::Result<T, ManualAuthorizationError>;

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
    pub signature: SignatureBytes,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualAuthorizationError {
    /// JSON was not canonical.
    Canonical(String),
    /// Persisted proof or claim shape was invalid.
    InvalidShape(String),
    /// Proof does not match the certified policy or expected claim.
    PolicyMismatch(&'static str),
    /// Proof did not satisfy certified quorum.
    QuorumUnsatisfied {
        /// Required unique operator signatures.
        required: u32,
        /// Accepted unique operator signatures.
        accepted: usize,
    },
    /// Verifier id is not registered in the process-local verifier registry.
    UnknownVerifier(spec::ManualAuthorizationVerifierId),
    /// Verifier id was registered more than once.
    VerifierAlreadyRegistered(spec::ManualAuthorizationVerifierId),
    /// Scheme-specific verifier rejected the proof.
    VerificationFailed(String),
}

impl fmt::Display for ManualAuthorizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canonical(message) => {
                write!(f, "manual authorization canonical error: {message}")
            }
            Self::InvalidShape(message) => {
                write!(f, "manual authorization shape error: {message}")
            }
            Self::PolicyMismatch(field) => {
                write!(
                    f,
                    "manual authorization proof does not match certified {field}"
                )
            }
            Self::QuorumUnsatisfied { required, accepted } => write!(
                f,
                "manual authorization quorum unsatisfied: required {required}, accepted {accepted}"
            ),
            Self::UnknownVerifier(verifier_id) => {
                write!(
                    f,
                    "manual authorization verifier {verifier_id} is not registered"
                )
            }
            Self::VerifierAlreadyRegistered(verifier_id) => write!(
                f,
                "manual authorization verifier {verifier_id} is already registered"
            ),
            Self::VerificationFailed(message) => {
                write!(f, "manual authorization verifier rejected proof: {message}")
            }
        }
    }
}

impl std::error::Error for ManualAuthorizationError {}

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
        signature: SignatureBytes::new(signature)
            .map_err(|error| ManualAuthorizationError::InvalidShape(error.to_string()))?,
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
                signature: SignatureBytes::new(vec![0xab; 64]).expect("signature"),
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
                        "operator-manual-auth-public",
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

    fn digest_byte(byte: u8) -> DigestBytes {
        DigestBytes::from_array([byte; 32])
    }
}
