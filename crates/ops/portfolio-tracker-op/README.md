# mfm-op-portfolio-tracker

Typed portfolio tracking operation:

- `mfm.portfolio.tracker_workflow`: typed operation authored through `mfm-program` and certified
  into a typed execution spec

This op is a thin typed planner for the canonical multi-network portfolio snapshot flow. It wires
typed state contracts through handles, domain-keyed fanout/fanin, non-empty observation batches,
and typed public outputs:

- `PrepareSources`
- `ResolveSubjects`
- `PinViews`
- `ResolveValuations`
- `ObserveBatch`
- `MergeObservations`
- `AssembleSnapshot`
- `ProjectReport`

The crate exposes no legacy dynamic `PlannedOp`, `PortKey`, `DynContext`, context-key, or
hand-authored dependency-edge surface. Runtime execution is provided by the typed runner registry
in `mfm-transports-portfolio`.

Docs:
- `docs/design.md`
- `docs/ops-and-states.md`
- `bin/cli/README.md`
