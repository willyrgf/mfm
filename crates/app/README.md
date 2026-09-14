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
`mfm.portfolio/snapshot@1` or `mfm.portfolio/enrich@1`; the route array has 1–64 entries, is strictly chain-ordered, and must
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
and stopped Runtime invocations. Ambiguous appends retain the complete `InvocationFailure`, including the original cause and exact
candidate in native custody. JSON carries `recovery` and `invocation`; its optional `last_observed`
is historical. Explicitly acknowledged insertion is separate evidence and may be newer than that view.

`RequestError` owns reviewed codes and messages. `SerializableClientError::for_run` fallibly prepares
the shared invocation/recovery detail, preserving a typed projection failure for its caller. An execution-stopped invocation includes its RunId and optional
last observation; stopped Effect recovery includes the observed pending Effect, original reviewed
operational cause, complete executed input and stop reason. It does not assert settlement or request
automatic retry. `read_run`, `progress_run` and `start_run` preserve that distinction.

`SerializableRunView::new` fallibly prepares the current durable state. `StartRunResult::serializable`
uses the same preparation inside its selected-config envelope. Prepared models implement Serialize;
the retained native values do not invoke fallible projectors through Serde. State alternatives are:

- `runnable`: execution position/visit and tagged advance, retry or restart reason;
- `effect_pending`: execution position/visit, exact retained EffectId and latest committed operational
  failure with its complete input/command and recovery decision;
- `awaiting_recovery`: execution position and original failure, with complete Pure input, Read
  input/intent, or Effect input/command/EffectId, plus returned evidence when applicable;
- `awaiting_interpretation`: execution position, EffectId and complete input/command/settlement
  evidence, without claiming interpretation has succeeded;
- `succeeded`: exact contract/value references and raw canonical value;
- `failed`: canonical report reference and raw report containing original cause, applicable root
  mapping, stop reason, execution position and committed recovery usage.

The shipping Portfolio uses Stop with zero allowances. Operational Read failure is a
durable failed result after a separate original-failure commit and recovery decision; it cannot be
resumed into a fresh attempt. Interruption after the original commit leaves `awaiting_recovery`.
An accepted Effect settlement is committed before interpretation and can leave `awaiting_interpretation`. Retained revisions are checked on start/list; deleting a revision does
not revoke an admitted run. Provider failures retain reviewed method, stage, status/code, source
facts and explicit capture omissions in canonical reports. Raw provider messages/bodies and
locators remain excluded from client models.

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

Explicit candidate enrichment uses the same selector, Portfolio config, and routes as snapshot
execution. Each collection requires a native source. The domain retains native sources and tokens
with nonzero anchored balance, preserving order and quotes. `publish_enrichment(name, run_id)` reads
the exact successful output, checks the immutable binding inventory, and imports a snapshot config
with RunId/head/output provenance. It does not rediscover or admit a dependent run.

Imported provenance is verified against the successful enrichment history before new admission.
C0 retains the selected revision identity and full resolved demand. Start first reads the requested
RunId and checks that identity; matching recovery survives config deletion, while conflicts reject.
Read/progress never reload configuration or enrichment history.

Size-limit invocation failures use `size_limit_exceeded` and include
`invocation.size_limit` with `kind`, `resource`, and `limit` (bytes, or frames for
`frame_count`). Measured violations carry `actual`; a serializer stopped at its ceiling carries
`observed_at_least`, never a fabricated final size. The last observation remains historical; oversized inline reports do not
append a terminal conclusion or discard pending Effect authority. Capacity arithmetic overflow
uses `capacity_arithmetic_overflow` without fabricated measurements. CLI uses exit 2; REST uses
422 for these stopped invocations. Response payloads retain the shared full inline report; request
body limits do not impose a response-size limit.


## Report preparation failures

`ReportFailure<T>` retains the existing `RunRequestError`, `RunView` or `StartRunResult` together
with the reporting stage (`prepare`, `encode`, or transport-observed `deliver`), original reporting cause, and any separate failure
of that cause's projection. Its prepared `IncompleteReport` contains `code`, `message`, available
`run_id` and recovery identity, compact `last_observed` head evidence, and explicit
`report_failure.omissions`. Only an explicit Runtime projection failure supplies `acknowledged`;
reading or adopting a head does not prove this invocation inserted it. Original candidate bytes stay
in native custody and never enter this JSON surface.

An incomplete error report keeps the original error category. A successful observation that cannot
be rendered uses `report_render_failed`. Omission fields name the unavailable detail and distinguish
`projection_failed`, `encoding_failed`, `delivery_failed`, and `bound_reached`; the last carries
the actual limit and either a measured size or an observed lower bound. A failed secondary projector is retained and never retried. Encoding
the incomplete report uses only prepared fields and cannot invoke that projector again.

This custody is local to the invocation. It does not append an internal fault, establish durable
failure auditing through a failed Store, or establish delivery. Native projections and individual
values retain the 32 MiB Values ceiling; the derived FailureReport retains its own 32 MiB ceiling.
`encode_response` encodes prepared transport fields without a new whole-response quota. Transports
own write/flush failures and response handoff; they must retain the reporting failure if that final
encoding also fails, without recursively attempting another JSON report.

Runtime persists one current operation and derives its public phase. Terminal reports borrow the
retained original and mapped domain root; pending recovery reports borrow the unchanged input,
command and original from their observed Effect. Their JSON field names and status rules remain
unchanged. There is no duplicate owned incident or terminal cause tree in the public library API.
