# mfm-state-portfolio

Typed portfolio state contracts for certified **fact-backed, receipt-pinned** portfolio snapshots.

This crate owns portfolio state specs, typed receipt/selection inputs, and hard-fail
assemble/report projection. Runtime capability execution is supplied by typed runners in
`mfm-adapters-portfolio` (Platform fact-index only).

## Graph (cutover)

```text
PortfolioCollectionReceipt → SelectHoldings → AssembleSnapshot → ProjectReport
```

State contracts:

- `SelectHoldingsState`
- `AssembleSnapshotState`
- `ProjectReportState`

`AssembleSnapshotState` derives wallet identity, symbol metadata, and fixed valuations directly
from its certified `PortfolioConfig`; selected holdings contribute only receipt-pinned quantities
and collection evidence.

Selection policy: `mfm.portfolio.holding.collection-receipt-anchor.v1`. The receipt fixes each
logical source, descriptor-checked fact-content identity, exact anchor, coverage, and status.
The adapter requests the closed N + 1 candidate limit, rejects saturation before hydration,
hydrates every candidate, filters exact identities, then orders only matching claims. BTC native,
EVM native, and EVM ERC-20 facts are supported; successful zero balances remain observations.

There is no live pin/observe path, no soft-success `errors` / `error_count`, and no view-dependent
valuation on the cutover surface.

The crate does not own store commits, runner registration, CLI/REST rendering, collector source
IO, or the complete graph/root binding (those live in adapters, family collector operations, and
`mfm-op-portfolio-snapshot`).
