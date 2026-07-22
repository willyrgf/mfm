# mfm-state-portfolio

Typed portfolio state contracts for certified portfolio snapshots.

Family collectors are external reusable operations. This crate consumes their typed Bitcoin and
EVM receipts plus registered fact descriptors; it defines no family read, RPC, fact-publication,
or collection-replay implementation.

`PortfolioReportOperation` owns the remaining snapshot projection:

```text
Bitcoin receipt vector ─┐
EVM receipt vector ─────┴→ SelectHoldings → AssembleSnapshot → ProjectReport
                              ↑
                      one shared fact-index snapshot
```

`SelectHoldingsState` validates both typed receipt vectors against exact portfolio demand, issues
all Bitcoin/EVM queries as one bounded batch, rehydrates every candidate response, rederives its full
fact identity, filters to the receipt-authorized content, and then applies deterministic ordering.
`AssembleSnapshotState` consumes only those selected store-backed observations and validates exact
wallet/symbol/source coverage before deriving totals and network pins. All-EVM portfolios use this
same fact-index path.

Portfolio admission bounds networks, wallets, symbols, wallet-symbol relations, and distinct EVM
sources per network before graph expansion. Generic EVM collection contracts live in
`mfm-states-evm`. This crate does not own live IO, family fact publication, runner registration,
store commits, CLI/REST rendering, or the complete graph/root binding.
