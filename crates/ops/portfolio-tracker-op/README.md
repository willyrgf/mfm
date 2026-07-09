# mfm-op-portfolio-tracker

Typed portfolio tracking operation (report-only after collectors cutover):

- `mfm.portfolio.tracker_workflow`: typed operation authored through `mfm-program` and certified
  into a typed execution spec

## Graph

```text
ResolveSubjects
  → SelectHoldings           // Platform fact-index + network-coherent select
  → ResolveValuations        // FixedUnitPrice only
  → AssembleSnapshot         // network_pins from selected holding anchors
  → ProjectReport
```

Live `PinViews` / `ObserveBatch` / `MergeObservations` are **not** part of this op. Collectors
write Platform holding facts in separate runs; this op only reports.

Runtime execution is provided by the typed runner registry in `mfm-adapters-portfolio`.

Docs:
- `docs/design.md`
- `docs/architecture.md`
- `bin/cli/README.md`
