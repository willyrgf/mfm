# mfm-portfolio

This pure aggregate-domain crate owns the canonical portfolio model, the sole
published snapshot entry point, its deterministic EVM-only topology, and the
three portfolio state callbacks.

The production graph is:

```text
ValidatePortfolioSnapshotSelectionState
  -> one EVM collection per validated network position
       bootstrap -> latest anchor -> bounded decimals/balance fan-out
       -> confirm anchor -> pure exact aggregation
  -> AssemblePortfolioSnapshotState
  -> ProjectPortfolioReportState
  -> PortfolioPublicOutputs { snapshot, report }
```

`PortfolioSnapshotSelector { target }` is the only run-admission input.
Configuration publication separately retains the matching `PortfolioConfig`.
Qualified support retains one immutable `PortfolioRoutingManifest` and the
framework `UnitConfig`; deterministic authoring reads those verified values
only to assemble topology. The validator is the single upstream state that
mints stronger selection authority for every downstream read and output.

The only published entry point is `mfm.portfolio/snapshot@1`, backed by stable
operation `mfm.portfolio/snapshot`, an empty framework-policy list, and empty
canonical profile parameters. Package-owned registration helpers expose the
entry contract, the three portfolio states, exact value contracts, and fixed
support-root paths without admitting support or performing provider IO.

Bitcoin execution, aggregate live readers, retry/failover/reselection, mutation
lifecycle, prior-run reducers, fact queries, and replay helpers are absent.
