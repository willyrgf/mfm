# mfm-states-evm

Reusable EVM state contracts with no portfolio, app, runtime, transport, storage, or signer-provider
dependency.

The crate owns exactly four state kinds:

```text
CollectEvmBalancesState → RecordEvmBalanceFactsState
SubmitEvmTransactionState
ValidateEvmContractState
```

Balance collection uses one bounded, sorted `EvmBalanceCollectionConfig`, one checked read session,
one exact block anchor, and one `EvmBalanceObservationBatch`. Recording validates the complete batch
and returns `EvmBalanceCollectionReceipt` while the adapter atomically records every
`evm.balance_snapshot` fact. The fact and receipt contain generic EVM source identity only.

State reducers are deterministic and replayable from retained evidence. Live execution and
publication bindings belong to `mfm-adapters-evm`; topology belongs to
`mfm-op-evm-collectors`.
