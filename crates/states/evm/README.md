# mfm-states-evm

Reusable EVM state contracts with no portfolio, app, runtime, transport, storage, or signer-provider
dependency.

The crate owns exactly three state kinds:

```text
CollectEvmBalancesState
SubmitEvmTransactionState
ValidateEvmContractState
```

Balance collection uses one bounded, sorted `EvmBalanceCollectionConfig`, one checked read session,
and one exact block anchor. Its reducer returns an ordered non-empty `evm.balance_snapshot` fact
batch and `EvmBalanceCollectionReceipt`; runtime settles both atomically with the read evidence and
completion. The fact and receipt contain generic EVM source identity only.

State reducers are deterministic and replayable from retained evidence. Live execution bindings
belong to `mfm-adapters-evm`; topology belongs to
`mfm-op-evm-collectors`.
