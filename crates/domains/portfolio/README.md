# mfm-portfolio

Portfolio owns semantic collection admission, declaration-ordered planning, checked aggregation and
snapshot/enrichment continuations. Its production dependency is the shared Chain domain; EVM,
provider, Runtime and Store dependencies are absent.

Native clients construct checked PortfolioSnapshotInput or PortfolioEnrichmentInput. Each collection
retains its BalanceRequest and source-aligned BalanceExecutionConfig descriptors. The descriptors
carry the caller's expected route reference plus opaque native facts; Portfolio checks count and
common route agreement without decoding native configuration. Total sources remain bounded at 64.

PortfolioSnapshotOperation and PortfolioEnrichmentOperation derive finite collection vectors from
admitted demand. Each source uses shared BalanceSourceDefinition and ObserveBalance/BalanceRead.
The native environment selects and injects its supporting States; Chain confirms and consolidates
semantic balances. Portfolio checks request, ordinal, correlation and route at entry/resumption.

Snapshot collections retain confirmed shared balances and execution descriptors. Enrichment keeps
caller-required sources or nonzero balances, filtering each source and descriptor together. Native
clients own endpoint reconstruction, native configuration publication, and native product rendering.
Semantic output identity remains distinct from the rendered representation.

Exact domain failures retain their native/shared originals. The native client decodes exact State
and input contracts and passes a typed BalanceFailureCode to Portfolio's continuation projection.
One private semantic check validates collection/context agreement. A mismatch is a projection error;
it does not become a fabricated ConsolidationFailed. Genuine Portfolio originals remain unchanged.

The evolving cutover and verification ledger is in [dsl-phase-b.md](../../../docs/dsl-phase-b.md).
Historical isolated proof evidence remains in [dsl-phase-a.md](../../../docs/dsl-phase-a.md).
