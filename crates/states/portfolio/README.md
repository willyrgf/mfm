# mfm-state-portfolio

Typed portfolio state contracts for certified portfolio snapshots.

The portfolio slice owns EVM balance collection because that read exists only for the portfolio
objective. For each demanded EVM network it defines exactly two states:

```text
CollectEvmNetworkState → PublishEvmHoldingsState → EvmNetworkSnapshot
```

`CollectEvmNetworkState` plans one sorted, unique batch of native and ERC-20 sources. Its reducer
requires one source-bound session, one latest number/hash anchor, exact EIP-1898 hash-selected
reads, one deduplicated `decimals()` result per token contract, exact source coverage and order,
canonical uint256 decimals, and one final number-to-hash canonicality check.

`PublishEvmHoldingsState` records the complete batch as
`portfolio.evm_balance_snapshot` facts and returns the direct network snapshot in the same atomic
managed-write attempt. Native facts use the subject asset key `native`; ERC-20 facts use
`erc20:<canonical-contract-address>`.

The remaining snapshot projection is:

```text
Bitcoin receipts → SelectHoldings ─┐
EVM network snapshots ─────────────┼→ AssembleSnapshot → ProjectReport
                                   ┘
```

Only Bitcoin uses receipt-pinned fact-index selection. An all-EVM portfolio has an empty Bitcoin
receipt and performs no fact-index read. `AssembleSnapshotState` validates exact EVM network/source
coverage directly against certified `PortfolioConfig`, then derives wallet identity, symbol
metadata, fixed valuations, and network pins from the direct snapshots.

Portfolio admission bounds networks, wallets, symbols, wallet-symbol relations, and distinct EVM
sources per network before graph expansion. The crate does not own live IO, runner registration,
store commits, CLI/REST rendering, or the complete graph/root binding.
