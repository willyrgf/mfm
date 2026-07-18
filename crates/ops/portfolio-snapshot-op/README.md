# mfm-op-portfolio-snapshot

This operation crate owns the complete internal portfolio objective:

```text
normalized PortfolioConfig
  → Bitcoin collection children → exact Bitcoin receipt → receipt-pinned Bitcoin selection
  → one EVM read + one atomic publication state per demanded EVM network
  → snapshot assembly from selected Bitcoin holdings + direct EVM network snapshots
  → report projection
  → one PortfolioPublicOutputs root binding
```

`portfolio_snapshot_program_draft` and `portfolio_snapshot_program_launch_plan` build the same
certified graph. App ingress uses that one graph for `mfm.portfolio/snapshot@1` after resolving one
target-keyed current `PortfolioConfig`; the app has no parallel graph builder. The crate does
not do live IO; family and portfolio adapters supply runners for the certified state descriptors.

EVM planning is private to this operation. It compiles normalized wallet/symbol demand into one
sorted, unique `EvmNetworkCollectionConfig` per network and never exposes a standalone EVM
collector operation. Portfolio config validation enforces all collection cardinality limits before
this graph is expanded.
