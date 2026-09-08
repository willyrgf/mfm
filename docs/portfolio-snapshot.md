# Portfolio snapshot

`plan_snapshot(selector, config, targets, admission)` returns the checked Program and typed C0. Selector and
configuration are checked secret-free authoring inputs. Targets are nonempty, strictly sorted and
unique by chain ID, and exactly cover the selected chains. Their refs are retained in Program and C0.

Planning constructs checked owned `CollectEvmBalances<PortfolioContinuation>` values, then expands
one private Portfolio root Operation. The root composes each child with an explicit ValueMap from
EvmBalanceFailure to PortfolioSnapshotFailure. It knows typed contracts, not child declaration
counts or indices. Normal completion resumes the next collection or final consolidation.

Each source expands check-chain, initial-anchor, native or token balance Reads, and confirm-anchor.
Token sources additionally read decimals. Planning chooses the typed source implementation directly.
Implementations and adapters register once; their occurrences repeat in the immutable sequence.
Up to 64 total sources are accepted; 65 are rejected.

The cumulative context retains the complete admitted request, caller continuation, route identity,
source order, and checked observations. Initial and final anchor checks prevent combining balances
from different blocks. The selectable EVM classifier recognizes authenticated AnchorChanged and
reviewed transient operational causes. Shipping snapshot execution selects NoRecovery/Stop with zero
recovery allowances; registration alone never enables retries or restarts.

A stopped child failure retains both its original cause and mapped Portfolio root failure in the
canonical FailureReport. Later normal work is suppressed. Final Pure consolidation builds the frozen
snapshot/report with checked decimal-string arithmetic. Read/progress reconstruct from admission and
history after configuration deletion, without consulting configuration custody.

`plan_enrichment` takes the same checked inputs and authors the shared collection prefix followed
by `ResolvePortfolioAssets`. It preserves configured quotes, all native candidates, and nonzero
tokens. Every collection must contain a native source. Its checked output includes resolved config,
selector, route bindings, and collection anchors; it contains no self-referential run/head linkage.
Application adds and verifies that linkage during explicit publication and dependent admission.
