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

- static compiled-component and entry-point discovery plus composed-binding discovery;
- immutable exact-revision config import, complete list, and idempotent delete;
- stored-config run start through one exact name/digest selection;
- run progress, semantic read, and mechanical `RunId`-keyset list.

Compiled-component discovery returns the entry points, public reusable Operations, and Pure/Read
States admitted by the product composition. Domain definitions own their stable IDs and
human-readable descriptions. The private State inventory also performs the exact Runtime State
registrations, so discovery cannot drift from live composition. It performs no Program expansion,
configuration parsing, deployment resolution, Store access, or provider IO; descriptions are
non-semantic and never enter Program bytes or durable history.

Every execution receives an explicit caller-owned `RunId`. The shared `generate_run_id()` client
primitive obtains exactly 32 bytes of OS cryptographic entropy and applies
`mfm.run-id.random.v1`; an entropy failure is the stable `RunIdGenerationError`.
`StartRunResult` reports the actual selected config revision. `RunRequestError` distinguishes
pre-execution request errors, ambiguous appends with exact `RunRecovery::Start`/`Progress` identity,
and stopped Runtime invocations. Ambiguous appends also retain the last completely qualified
observation, or null when none was obtained. That observation is not a claim about the current head.

`RequestError` owns reviewed codes and messages. `SerializableClientError::for_run` renders the
shared invocation/recovery detail. An execution-stopped invocation includes its RunId and optional
last observation; stopped Effect recovery includes the observed pending Effect, original reviewed
operational cause, State-owned context and stop reason. It does not assert settlement or request
automatic retry. `read_run`, `progress_run` and `start_run` preserve that distinction.

`SerializableRunView` renders the current durable state as:

- `runnable`: execution position/visit and tagged advance, retry or restart reason;
- `effect_pending`: execution position/visit and exact retained EffectId;
- `succeeded`: exact contract/value references and raw canonical value;
- `failed`: canonical report reference and raw report containing original cause, applicable root
  mapping, stop reason, execution position and committed recovery usage.

The shipping Portfolio uses NoRecovery/Stop with zero allowances. Operational Read failure is a
durable failed result; it cannot be resumed into a fresh attempt. Cancellation before a conclusion
retains a runnable prefix. Retained revisions are checked on start/list; deleting a revision does
not revoke an admitted run. No provider details or locators enter client models.

The ignored `evm_contract_effect_e2e` integration test is the only app-level composition of the EVM
transaction Effect and anchored transaction-route Read. It compiles a first-party Solidity fixture,
checks fixed fixture selectors against the emitted ABI, admits the deployment transaction directly,
and uses a generated ephemeral signer plus real PostgreSQL authority and Reth. A bounded caller loop
reconstructs Runtime and every IO handle across forced recovery boundaries; the final report decodes
its value from anchored Read evidence. Creation, call, and anchored transition values are public EVM
contracts; the fixture ABI, transaction/observation policy, exact lifecycle Operation, nine State
registrations, report, failure adaptation, and fixed development settlement policy remain test-only.
They add no production component inventory, deployment configuration, CLI, REST, or `Application`
entry point.
