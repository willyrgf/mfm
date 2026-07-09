# mfm-state-portfolio

Typed portfolio state contracts for certified **fact-backed, report-only** portfolio snapshots.

This crate owns the portfolio state specs, typed inputs/outputs, pure selection policy helpers,
and hard-fail assemble/report projection used by `mfm-op-portfolio-tracker`. Runtime capability
execution is supplied by typed runners in `mfm-adapters-portfolio` (Platform fact-index only).

## Graph (cutover)

```text
ResolveSubjects
  → SelectHoldings           // Platform fact-index + network-coherent select
  → ResolveValuations        // FixedUnitPrice only
  → AssembleSnapshot         // pins from selected holding anchors
  → ProjectReport
```

State contracts:

- `ResolveSubjectsState`
- `SelectHoldingsState`
- `ResolveValuationsState`
- `AssembleSnapshotState`
- `ProjectReportState`

Selection policy: `mfm.portfolio.holding.latest-network-coherent.v1` (pure
`select_network_coherent`). Cutover projections: BTC + EVM native holdings only; ERC-20 and
protocol positions hard-fail as `unsupported_requirement`.

There is no live pin/observe path, no soft-success `errors` / `error_count`, and no view-dependent
valuation on the cutover surface.

The crate does not own store commits, runner registration, CLI/REST rendering, or collector source
IO (those live in family collector ops).
