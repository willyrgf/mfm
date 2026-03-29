# mfm-op-portfolio-tracker

Portfolio tracking operation (`op_id = "portfolio_tracker"`, `op_version = "v1"`).

This op is the thin planner for the canonical multi-network portfolio snapshot flow. It consumes
either the legacy canonical `portfolio` plus `valuation_source_registry` shape or the newer
pre-built portfolio execution config emitted by `mfm-portfolio-config`, then wires the fixed
semantic runtime:

- `PrepareExecutionSources`
- `ResolveSubjects`
- `PinExecutionViews`
- `ResolveValuationInputs`
- `ObserveCompiledBatch`
- `MergeObservations`
- `AssembleSnapshot`
- `ProjectReport`

Protocol and network specialization now happens in the semantic adapter catalog rather than through
protocol-specific graph shapes.

Docs:
- `docs/design.md`
- `docs/ops-and-states.md`
- `bin/cli/README.md`
