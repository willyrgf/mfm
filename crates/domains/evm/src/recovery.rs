//! Bounded operational causes and intrinsic recovery classifications.

use super::*;

/// Reviewed operational cause during transaction preparation or reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "transaction-operational-error",
    version = "2",
    schema = "mfm.evm-transaction-operational-error"
)]
pub enum EvmTransactionOperationalError {
    /// The EVM provider did not produce the required observation.
    #[error("transaction provider failed")]
    Provider {
        /// Originating transaction provider operation.
        operation: TransactionProviderOperation,
        /// Closed provider cause without response text.
        #[source]
        cause: EvmOperationalError,
    },
    /// Transaction authority did not acknowledge the requested operation.
    #[error("transaction authority unavailable")]
    AuthorityUnavailable,
    /// The signer could not produce a signature.
    #[error("transaction signer unavailable")]
    SignerUnavailable,
}

impl mfm_program::ClassifyError for EvmOperationalError {
    fn classify(&self) -> mfm_program::Classification {
        // Executable Reads using this exact cause contract are duplicate-safe observations.
        match self.kind() {
            EvmOperationalKind::Unavailable
            | EvmOperationalKind::Timeout
            | EvmOperationalKind::RateLimited => mfm_program::Classification::Retryable,
        }
    }
}

impl mfm_program::ClassifyError for EvmBalanceFailure {
    fn classify(&self) -> mfm_program::Classification {
        match self {
            Self::AnchorChanged { .. } => mfm_program::Classification::InputInvalidated,
            Self::SourceUnavailable { .. } | Self::IntegrityBlocked { .. } => {
                mfm_program::Classification::Permanent
            }
        }
    }
}

impl mfm_program::ClassifyError for EvmTransactionOperationalError {
    fn classify(&self) -> mfm_program::Classification {
        match self {
            // These causes do not establish whether the external operation was acknowledged.
            Self::Provider { .. } | Self::AuthorityUnavailable => {
                mfm_program::Classification::OutcomeUnknown
            }
            // Signature acquisition precedes retaining and broadcasting prepared wire.
            Self::SignerUnavailable => mfm_program::Classification::Retryable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_program::{Classification, ClassifyError};

    #[test]
    fn operational_semantics_distinguish_observations_from_unknown_transaction_outcomes() {
        for kind in [
            EvmOperationalKind::Unavailable,
            EvmOperationalKind::Timeout,
            EvmOperationalKind::RateLimited,
        ] {
            let cause = EvmOperationalError::new(
                kind,
                ProviderFailure {
                    method: EvmRpcMethod::ChainId,
                    stage: RpcStage::Send,
                    failure: ProviderFailureKind::Client,
                    diagnostics: mfm_values::DiagnosticEvidence::from_value(serde_json::json!({
                        "response": null, "sources": [],
                    })),
                },
            );
            let bytes = mfm_values::canonicalize_mfm_value(&cause).unwrap().0;
            assert_eq!(
                serde_json::from_slice::<EvmOperationalError>(bytes.as_bytes()).unwrap(),
                cause
            );
            let mut invalid = serde_json::to_value(&cause).unwrap();
            invalid["kind"] = serde_json::json!("unknown");
            assert!(serde_json::from_value::<EvmOperationalError>(invalid).is_err());
            assert_eq!(cause.classify(), Classification::Retryable);
            assert_eq!(
                EvmTransactionOperationalError::Provider {
                    operation: crate::TransactionProviderOperation::Submit,
                    cause
                }
                .classify(),
                Classification::OutcomeUnknown
            );
        }
        assert_eq!(
            EvmTransactionOperationalError::AuthorityUnavailable.classify(),
            Classification::OutcomeUnknown
        );
        assert_eq!(
            EvmTransactionOperationalError::SignerUnavailable.classify(),
            Classification::Retryable
        );
    }
}
