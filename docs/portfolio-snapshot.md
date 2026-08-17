# Portfolio snapshot

`plan_snapshot(selector, config, targets)` returns the checked Program and typed C0. Selector and
configuration are checked process-local authoring inputs, not persisted generic configuration.
Targets are strictly sorted/unique by chain ID and each selected target ref is retained in Program
and C0.

Planning constructs checked owned `CollectEvmBalances<PortfolioContinuation>` values, then expands
one private Portfolio root Operation. The root composes each child through a scoped exact
`EvmBalanceFailure` handler; it knows the child's typed contracts but never its declaration count or
indices. The mapper is a terminal root-success path, while ordinary child completion resumes the
next collection or final consolidation.

For every source, the Program emits check-chain, initial-anchor, select-asset, native/token Match,
balance reads, confirm-anchor, and consolidation occurrences. Occurrences repeat in the Program;
typed State implementations and adapter callbacks register once. Up to 64 total sources are
accepted; 65 are rejected.

The cumulative context retains the complete admitted request, caller continuation, route identity,
source order, and checked observations. Native and token branches rejoin one contract. Each child
failure reaches one Portfolio failure mapper; later normal work is suppressed. Final Pure
consolidation builds the frozen snapshot/report with checked decimal-string arithmetic.
