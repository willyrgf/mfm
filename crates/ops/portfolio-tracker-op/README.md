# mfm-op-portfolio-tracker

Portfolio tracking operations:

- `portfolio_execute/v1`: strict built-config execution root used by thin transport adapters
- `portfolio_tracker/v1`: legacy compatibility root that still accepts canonical-or-built config

These ops are thin planners for the canonical multi-network portfolio snapshot flow. They wire the
same fixed semantic runtime:

- `PrepareExecutionSources`
- `ResolveSubjects`
- `PinExecutionViews`
- `ResolveValuationInputs`
- `ObserveCompiledBatch`
- `MergeObservations`
- `AssembleSnapshot`
- `ProjectReport`

`portfolio_execute` consumes the pre-built execution config emitted by `mfm-portfolio-config`.
`portfolio_tracker` remains available during migration for callers that still send the legacy
canonical `portfolio` plus `valuation_source_registry` shape.

Protocol and network specialization now happens in the semantic adapter catalog rather than through
protocol-specific graph shapes.

Docs:
- `docs/design.md`
- `docs/ops-and-states.md`
- `bin/cli/README.md`
