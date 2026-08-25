# mfm-portfolio

Portfolio owns checked secret-free selector/config authoring, exact target-set resolution, Program/C0
planning, cumulative aggregation, EVM child-failure mapping, and frozen snapshot/report values.
Targets must be sorted and unique by chain ID; total sources are bounded at 64.
Balance-request fields remain private behind checked accessors, and their constructor fixes one
nonzero chain across the declaration-ordered source set.

The maximum current public Portfolio config shape, including 64 route selections, fits the shared
256 KiB config-custody ceiling. Naming, persistence, and lifecycle remain outside the domain.

Five Pure States implement Program contracts directly. A private root Operation composes checked
owned EVM collection Operations with structured exact failure handlers, so Portfolio does not know
child declaration sizes or indices. The domain has no Runtime, Store, provider, generic
configuration lifecycle, or IO dependency.

The entry point and public State definitions own their compiled-product inspection IDs and
descriptions. This source metadata is not part of Program expansion or identity. The private root
Operation is represented publicly by the entry point rather than admitted as a reusable Operation.
