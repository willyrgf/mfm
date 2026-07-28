# Portfolio Snapshot

MFM publishes exactly one product entry point:

```text
entry_point_id           = "mfm.portfolio/snapshot@1"
entry_point_operation_id = "mfm.portfolio/snapshot"
```

Its exact content-addressed `PlanningProfile` has one planner contract, one planner implementation,
an empty ordered `framework_policy_refs` list, and an empty canonical parameter object. Generic
authoring helpers do not grant admission authority.

## Admit And Drive

Deployment administration must provision a canonical `PortfolioConfig` under its intrinsic
`portfolio_id`; the runtime application exposes no configuration mutation surface. Admission
resolves that stable target under the caller's exact tenant and entry point and requires a
caller-generated canonical lower-case hyphenated UUIDv4:

```sh
mfm run admit mfm.portfolio/snapshot@1 \
  --invocation-identity de305d54-75b4-431b-adb2-eb6b9e546014 \
  --target acme/primary \
  --access-token-file /path/to/token
```

Admission:

1. authenticates and authorizes `Admit`;
2. resolves the target's current canonical configuration;
3. verifies schema, digest, embedded portfolio id, and bounded aggregate demand;
4. deterministically authors and expands the graph;
5. certifies the exact planning profile, graph, implementations, terminal contract, and public
   output;
6. binds immutable non-secret routing-generation references; and
7. appends `RunAdmitted`.

It performs no EVM, Bitcoin, provider, signer, or executor semantic call and does not drive the
graph. Repeating the exact invocation and root attaches to the same run; changed root material is
an admission conflict.

Execution is invoker-driven one action at a time:

```sh
mfm run drive RUN_ID --access-token-file /path/to/token
mfm run show RUN_ID --access-token-file /path/to/token
```

Any authorized process can issue the next `drive` call. Runtime reconstructs all semantic state
from the certified graph and verified journal; no process-local run state is required.

## Current Production Graph

The portfolio operation projects explicit wallet-to-symbol demand into ordinary typed source
graphs and a final portfolio graph. Same-run source values travel through graph edges; the report
does not query a store-global fact surface to recover values produced by its own run.

For each demanded EVM network, the certified graph is:

```text
audited routing-generation/source/chain bootstrap
  -> audited initial number/hash anchor
  -> independently audited fan-out:
       native balance reads
       ERC-20 metadata reads
       ERC-20 balance reads
  -> audited final number-to-hash confirmation
  -> pure typed EVM aggregation
  -> portfolio assembly
  -> public report projection
```

Every external JSON-RPC operation has its own `ExternalAccessAuthorized` and
`ExternalAccessObserved` records. Every request derived from another response is linked by a typed
graph edge. Independent fan-out nodes may run in any process and settle in certified node order;
one missing or cancelled call cannot disappear inside an aggregate capability result.

All anchored reads use the exact EIP-1898 block hash with `requireCanonical: true`. The final
number-to-hash observation must match the initial anchor. The pure EVM aggregation state validates
complete demand, deduplicated metadata, exact source/chain/anchor agreement, canonical quantities,
and one result per certified request before producing typed balances and transition facts.

The portfolio graph consumes those typed outputs directly, rechecks exact config-derived coverage,
constructs wallet snapshots, and produces the public output. Transition facts remain available for
explicit future certified cross-run selection, but they are not an indirect wiring mechanism for
this run.

## Bitcoin Availability

Bitcoin collection is not registered in the production state/capability catalog. A portfolio
configuration that requires Bitcoin cannot be admitted as an executable current product graph.
There is no aggregate-reader fallback.

The prospective Bitcoin graph and its repeat-work-safe qualification requirements are documented
in `docs/btc-rpc-routing.md`. Registration requires a separate reviewed change after those gates
pass.

## Facts And Prior-Run Selection

An ordinary portfolio snapshot does not browse or select arbitrary prior facts. If a future
certified operation deliberately consumes a prior-run observation, it authors one
`FactSelectionRequest` and uses the reserved audited `mfm.journal.fact-selection.v1` capability at
an exact tenant frontier. The returned response is interpreted by its state callback and recorded
as normal read evidence.

There is no public fact command, route, catalog, or object reader.

## Public Output

The certified root binds one `PortfolioPublicOutputs` value with exactly:

```text
snapshot
report
```

Both use `schema_version: 1`. They preserve explicit zero holdings and reviewed quote totals. They
do not expose:

- transition or record identities;
- provider/source routing;
- capability requests or observations;
- fact or artifact references;
- executor evidence;
- internal anchors beyond reviewed public portfolio pins; or
- credentials and secret-bearing configuration.

`mfm run show` and `GET /v1/runs/{run_id}` return status plus this certified public output under
`ReadPublic`. Trace, audit, replay, and export require separate grants.

## Replay

Recorded verification checks the complete transition graph, exact inputs and outputs, EVM access
audit, facts, object bindings, and closure without callbacks or live IO. Exact reproduction reruns
the admitted pure request/reduction and portfolio computations using only retained values.
Candidate comparison uses only the self-attested current candidate catalog.

No replay mode resolves routing, opens JSON-RPC, reads current configuration, invokes an executor,
or appends to the run.
