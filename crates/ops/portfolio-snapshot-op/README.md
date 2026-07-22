# mfm-op-portfolio-snapshot

This operation crate owns the complete internal portfolio objective through two composable
operations:

```text
PortfolioSnapshotOperation(normalized PortfolioConfig)
  ├─→ Bitcoin collection children → typed Bitcoin receipt vector ─┐
  ├─→ EVM balance collection children → typed EVM receipt vector ─┤
  └─→ PortfolioReportOperation(receipt handles) ←──────────────────┘
        → receipt-pinned Bitcoin/EVM store selection
        → snapshot assembly → report projection → one PortfolioPublicOutputs root binding
```

`portfolio_snapshot_program_draft` and `portfolio_snapshot_program_launch_plan` build the same
certified graph. App ingress uses that one graph for `mfm.portfolio/snapshot@2` after resolving one
target-keyed current `PortfolioConfig`; the app has no parallel graph builder. The two typed family
receipt vectors flow directly into `SelectHoldingsState`, so collector settlement is the
selection barrier without a generic fan-in state. Assembly consumes no direct family observation.
`PortfolioSnapshotOperation` constructs no states directly. `PortfolioReportOperation` owns the
exact SelectHoldings → AssembleSnapshot → ProjectReport state chain, and its structured operation
input is the same binding consumed by `SelectHoldingsState`.

Portfolio planning compiles normalized wallet/symbol demand into one sorted, unique generic
`EvmBalanceCollectionConfig` per network, then calls `EvmBalanceCollectionOperation` in a child
scope and bridges only its receipt. Portfolio config validation enforces collection cardinality
limits before graph expansion. The crate performs no live IO and constructs no family state
directly; family and portfolio adapters supply runners for certified descriptors.
