# mfm-op-portfolio-snapshot

This operation crate owns the complete internal portfolio objective:

```text
normalized PortfolioConfig
  → family collection children
  → exact PortfolioCollectionReceipt
  → receipt-pinned fact selection
  → snapshot assembly
  → report projection
  → one PortfolioPublicOutputs root binding
```

`portfolio_snapshot_program_draft` and `portfolio_snapshot_program_launch_plan` build the same
certified graph. App ingress uses that one graph for `mfm.portfolio/snapshot@1` after resolving one
exact `PortfolioConfig` catalog reference; the app has no parallel graph builder. The crate does
not do live IO; family and portfolio adapters supply runners for the certified state descriptors.
