# mfm-op-evm-collectors

`EvmBalanceCollectionOperation` is the sole reusable EVM balance topology:

```text
CollectEvmBalancesState → EvmBalanceCollectionReceipt + ordered fact batch
```

It exports only the typed receipt. Parent operations compose it directly; this crate exposes no
standalone draft, launch plan, application entry point, target resolver, discovery id, or renderer.
