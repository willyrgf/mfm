# mfm-evm

Pure EVM protocol, audited-read state, graph, and signing contracts. The crate
has no portfolio, app, runtime, transport, storage, keystore, or concrete
signer-provider dependency.

Balance collection authors a certified graph with these audit units:

1. bootstrap one exact admitted routing generation and verify its chain;
2. read an initial latest-block anchor;
3. fan out one token-decimals read per distinct token and one anchored balance
   read per holding;
4. confirm the initial anchor after every fan-out read; and
5. aggregate the retained same-run values with pure state logic.

Each read state selects exactly one protocol operation. A collection admits at
most 1,024 holdings and 1,024 distinct tokens, producing at most 2,052 graph
nodes. The source, chain, routing generation, fan-out coverage, and final
anchor are checked again during pure aggregation.

Production aggregate-reader, transaction/effect lifecycle, ingress-validation,
and replay-helper surfaces are absent. Recoverability-neutral transaction
models and EIP-1559 signing remain reusable protocol primitives for a future
qualified executor.
