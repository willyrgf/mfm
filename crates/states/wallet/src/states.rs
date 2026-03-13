use async_trait::async_trait;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx::write_json;
use mfm_state_common::errors::{state_error_with_state, state_unknown};
use mfm_state_common::states::meta;

use crate::model::{ResolvedWallet, WalletCapabilities, WalletConfig, WalletImplementationConfig};

/// Resolves configured wallets into runtime wallet identities and capabilities.
#[derive(Clone, Debug)]
pub struct ResolveWalletsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Wallet configs to resolve.
    pub wallets: Vec<WalletConfig>,
    /// Context key that receives the resolved wallets.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for ResolveWalletsState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut resolved = self.wallets.clone();
        resolved.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));

        let resolved: Vec<ResolvedWallet> = resolved
            .into_iter()
            .map(|wallet| match wallet.implementation {
                WalletImplementationConfig::AddressOnly {} => Ok(ResolvedWallet {
                    wallet_id: wallet.wallet_id,
                    address: wallet.address,
                    network_id: wallet.network_id,
                    implementation_kind: "address_only".to_string(),
                    capabilities: WalletCapabilities {
                        can_resolve_address: true,
                        can_sign: false,
                        can_submit: false,
                    },
                    signer: None,
                }),
                WalletImplementationConfig::KeystoreEntry { .. } => {
                    Err(unsupported_wallet_impl(&self.state_id, "keystore_entry"))
                }
                WalletImplementationConfig::NodeManagedAccount { .. } => Err(
                    unsupported_wallet_impl(&self.state_id, "node_managed_account"),
                ),
                WalletImplementationConfig::ExternalSigner { .. } => {
                    Err(unsupported_wallet_impl(&self.state_id, "external_signer"))
                }
            })
            .collect::<Result<_, _>>()?;

        let value = serde_json::to_value(&resolved).map_err(|_| {
            state_unknown(
                "resolved_wallets_serialize_failed",
                "failed to serialize resolved wallets",
            )
        })?;
        write_json(ctx, self.output_key.clone(), value)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

fn unsupported_wallet_impl(state_id: &StateId, kind: &'static str) -> StateError {
    state_error_with_state(
        state_id.clone(),
        "unsupported_wallet_implementation_kind",
        ErrorCategory::Unknown,
        false,
        format!("wallet implementation kind `{kind}` is not supported in the base runtime slice"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_machine::context::DynContext;
    use mfm_machine::errors::{ContextError, ErrorInfo, RunError};
    use mfm_machine::events::DomainEvent;
    use mfm_machine::ids::ErrorCode;

    #[derive(Default)]
    struct MapContext {
        inner: std::collections::HashMap<String, serde_json::Value>,
    }

    impl DynContext for MapContext {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
            Ok(self.inner.get(&key.0).cloned())
        }

        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
            self.inner.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
            self.inner.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, ContextError> {
            let mut out = serde_json::Map::new();
            for (key, value) in &self.inner {
                out.insert(key.clone(), value.clone());
            }
            Ok(serde_json::Value::Object(out))
        }
    }

    struct NoopIo;

    #[async_trait]
    impl IoProvider for NoopIo {
        async fn call(
            &mut self,
            _call: mfm_machine::io::IoCall,
        ) -> Result<mfm_machine::io::IoResult, mfm_machine::errors::IoError> {
            unreachable!("no io expected")
        }

        async fn record_value(
            &mut self,
            _key: mfm_machine::ids::FactKey,
            _value: serde_json::Value,
        ) -> Result<mfm_machine::ids::ArtifactId, mfm_machine::errors::IoError> {
            unreachable!("no io expected")
        }

        async fn get_recorded_fact(
            &mut self,
            _key: &mfm_machine::ids::FactKey,
        ) -> Result<Option<mfm_machine::ids::ArtifactId>, mfm_machine::errors::IoError> {
            unreachable!("no io expected")
        }

        async fn now_millis(&mut self) -> Result<u64, mfm_machine::errors::IoError> {
            Ok(0)
        }

        async fn random_bytes(
            &mut self,
            n: usize,
        ) -> Result<Vec<u8>, mfm_machine::errors::IoError> {
            Ok(vec![0; n])
        }
    }

    #[derive(Default)]
    struct NoopRecorder;

    #[async_trait]
    impl EventRecorder for NoopRecorder {
        async fn emit(&mut self, _event: DomainEvent) -> Result<(), RunError> {
            Ok(())
        }

        async fn emit_many(&mut self, _events: Vec<DomainEvent>) -> Result<(), RunError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn resolves_address_only_wallets() {
        let mut ctx = MapContext::default();
        let mut io = NoopIo;
        let mut rec = NoopRecorder;
        let state = ResolveWalletsState {
            state_id: StateId::must_new("wallet.main.resolve".to_string()),
            wallets: vec![WalletConfig {
                wallet_id: "wallet_a".to_string(),
                address: "0x000000000000000000000000000000000000dead".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".to_string()],
                metadata: std::collections::BTreeMap::new(),
            }],
            output_key: ContextKey("resolved_wallets".to_string()),
        };

        state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("resolve");
        let resolved = ctx
            .read(&ContextKey("resolved_wallets".to_string()))
            .expect("read")
            .expect("present");
        let resolved: Vec<ResolvedWallet> = serde_json::from_value(resolved).expect("typed");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].implementation_kind, "address_only");
        assert!(!resolved[0].capabilities.can_sign);
    }

    #[tokio::test]
    async fn rejects_unsupported_wallet_impls() {
        let mut ctx = MapContext::default();
        let mut io = NoopIo;
        let mut rec = NoopRecorder;
        let state = ResolveWalletsState {
            state_id: StateId::must_new("wallet.main.resolve".to_string()),
            wallets: vec![WalletConfig {
                wallet_id: "wallet_a".to_string(),
                address: "0x000000000000000000000000000000000000dead".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                implementation: WalletImplementationConfig::ExternalSigner {
                    signer_id: "sig".to_string(),
                },
                symbol_ids: vec!["eth.native.ethereum-mainnet".to_string()],
                metadata: std::collections::BTreeMap::new(),
            }],
            output_key: ContextKey("resolved_wallets".to_string()),
        };

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("expected unsupported impl");
        assert_eq!(
            err.info,
            ErrorInfo {
                code: ErrorCode("unsupported_wallet_implementation_kind".to_string()),
                category: ErrorCategory::Unknown,
                retryable: false,
                message: "wallet implementation kind `external_signer` is not supported in the base runtime slice".to_string(),
                details: None,
            }
        );
    }
}
