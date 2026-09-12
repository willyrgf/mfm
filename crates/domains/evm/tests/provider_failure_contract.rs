use mfm_diagnostics::*;
use mfm_evm::*;
use mfm_values::{canonicalize_mfm_value, MfmValue};

#[test]
fn causal_contracts_preserve_bounded_evidence_through_exact_descriptors() {
    #[derive(Debug)]
    struct Cycle;
    impl std::fmt::Display for Cycle {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("no source formatting");
        }
    }
    impl std::error::Error for Cycle {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(self)
        }
    }
    let diagnostics = DiagnosticEvidence::capture(
        Some((
            ResponseContext::new(HttpStatusCode::new(599).unwrap(), Some(i64::MIN)),
            vec![(OmittedField::Body, OmissionReason::Withheld, Some(u64::MAX)); 32],
        )),
        Some(&Cycle),
        ChainEnd::Complete,
        |_| {
            (
                SourceLayer::new(
                    SourceKind::Parse,
                    vec![
                        SourceFact::Parse {
                            category: ParseCategory::Syntax,
                            location: ParseLocation::LineColumn {
                                line: u64::MAX,
                                column: u64::MAX,
                            },
                        },
                        SourceFact::Size {
                            limit: u64::MAX,
                            observed: ObservedSize::AtLeast { value: u64::MAX },
                        },
                    ],
                    false,
                )
                .unwrap(),
                vec![],
            )
        },
    );
    let diagnostics_bytes = canonicalize_mfm_value(&diagnostics)
        .unwrap()
        .0
        .as_bytes()
        .len();
    assert!((7800..=8192).contains(&diagnostics_bytes));
    assert_eq!(diagnostics.sources().end(), ChainEnd::BoundReached);
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
    for (descriptor, expected, expected_bytes) in [
        (EvmOperationalError::schema_descriptor().unwrap(),
            "schema:mfm.evm-operational-error:2:sha256-jcs-v1:c68ccfcced460b7b8bac450fedf549a9710140ef820af7e884e99630ce93a53d", 19296),
        (EvmTransactionOperationalError::schema_descriptor().unwrap(),
            "schema:mfm.evm-transaction-operational-error:2:sha256-jcs-v1:684b0846a3f6e2510c1ec265852db0ee50e0ed6e816d86d3ba7f4b7d152c3eec", 20590),
    ] {
        assert_eq!(descriptor.schema_id().unwrap().to_string(), expected);
        assert_eq!(descriptor.identity_canonical_json().unwrap().as_bytes().len(), expected_bytes);
    }
}
