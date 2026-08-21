# mfm-app

`mfm-app` owns the typed, transport-neutral config, run, and discovery use cases shared by client
surfaces. `Application::from_parts` keeps hermetic library composition available; `Application::open`
is the production convenience that resolves one strict `Deployment`, opens PostgreSQL config/run
custody, constructs stock EVM HTTP(S) clients, and delegates to the same injected constructor.

`Deployment::load` selects an explicit override or exactly
`$XDG_CONFIG_HOME/mfm/deployment.toml`, falling back to `$HOME/.config/mfm/deployment.toml` only when
the XDG base is unset, empty, or relative. The bounded strict TOML contains environment resolver
names, never locator values. Its EVM routes are empty or strictly ordered and unique by
`(chain_id, endpoint_id)`, with at most 256 bindings.

`ConfigDocument` is an opaque async checked value. It bounds input at 256 KiB, rejects malformed
UTF-8/JSON, duplicate keys, floats, excess depth, unknown fields, and invalid domain values, then
owns canonical JSON plus its `sha256-jcs-v1` digest. The required entry-point tag is
`mfm.portfolio/snapshot@1`; its route array has 1–64 entries, is strictly chain-ordered, and must
exactly cover the Portfolio source chains when Application runs the planner during import.

One `ComposedRuntime` derives Runtime adapter registration, public binding discovery, planning
targets, Store, and RunIndex from the same checked inputs. Config catalog custody is independently
injected. The complete Application surface is:

- static entry-point and composed-binding discovery;
- config import, read, keyset list, and conditional delete;
- stored-config run start through exhaustive `Current` or `Exact` selection;
- run progress, semantic read, and mechanical keyset list.

Every execution receives an explicit caller-owned `RunId`. The pure `derive_run_id([u8; 32])`
helper implements `mfm.run-id.random.v1`; it performs no IO and Application never calls it.
`StartRunResult` reports the actual selected config revision. An ambiguously acknowledged append is
the only `RunRequestError` carrying data, through the exact `RunRecovery::Start` or
`RunRecovery::Progress` sum.

`RequestError` owns stable redaction-safe codes and messages. Catalog rows are revalidated as
canonical documents on every read/start/list. Unbound routes fail before Runtime Store IO;
deleting or rebinding a config never changes retained run genesis. Shared serializers preserve the
`RunViewState` sum and embed terminal canonical bytes as a raw JSON value.
