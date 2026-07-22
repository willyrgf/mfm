# mfm-portfolio-live

Live selection, hydration, and runner bindings for certified portfolio states.

## Live path

`SelectHoldings` consumes the typed Bitcoin and EVM receipt vectors directly. The adapter submits
all receipt-derived requests in one fact-index batch, requires one shared snapshot frontier,
hydrates each returned response artifact, and records the state-produced query evidence. Portfolio
assembly therefore receives only store-reread and identity-reverified observations; an all-EVM
portfolio follows exactly the same path.

## Replay path

Portfolio replay uses the generic external-read and pure-state verifiers selected by app assembly.
It consumes only certified configs, the append-only stream, and retained artifacts. Bitcoin/EVM
selection is recomputed from retained fact-query evidence, and the pure snapshot/report outputs
are compared byte-for-byte. Family collection/publication replay remains in the corresponding
family live package.

Registration accepts one shared store `Arc` implementing both fact-query and retained-artifact-read
contracts, then internally narrows clones of that same value to the least authority each path
needs. It is therefore impossible for app assembly to combine query results from one store with
artifact bytes from another.

This crate binds portfolio fact-index reads and pure projections. It implements no generic provider
or transport, and it does not create workflow topology, select runtime routes, implement copied
JSON-RPC/ERC-20 codecs, publish family facts, or own family state semantics.
