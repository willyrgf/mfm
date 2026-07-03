use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
use mfm_facts::{
    FactQueryEvidence, FactQueryReceipt, StoreIdentity, StoreKeyId,
    StoreReceiptAuthenticationScheme,
};
use mfm_ids::ContentDigest;

use super::{Result, StoreError};

/// Public trust root for fact-query receipt authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryReceiptTrustRoot {
    store_identity: StoreIdentity,
    scheme: StoreReceiptAuthenticationScheme,
    key_id: StoreKeyId,
    verifying_key: [u8; 32],
}

impl FactQueryReceiptTrustRoot {
    /// Creates a receipt trust root and validates the public verifying key.
    pub fn new(
        store_identity: StoreIdentity,
        scheme: StoreReceiptAuthenticationScheme,
        key_id: StoreKeyId,
        verifying_key: [u8; 32],
    ) -> Result<Self> {
        match scheme {
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1 => {
                VerifyingKey::from_bytes(&verifying_key)
                    .map_err(|_| receipt_authentication_error("invalid Ed25519 verifying key"))?;
            }
        }
        Ok(Self {
            store_identity,
            scheme,
            key_id,
            verifying_key,
        })
    }

    /// Returns the store identity bound to this trust root.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the authentication scheme bound to this trust root.
    pub const fn scheme(&self) -> StoreReceiptAuthenticationScheme {
        self.scheme
    }

    /// Returns the non-secret key id bound to this trust root.
    pub const fn key_id(&self) -> &StoreKeyId {
        &self.key_id
    }

    /// Returns the raw Ed25519 verifying key bytes.
    pub const fn verifying_key(&self) -> &[u8; 32] {
        &self.verifying_key
    }
}

/// Returns the canonical signing message for a fact-query receipt authentication record.
pub fn fact_query_receipt_authentication_message(
    store_identity: &StoreIdentity,
    scheme: StoreReceiptAuthenticationScheme,
    key_id: &StoreKeyId,
    store_receipt_hash: &ContentDigest,
) -> Result<CanonicalJsonBytes> {
    let value = CanonicalValue::object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-receipt-authentication.v1".to_owned()),
        ),
        (
            "store_identity",
            CanonicalValue::String(store_identity.as_str().to_owned()),
        ),
        ("scheme", CanonicalValue::String(scheme.as_str().to_owned())),
        ("key_id", CanonicalValue::String(key_id.as_str().to_owned())),
        (
            "store_receipt_hash",
            CanonicalValue::String(store_receipt_hash.as_str().to_owned()),
        ),
    ])
    .map_err(|error| receipt_authentication_error(error.to_string()))?;
    Ok(CanonicalJsonBytes::from_value(&value))
}

/// Verifies a fact-query receipt against a store-owned receipt trust root.
pub fn verify_fact_query_receipt_authentication(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
    trust_root: &FactQueryReceiptTrustRoot,
) -> Result<()> {
    let auth = receipt.store_receipt_authentication();
    if auth.store_identity() != trust_root.store_identity() {
        return Err(receipt_authentication_error("store identity mismatch"));
    }
    if auth.scheme() != trust_root.scheme() {
        return Err(receipt_authentication_error(
            "authentication scheme mismatch",
        ));
    }
    let key_id = auth
        .key_id()
        .ok_or_else(|| receipt_authentication_error("missing receipt key id"))?;
    if key_id != trust_root.key_id() {
        return Err(receipt_authentication_error("receipt key id mismatch"));
    }
    let body_hash = mfm_facts::fact_query_receipt_body_hash(plan_hash, receipt)
        .map_err(|error| receipt_authentication_error(error.to_string()))?;
    if &body_hash != receipt.store_receipt_hash() {
        return Err(receipt_authentication_error("receipt body hash mismatch"));
    }
    let signature_bytes: [u8; 64] = auth
        .signature_or_mac()
        .try_into()
        .map_err(|_| receipt_authentication_error("invalid Ed25519 signature length"))?;
    let signature = Signature::from_bytes(&signature_bytes);
    let verifying_key = VerifyingKey::from_bytes(trust_root.verifying_key())
        .map_err(|_| receipt_authentication_error("invalid Ed25519 verifying key"))?;
    let message = fact_query_receipt_authentication_message(
        auth.store_identity(),
        auth.scheme(),
        key_id,
        receipt.store_receipt_hash(),
    )?;
    verifying_key
        .verify(message.as_bytes(), &signature)
        .map_err(|_| receipt_authentication_error("invalid receipt signature"))
}

