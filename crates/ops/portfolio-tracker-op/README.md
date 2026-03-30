# mfm-op-portfolio-tracker

Portfolio tracking operations:

- `portfolio_config_build/v1`: canonical config build root that publishes built config plus
  config artifacts
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

`portfolio_config_build` consumes canonical portfolio config and publishes the typed built config,
the canonical config artifact id, the built config artifact id, and a stable build report.
`portfolio_execute` consumes the pre-built execution config emitted by `mfm-portfolio-config`.
`portfolio_tracker` is the composed compatibility root: canonical input plans the
`portfolio_config_build` publication step before lowering into the same semantic runtime, while
built input keeps the direct execution compatibility path.

Protocol and network specialization now happens in the semantic adapter catalog rather than through
protocol-specific graph shapes.

Docs:
- `docs/design.md`
- `docs/ops-and-states.md`
- `bin/cli/README.md`
