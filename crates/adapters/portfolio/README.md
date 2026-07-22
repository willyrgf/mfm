# mfm-adapters-portfolio

Executable runner bindings for certified portfolio states.

## Live path

`SelectHoldings` consumes the typed Bitcoin and EVM receipt vectors directly. The adapter submits
all receipt-derived requests in one fact-index batch, requires one shared snapshot frontier,
hydrates each returned response artifact, and records the state-produced query evidence. Portfolio
assembly therefore receives only store-reread and identity-reverified observations; an all-EVM
portfolio follows exactly the same path.

## Replay path

`verify_portfolio_replay` uses only certified configs, the append-only stream, and retained
artifacts. Bitcoin/EVM selection is recomputed from retained fact-query evidence, and the pure
snapshot/report outputs are compared byte-for-byte. EVM collection/publication replay lives only
in the private adapter of `mfm-evm-live`.

This crate binds portfolio fact-index reads and pure projections. It does not create workflow
topology, select runtime routes, implement JSON-RPC/ERC-20 codecs, publish family facts, or own
family state semantics.
