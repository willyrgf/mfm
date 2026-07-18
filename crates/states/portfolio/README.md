# mfm-state-portfolio

Typed portfolio state contracts for certified portfolio snapshots.

The portfolio slice currently owns EVM balance collection. For each demanded EVM network it
defines exactly two states:

```text
CollectEvmNetworkState → PublishEvmHoldingsState → EvmBalanceCollectionReceipt
```

`CollectEvmNetworkState` plans one sorted, unique batch of native and ERC-20 sources. Its reducer
requires one source-bound session, one latest number/hash anchor, exact EIP-1898 hash-selected
reads, one deduplicated `decimals()` result per token contract, exact source coverage and order,
canonical uint256 decimals, and one final number-to-hash canonicality check.

`PublishEvmHoldingsState` records the complete batch as `evm.balance_snapshot` facts and returns a
checked receipt in the same atomic managed-write attempt. The receipt contains only network/chain,
the common anchor, sorted sources, and fact content-identity evidence; it does not duplicate balance
responses. Every fact retains the complete typed subject object using the same
`HoldingSourceConfig` native/ERC-20 algebra as configuration and collection.

The remaining snapshot projection is:

```text
Bitcoin receipt vector ─┐
EVM receipt vector ─────┴→ SelectHoldings → AssembleSnapshot → ProjectReport
                              ↑
                      one shared fact-index snapshot
```

`SelectHoldingsState` validates both typed receipt vectors against exact portfolio demand, issues
all BTC/EVM queries as one bounded batch, rehydrates every candidate response, rederives its full
fact identity, filters to the receipt-authorized content, and then applies deterministic ordering.
`AssembleSnapshotState` consumes only those selected store-backed observations and validates exact
wallet/symbol/source coverage before deriving totals and network pins. All-EVM portfolios use this
same fact-index path.

Portfolio admission bounds networks, wallets, symbols, wallet-symbol relations, and distinct EVM
sources per network before graph expansion. The crate does not own live IO, runner registration,
store commits, CLI/REST rendering, or the complete graph/root binding.