/// Validates query evidence and verifies its receipt authentication against a trust root.
pub fn validate_fact_query_evidence_recording(
    evidence: &FactQueryEvidence,
    trust_root: &FactQueryReceiptTrustRoot,
) -> Result<()> {
    mfm_facts::validate_fact_query_evidence(evidence)
        .map_err(|error| receipt_authentication_error(error.to_string()))?;
    let plan_hash = mfm_facts::fact_query_plan_hash(evidence.plan())
        .map_err(|error| receipt_authentication_error(error.to_string()))?;
    verify_fact_query_receipt_authentication(&plan_hash, evidence.receipt(), trust_root)
}

fn receipt_authentication_error(message: impl Into<String>) -> StoreError {
    StoreError::ReceiptAuthentication {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use mfm_facts::{DescriptorCatalogWatermark, FactProjectionGeneration, StoreCommitWatermark};
    use mfm_facts::{
        FactAudience, FactFieldId, FactOrderingName, FactOrderingPolicy, FactOrderingTerm,
        FactQueryEvidence, FactQueryScope, FactSelectionEvidence, FactVisibilityScope,
        NullOrdering, ScopeDecisionEvidence, SortDirection, StoreReadFrontier, StoreScopeRef,
    };
    use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

    use super::*;

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    fn plan() -> mfm_facts::CanonicalFactQueryPlan {
        let descriptor = mfm_facts::FactDescriptor::new(
            mfm_facts::FactKind::new("chain.head").expect("kind"),
            mfm_facts::fact_descriptor_schema_id().expect("descriptor schema"),
            mfm_ids::SchemaId::new(
                "mfm.fact.test.subject",
                "v1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x41; 32]),
            )
            .expect("subject schema"),
            mfm_ids::SchemaId::new(
                "mfm.fact.test.response",
                "v1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x42; 32]),
            )
            .expect("response schema"),
            vec![
                mfm_facts::FactFieldDescriptor::new(
                    FactFieldId::new("subject.source").expect("field"),
                    mfm_facts::FactFieldPath::new("subject.source").expect("path"),
                    mfm_facts::FactFieldValueType::String,
                    mfm_facts::FactFieldExtraction::SubjectPath(
                        mfm_facts::CanonicalValuePath::new("source").expect("subject path"),
                    ),
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::QueryOnly,
                    None,
                    None,
                    false,
                    true,
                )
                .expect("source field"),
                mfm_facts::FactFieldDescriptor::new(
                    FactFieldId::new("result.height").expect("field"),
                    mfm_facts::FactFieldPath::new("result.height").expect("path"),
                    mfm_facts::FactFieldValueType::UnsignedInteger,
                    mfm_facts::FactFieldExtraction::ResponsePath(
                        mfm_facts::CanonicalValuePath::new("height").expect("response path"),
                    ),
                    vec![mfm_facts::FactQueryOperator::Equal],
                    mfm_facts::FactFieldExposure::Returnable,
                    None,
                    None,
                    true,
                    true,
                )
                .expect("height field"),
            ],
            vec![FactOrderingPolicy::new(
                FactOrderingName::new("result.height.desc").expect("ordering"),
                vec![FactOrderingTerm::new(
                    FactFieldId::new("result.height").expect("field"),
                    SortDirection::Descending,
                    NullOrdering::Last,
                    false,
                )],
            )
            .expect("ordering")],
        )
        .expect("descriptor");
        let input = mfm_facts::FactQueryInput::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(digest(2)),
            Vec::new(),
            vec![mfm_facts::FactQueryReturnField::new(
                FactFieldId::new("result.height").expect("field"),
            )],
            FactOrderingName::new("result.height.desc").expect("ordering"),
            Some(10),
        )
        .expect("query input");
        mfm_facts::compile_fact_query_plan(&descriptor, input).expect("plan")
    }

    fn trust_root(key: &SigningKey) -> FactQueryReceiptTrustRoot {
        FactQueryReceiptTrustRoot::new(
            StoreIdentity::new("store.default").expect("store identity"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            StoreKeyId::new("key.default").expect("key id"),
            key.verifying_key().to_bytes(),
        )
        .expect("trust root")
    }

    fn signed_receipt(
        plan_hash: &ContentDigest,
        key: &SigningKey,
        store_identity: StoreIdentity,
        key_id: StoreKeyId,
    ) -> FactQueryReceipt {
        let frontier = StoreReadFrontier::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            DescriptorCatalogWatermark::new(1),
            FactProjectionGeneration::new(1),
            0,
            StoreCommitWatermark::new(0),
        );
        let rows: [mfm_facts::FactQueryResultRow; 0] = [];
        let material = mfm_facts::FactQueryReceiptMaterial::from_rows(
            plan_hash,
            frontier,
            mfm_facts::StoreReadFrontierType::Snapshot,
            &rows,
            false,
            None,
        )
        .expect("receipt material");
        let message = fact_query_receipt_authentication_message(
            &store_identity,
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            &key_id,
            material.store_receipt_hash(),
        )
        .expect("message");
        let auth = mfm_facts::StoreReceiptAuthentication::new(
            store_identity,
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(key_id),
            key.sign(message.as_bytes()).to_bytes().to_vec(),
        )
        .expect("auth");
        material.into_receipt(auth)
    }

    #[test]
    fn verifies_ed25519_receipt_authentication() {
        let key = signing_key();
        let plan = plan();
        let plan_hash = mfm_facts::fact_query_plan_hash(&plan).expect("plan hash");
        let receipt = signed_receipt(
            &plan_hash,
            &key,
            StoreIdentity::new("store.default").expect("store"),
            StoreKeyId::new("key.default").expect("key"),
        );
        verify_fact_query_receipt_authentication(&plan_hash, &receipt, &trust_root(&key))
            .expect("verified receipt");

        let evidence = FactQueryEvidence::new(
            plan,
            receipt,
            FactSelectionEvidence::new(digest(4), Vec::new(), None).expect("selection"),
        );
        mfm_facts::fact_query_evidence_hash(&evidence).expect("evidence hash");
    }

    #[test]
    fn validates_query_evidence_recording_against_trust_root() {
        let key = signing_key();
        let plan = plan();
        let plan_hash = mfm_facts::fact_query_plan_hash(&plan).expect("plan hash");
        let receipt = signed_receipt(
            &plan_hash,
            &key,
            StoreIdentity::new("store.default").expect("store"),
            StoreKeyId::new("key.default").expect("key"),
        );
        let evidence = FactQueryEvidence::new(
            plan.clone(),
            receipt.clone(),
            FactSelectionEvidence::new(digest(4), Vec::new(), None).expect("selection"),
        );
        validate_fact_query_evidence_recording(&evidence, &trust_root(&key))
            .expect("validated evidence recording");

        let tampered = FactQueryEvidence::new(
            plan,
            receipt_with_invalid_signature(&plan_hash, &receipt),
            FactSelectionEvidence::new(digest(4), Vec::new(), None).expect("selection"),
        );
        assert!(validate_fact_query_evidence_recording(&tampered, &trust_root(&key)).is_err());
    }

    #[test]
    fn rejects_tampered_receipt_authentication() {
        let key = signing_key();
        let plan_hash = mfm_facts::fact_query_plan_hash(&plan()).expect("plan hash");
        let receipt = signed_receipt(
            &plan_hash,
            &key,
            StoreIdentity::new("store.default").expect("store"),
            StoreKeyId::new("key.default").expect("key"),
        );

        let tampered_hash = receipt_with_invalid_signature(&plan_hash, &receipt);
        assert!(verify_fact_query_receipt_authentication(
            &plan_hash,
            &tampered_hash,
            &trust_root(&key)
        )
        .is_err());

        let wrong_root = FactQueryReceiptTrustRoot::new(
            StoreIdentity::new("store.default").expect("store"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            StoreKeyId::new("key.other").expect("key"),
            key.verifying_key().to_bytes(),
        )
        .expect("wrong root");
        assert!(
            verify_fact_query_receipt_authentication(&plan_hash, &receipt, &wrong_root).is_err()
        );
    }

    fn receipt_with_invalid_signature(
        plan_hash: &ContentDigest,
        receipt: &FactQueryReceipt,
    ) -> FactQueryReceipt {
        let rows = mfm_facts::fact_query_result_rows_from_receipt(receipt);
        let material = mfm_facts::FactQueryReceiptMaterial::from_rows(
            plan_hash,
            receipt.read_frontier().clone(),
            receipt.frontier_type(),
            &rows,
            receipt.returned_field_summaries().is_some(),
            None,
        )
        .expect("receipt material");
        let auth = mfm_facts::StoreReceiptAuthentication::new(
            StoreIdentity::new("store.default").expect("store"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(StoreKeyId::new("key.default").expect("key")),
            vec![0x44; 64],
        )
        .expect("tampered auth");
        material.into_receipt(auth)
    }
}
