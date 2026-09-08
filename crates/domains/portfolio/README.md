# mfm-portfolio

Portfolio owns checked secret-free selector/config authoring, exact target-set resolution, Program/C0
planning, cumulative aggregation, EVM child-failure mapping, and frozen snapshot/report values.
Targets must be sorted and unique by chain ID; total sources are bounded at 64.
Balance-request fields remain private behind checked accessors, and their constructor fixes one
nonzero chain across the declaration-ordered source set.

The maximum current public Portfolio config shape, including 64 route selections, fits the shared
256 KiB config-custody ceiling. Naming, persistence, and lifecycle remain outside the domain.

Four Pure States initialize, enter, resume and consolidate the cumulative context. A private root
Operation composes checked EVM collection Operations in request order. `MapEvmBalanceFailure` is a
ValueMap from the child's original failure to the public Portfolio failure. Program identity commits
the exact admitted input and selected routes; child authoring receives the complete checked request.

Planning bounds the complete continuation, final output and root failure, then supplies each EVM
child's complete conclusion bound. The shipping planner selects the framework's stop defaults,
with zero global and local recovery allowances. These declarations are checked by Runtime
against actual closure sizes and Journal capacity. The domain has no Runtime, Store, provider,
generic configuration lifecycle, or IO dependency.

The entry point and public State definitions own their compiled-product inspection IDs and
descriptions. This source metadata is not part of Program expansion or identity. The private root
Operation is represented publicly by the entry point rather than admitted as a reusable Operation.

Focused contract tests cover exact route selection, 64/65-source planning, concrete native/token
execution, and cold reconstruction of domain and operational failures. The value-bound unit test
covers completed prefixes and final snapshots with full-width balances and public anchor fields.

`plan_snapshot` and `plan_enrichment` accept optional checked `PortfolioAdmission` metadata.
Application supplies the exact source revision identity and verified enrichment linkage; direct
library callers may use `None`. The shared sequence collects anchored observations, then selects
either snapshot consolidation or candidate resolution. Enrichment requires a native source in each
collection, retains all natives and nonzero tokens in order, and preserves quotes and route refs.
`PortfolioEnrichmentOutput` is the checked publication input; no domain code owns config custody.
