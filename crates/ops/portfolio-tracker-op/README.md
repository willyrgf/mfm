# mfm-op-portfolio-tracker

Typed portfolio tracking operation (report-only after collectors cutover):

- `mfm.portfolio.tracker_workflow`: typed operation authored through `mfm-program` and certified
  into a typed execution spec

## Graph

```text
PortfolioCollectionReceipt → SelectHoldings  // exact receipt-pinned Platform fact query
ResolveSubjects ────────────────────────────┐
ResolveValuations ──────────────────────────┼→ AssembleSnapshot → ProjectReport
SelectHoldings ─────────────────────────────┘
```

Live `PinViews` / `ObserveBatch` / `MergeObservations` are **not** part of this op. The parent
collection operation supplies one exact family-complete receipt; this workflow reads only
receipt-named Platform facts and reports from that evidence.

Runtime execution is provided by the typed runner registry in `mfm-adapters-portfolio`.

Docs:
- `docs/design.md`
- `docs/architecture.md`
- `bin/cli/README.md`
