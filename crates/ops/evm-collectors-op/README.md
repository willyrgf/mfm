# mfm-op-evm-collectors

`EvmBalanceCollectionOperation` is the sole reusable EVM balance topology:

```text
CollectEvmBalancesState → RecordEvmBalanceFactsState → EvmBalanceCollectionReceipt
```

It exports only the typed receipt. `evm_balance_collection_cycle_program_draft` and its launch-plan
helper wrap the same operation for scheduler-owned internal cycles and bind only that receipt. They
do not define an application entry point, target resolver, discovery id, or renderer.
