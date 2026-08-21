# mfm-app

`mfm-app` owns the typed, transport-neutral config, run, and discovery use cases shared by client
surfaces. `Application::from_parts` keeps hermetic library composition available; `Application::open`
is the production convenience that resolves one strict `Deployment`, opens PostgreSQL config/run
custody, constructs stock EVM HTTP(S) clients, and delegates to the same injected constructor.

`Deployment::load` selects an explicit override or exactly
`$XDG_CONFIG_HOME/mfm/deployment.toml`, falling back to `$HOME/.config/mfm/deployment.toml` only when
the XDG base is unset, empty, or relative. The bounded strict TOML contains environment resolver
names, never locator values. Its EVM routes are empty or strictly ordered and unique by
`(chain_id, endpoint_id)`, with at most 256 bindings. Chain IDs are integers in the inclusive range
1 through 9,223,372,036,854,775,807.

A complete deployment with one EVM route is:

```toml
[postgres]
runtime_locator_env = "MFM_RUNTIME_POSTGRES_LOCATOR"

[[evm_routes]]
chain_id = 1
endpoint_id = "mainnet-primary"
adapter_locator_env = "MFM_EVM_MAINNET_LOCATOR"
```

Resolver names use `[A-Z_][A-Z0-9_]*` and contain 1–64 bytes. The runtime PostgreSQL resolver must hold
one raw, loopback-only `postgresql` URL accepted by `RuntimePostgresLocator`; each adapter resolver
must hold one raw HTTP(S) URL accepted by `EvmAdapterLocator`. Administrative PostgreSQL authority
is not part of this file and is resolved only by the CLI `postgres init` command.

`ConfigDocument` is an opaque async checked value. It bounds input at 256 KiB, rejects malformed
UTF-8/JSON, duplicate keys, floats, excess depth, unknown fields, and invalid domain values, then
owns canonical JSON plus its `sha256-jcs-v1` digest. The required entry-point tag is
`mfm.portfolio/snapshot@1`; its route array has 1–64 entries, is strictly chain-ordered, and must
exactly cover the Portfolio source chains when Application runs the planner during import.

One `ComposedRuntime` derives Runtime adapter registration, public binding discovery, planning
targets, Store, and RunIndex from the same checked inputs. The production Application coerces the
same PostgreSQL backend into Store, RunIndex, and configuration-repository ports. The complete Application
surface is:

- static entry-point and composed-binding discovery;
- atomic config import/current selection and complete retained-revision list;
- stored-config run start through exhaustive `Current` or `Exact` selection;
- run progress, semantic read, and mechanical `RunId`-keyset list.

Every execution receives an explicit caller-owned `RunId`. The pure `derive_run_id([u8; 32])`
helper implements `mfm.run-id.random.v1`; it performs no IO and Application never calls it.
`StartRunResult` reports the actual selected config revision. An ambiguously acknowledged append is
the only `RunRequestError` carrying data, through the exact `RunRecovery::Start` or
`RunRecovery::Progress` sum.

`RequestError` owns stable redaction-safe codes and messages. Retained revisions are revalidated as
canonical documents on every start/list. Unbound routes fail before Runtime Store IO; importing a
new revision never changes retained run genesis. Shared serializers preserve the
`RunViewState` sum and embed terminal canonical bytes as a raw JSON value.
