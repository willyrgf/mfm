use mfm_evm::*;
use mfm_values::canonicalize_mfm_value;
use mfm_values::{DiagnosticEvidence, Object};

// Persisting a transaction failure must retain nested provider causes and supplied diagnostic
// text, including text outside ordinary value rules.
#[test]
fn causal_contracts_preserve_diagnostic_text_through_whole_owner_admission() {
    let diagnostics = DiagnosticEvidence::from_value(serde_json::json!({
        "response": {
            "status": 599,
            "rpc_code": i64::MIN,
            "message": "provider reports missing api_key marker",
            "data_json": "{\"ratio\":1.2300e-4}",
        },
        "sources": [{"message": "source detail".repeat(1024)}],
    }));
    let maximum = EvmU256::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    let anchor = EvmBlockAnchor {
        number: maximum,
        hash: EvmHash::from_bytes([255; 32]),
    };
    let source = ProviderFailure {
        method: EvmRpcMethod::GetBlockByNumber,
        stage: RpcStage::Validation,
        failure: ProviderFailureKind::Rejected {
            field: RpcField::Block,
            cause: RpcRejection::AnchorMismatch {
                expected: anchor.clone(),
                observed: EvmBlockAnchor {
                    number: anchor.number,
                    hash: EvmHash::from_bytes([254; 32]),
                },
            },
        },
        diagnostics,
    };
    let error = EvmTransactionOperationalError::Provider {
        operation: TransactionProviderOperation::CanonicalBlock,
        cause: EvmOperationalError::new(EvmOperationalKind::Unavailable, source),
    };
    let bytes = canonicalize_mfm_value(&error).unwrap().0;
    assert_eq!(
        serde_json::from_slice::<EvmTransactionOperationalError>(bytes.as_bytes()).unwrap(),
        error
    );
    assert!(std::error::Error::source(&error)
        .unwrap()
        .source()
        .is_some());
    let object = Object::from_value(&error).unwrap();
    assert_eq!(
        object.decode::<EvmTransactionOperationalError>().unwrap(),
        error
    );
}

// Frozen schema identities distinguish the current persisted provider and transaction error
// contracts.
#[test]
fn diagnostic_owner_schema_identity() {
    use mfm_values::MfmValue;
    for (schema, identity, bytes) in [
        (EvmOperationalError::schema_descriptor().unwrap(), "schema:mfm.evm-operational-error:3:sha256-jcs-v1:40cb1920449b8270da4ef453fc654762a18e7165305cd402e716ef5bfd08fd95", 12756),
        (EvmTransactionOperationalError::schema_descriptor().unwrap(), "schema:mfm.evm-transaction-operational-error:4:sha256-jcs-v1:cc11620d29b5503c3bdea1a162847b3522d71d64fd26228c0825395f14739985", 14359),
    ] {
        assert_eq!(schema.schema_id().unwrap().to_string(), identity);
        assert_eq!(schema.identity_canonical_json().unwrap().as_bytes().len(), bytes);
    }
}

// Authority and signer failures must retain their diagnostics and distinct recovery
// classifications while rejecting obsolete schema identities.
#[test]
fn authority_and_signer_owners_keep_diagnostic_admission_and_reject_old_contracts() {
    use mfm_program::ClassifyError;
    for authority in [true, false] {
        let owner = |cause| {
            if authority {
                EvmTransactionOperationalError::AuthorityUnavailable { cause }
            } else {
                EvmTransactionOperationalError::SignerUnavailable { cause }
            }
        };
        let error = owner(DiagnosticEvidence::from_value(serde_json::json!({
            "operation": "execution fixture", "message": "dependency mentions private_key marker",
            "sources": [{"message": "root"}, {"message": "leaf"}]
        })));
        let object = Object::from_value(&error).unwrap();
        assert_eq!(
            object.decode::<EvmTransactionOperationalError>().unwrap(),
            error
        );
        assert_eq!(
            error.classify(),
            if authority {
                mfm_program::Classification::OutcomeUnknown
            } else {
                mfm_program::Classification::Retryable
            }
        );
        assert!(Object::from_value(&owner(DiagnosticEvidence::from_value(
            serde_json::json!({"ratio": 0.5})
        )))
        .is_err());
        let old_ref = mfm_ids::ContentRef::new(
            mfm_ids::SchemaId::parse("schema:mfm.evm-transaction-operational-error:2:sha256-jcs-v1:6293c5ceeb0e9cc6329dfe21ea5d4001f9efce5114878ea77a6a0503b109ce45").unwrap(),
            object.value_ref().content_digest().clone(),
        ).unwrap();
        let old_object = Object::from_canonical(old_ref, object.canonical_bytes()).unwrap();
        assert!(old_object
            .decode::<EvmTransactionOperationalError>()
            .is_err());
    }
}

#[test]
fn scalar_rejection_survives_the_complete_provider_original_with_its_classification() {
    use mfm_program::{Classification, ClassifyError};
    use mfm_values::Unsigned256Error;
    let scalar = EvmU256::new("01").unwrap_err();
    let error = EvmTransactionOperationalError::Provider {
        operation: TransactionProviderOperation::CanonicalBlock,
        cause: EvmOperationalError::new(
            EvmOperationalKind::Unavailable,
            ProviderFailure {
                method: EvmRpcMethod::GetBlockByNumber,
                stage: RpcStage::Validation,
                failure: ProviderFailureKind::Rejected {
                    field: RpcField::Block,
                    cause: RpcRejection::Domain { cause: scalar },
                },
                diagnostics: DiagnosticEvidence::from_value(
                    serde_json::json!({"operation": "decode_block_number"}),
                ),
            },
        ),
    };
    let object = Object::from_value(&error).unwrap();
    let cold = object.decode::<EvmTransactionOperationalError>().unwrap();
    assert_eq!(cold, error);
    assert_eq!(cold.classify(), Classification::OutcomeUnknown);
    let mut source: &(dyn std::error::Error + 'static) = &cold;
    while let Some(next) = source.source() {
        source = next;
    }
    assert_eq!(source.downcast_ref(), Some(&Unsigned256Error::NonCanonical));
    // The former descriptor lacks the constructor cause and transaction-presence method.
    // Exact admission must not fall back to a familiar type name.
    let previous = mfm_ids::SchemaId::parse(
        "schema:mfm.evm-transaction-operational-error:3:sha256-jcs-v1:2292902eaf07f4fe168ab33b496a1eb0913203f3010c41db59fe0ce9d0c56a81",
    ).unwrap();
    let old_ref =
        mfm_ids::ContentRef::new(previous, object.value_ref().content_digest().clone()).unwrap();
    let old = Object::from_canonical(old_ref, object.canonical_bytes()).unwrap();
    assert!(old.decode::<EvmTransactionOperationalError>().is_err());
}
