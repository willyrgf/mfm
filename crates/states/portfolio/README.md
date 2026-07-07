# mfm-state-portfolio

Typed portfolio state contracts for certified portfolio snapshot workflows.

This crate owns the portfolio state specs, typed inputs/outputs, domain-keyed fanout/fanin behavior,
pure/read state logic, and deterministic read-intent classification used by
`mfm-op-portfolio-tracker`. Runtime capability execution is supplied by typed runners in
`mfm-adapters-portfolio`.

State contracts:

- `PrepareSourcesState`
- `ResolveSubjectsState`
- `PinViewsState`
- `ResolveValuationsState`
- `ObserveBatchState`
- `MergeObservationsState`
- `AssembleSnapshotState`
- `ProjectReportState`

The crate does not own store commits, runner registration, CLI/REST rendering, or live RPC source
configuration.
