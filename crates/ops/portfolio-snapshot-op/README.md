# mfm-op-portfolio-snapshot

This operation crate owns the complete internal portfolio objective:

```text
normalized PortfolioConfig
  → Bitcoin collection children → typed Bitcoin receipt vector ─┐
  → one EVM read + atomic publication pair per network → typed EVM receipt vector ─┤
  → receipt-pinned BTC/EVM store selection ←───────────────────────────────────────┘
  → snapshot assembly → report projection → one PortfolioPublicOutputs root binding
```

`portfolio_snapshot_program_draft` and `portfolio_snapshot_program_launch_plan` build the same
certified graph. App ingress uses that one graph for `mfm.portfolio/snapshot@1` after resolving one
target-keyed current `PortfolioConfig`; the app has no parallel graph builder. The two typed family
receipt vectors flow directly into `SelectHoldingsState`, so managed-write completion is the
selection barrier without a generic fan-in state. Assembly consumes no direct family observation.

EVM planning is private to this operation for now. It compiles normalized wallet/symbol demand into
one sorted, unique `EvmNetworkCollectionConfig` per network. Portfolio config validation enforces
all collection cardinality limits before graph expansion. The crate performs no live IO; family and
portfolio adapters supply runners for certified state descriptors.
