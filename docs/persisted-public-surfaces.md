# Persisted And Public Surfaces

This is the review inventory for every value MFM persists or returns through app, CLI, or REST.
`docs/design.md` owns semantic authority; the frozen annex owns exact recoverability encodings.

No surface in this document may contain a password, mnemonic, private key, credential,
authorization header, unlock material, raw signature, signed bearer payload, secret-bearing path,
RPC URL, provider response body, provider message, or unreviewed diagnostic text.

## Authority Classes

- **strict journal authority** — trusted only after canonical, identity, predecessor, object,
  certificate, and fold verification against the exact run.
- **strict executor authority** — trusted only inside the exact executor tenant/deployment/resource
  binding and its independently durable ledger.
- **pre-admission authority** — mutable current configuration that can influence only a future
  root; selected values become immutable root material at admission.
- **audit provenance** — reviewed non-secret information explaining an authorized access; it grants
  no replay, retry, source selection, or object access.
- **operational telemetry** — best-effort health/log/metric data with no semantic effect.
- **public representation** — deliberately disclosed DTO or export data derived under an exact
  purpose grant; it is not bearer authority.

## Journal And PostgreSQL

| Surface | Physical location | Secret boundary | Authority and validation |
| --- | --- | --- | --- |
| Store identity | `store_identity` | Non-secret store scope and epoch only. | Immutable strict store authority. A reset uses a never-reused scope and fresh epoch. |
| Schema contract | `store_schema_metadata` | Non-secret version/contract data. | Store-open authority checked before any read or append. |
| Native run commits | `journal_commits` | No secrets. Contains run/tenant, sequence, predecessor, candidate/commit digests, append id, batch purpose, tagged fact coordinate, count, and coarse operational commit time. | Strict journal authority. Exact predecessor and commit digest form the run head. |
| Native run records | `journal_records` | No secrets. Contains one of the five canonical payloads plus identities, logical key, ordinal, schema/spec binding, and fact-routing fields. | Strict journal authority only as part of its verified containing commit. |
| Immutable object bytes | `artifact_blobs` | Typed non-secret bytes only. | Raw content address is SHA-256 of exact bytes; bytes alone grant no run access. |
| Object admission evidence | `artifact_admissions` | Non-secret schema, semantic type, digest, length, media type, role, and evidence identity. | Strict object evidence after exact byte verification. |
| Journal-to-object reachability | `commit_artifact_bindings` | Non-secret `ValueRef` material and field path. | Strict authority tying every required object to one exact producing or consuming commit. |
| Tenant fact frontier | `tenant_fact_order_heads` and tagged commit coordinates | Tenant id and dense unsigned order only. | Strict same-store fact completeness. Publication increments once; a selection barrier snapshots without incrementing. Direct arbitrary writes are forbidden. |
| Current configured values | `configured_values` | Canonical non-secret semantic config only. | Pre-admission authority keyed by stable target. A run never rereads it after root admission. |

Every authority-bearing read and write uses one qualified fenced authoritative PostgreSQL writer.
The application role may insert/select through the store contract but cannot update, delete,
truncate, or directly manipulate immutable rows, store identity, or tenant fact heads. A replica,
backup clone, cursor, or apparent applied position cannot mint v1 store-backed authority.

The in-memory store has the same logical surfaces. It validates one complete candidate in scratch
state and performs one infallible swap only after every check succeeds.

## Five Journal Records

| Record | Retained data | Authority role |
| --- | --- | --- |
| `RunAdmitted` | Run/store/tenant identity, canonical invocation identity, entry point, planning profile, authored/expanded graph, certificate, config/seeds, cross-run source manifest, routing-generation refs, executable and implementation manifests, genesis digest. | Establishes the immutable root. Admission performs no semantic live IO. |
| `StateTransitionCommitted` | Node occurrence, transition kind, before/after state, exact input lineage, request/selected observation where applicable, result, outputs, facts, evidence, typed failure or blocking sources, binding delta. | The only semantic state change. |
| `ExternalAccessAuthorized` | Exact read or ensure scope, immutable request identity/value, capability/operation binding, and optional effect/executor binding. | Authorizes zero or one application-protocol operation and changes no semantic state. |
| `ExternalAccessObserved` | Exact authorization reference and one `Returned`, `DidNotEnter`, or `Indeterminate` typed outcome. | Audit evidence only; state changes only if a later transition consumes it. |
| `RunClosed` | Final transition reference and terminal semantic-state digest. | Structurally fixes closure in the same commit as the final transition. |

`JournalHead` advances for every commit. `SemanticHead` advances only for admission or a semantic
transition. A legal late observation after closure advances only the journal head.

## Retained Typed Values

| Surface | Secret boundary | Authority role |
| --- | --- | --- |
| `mfm_ids::ContentRef` | Schema id and raw-byte digest only. | Lightweight content identity; never journal reachability or access authority. |
| `mfm_journal::ValueRef` | Full reviewed producer binding, role, schema/semantic type, digest/evidence, length, and media type. | Exact retained journal identity when reachable through a verified commit binding. |
| Config and seed objects | Canonical typed non-secret values. | Root authority after admission binding. |
| State request objects | Canonical typed non-secret requests. | Immutable state intent. A read request becomes committed by authorization; an effect request by `EffectRequested`. |
| Observation objects | Reviewed typed result or closed `SafeFailure`. | Audit evidence selected by an exact authorization. |
| Output and fact objects | Canonical typed non-secret values. | Semantic result only through a verified transition binding. |
| Typed failure objects | Closed domain failure values without provider diagnostics. | Committed domain truth for one terminal state. |
| Public-output objects | Only fields approved by the certified public schema. | Strict source for `read_public_run`; rendered JSON is a representation. |

Objects referenced by committed authority are retained indefinitely in v1. Garbage collection is
not a semantic workflow and cannot delete a reachable object.

## Facts

A fact exists only as a typed emission inside `StateTransitionCommitted`. It contains complete
canonical subject material, response value, descriptor/logical/content identities, and producer
binding. Query terms are deterministic searchable projections of the retained subject; they do not
replace identity.

Same-run state data uses graph bindings. A prior-run selection records:

1. `ExternalAccessAuthorized` for `mfm.journal.fact-selection.v1` at one tenant barrier;
2. `ExternalAccessObserved` containing the complete typed `FactSelectionResponse`; and
3. `ReadSettled` consuming that exact observation.

Private scan state, pagination, scratch material, and index rows are not persisted or public
authority. Only exact coverage through the authorization barrier produces
`FactSelectionCompleteness`.

There is no ordinary public fact DTO. Facts can appear only through separately authorized trace,
replay, audit, or export closure when that surface's disclosure contract permits them.

## External-Access Evidence

The generic `SafeFailure` envelope retains:

- selected safe-failure contract reference;
- closed stable code;
- closed failure class;
- `before_boundary_entry | boundary_entry | boundary_observation`;
- optional reviewed coarse size class; and
- optional canonical typed diagnostic reference, bounded to 16 KiB.

It never retains provider-controlled strings or arbitrary maps.

EVM read audit records may retain exact reviewed HTTP status, JSON-RPC numeric code, or closed
response-invalid discriminator through the EVM diagnostic union. Semantic source mismatch and
anchor change are typed returned values interpreted by the state, not provider diagnostics.

### Transient EVM transport ownership

The exact-generation EVM transport changes no retained surface. MFM-owned authorization values,
encoded request bodies, signed envelopes, response buffers, decoded error messages, and
secret-bearing fixture captures are transient bounded zeroizing owners. Typed decoding borrows from
the response owner; only reviewed typed results or closed safe failures may leave it. Provider text,
error data, authorization, and raw signed bytes are discarded before any executor or journal value
is built.

The live wallet qualification privately retains a clone of the exact transport runtime and route
catalog so execution cannot inject a parallel transport. That process-local handle is excluded from
the qualification's secret-free canonical proof, content reference, and debug representation; it
creates no persisted or public surface.

The zeroization guarantee ends at allocations directly owned by MFM. HTTP/TLS libraries, allocators,
the operating system, and remote peers can maintain internal transport copies outside that
guarantee; none of those copies is a persisted or public MFM surface.

Production EVM portfolio reads retain separate authorization/observation pairs for:

- routing-generation/source/chain bootstrap;
- the initial anchor;
- each independently meaningful token metadata, native balance, or token balance call;
- final anchor confirmation.

Pure aggregation consumes the typed graph outputs and may emit final facts and portfolio values.
There is no aggregate multi-call observation that hides sibling calls.

Bitcoin collection produces no current product records because its capability is unregistered.
Its prospective audited surfaces are described in `docs/btc-rpc-routing.md`.

## Executor Surfaces

| Surface | Secret boundary | Authority role |
| --- | --- | --- |
| Committed executor request | Exact canonical safe request identity; no credential or bearer material. | Identity only; cannot enter a target. |
| Delivery authorization | Tenant/deployment/effect/attempt and exact request digest. | Strict executor authority that precedes one target call. |
| Target receipt and observation | Closed returned/did-not-enter/indeterminate outcome with reviewed safe result/failure refs. | Exact evidence for the committed delivery authorization. |
| Delivery frontier and tombstone | Bounded predecessor-linked audit, exact terminal proof, assurance-policy ref. | Strict executor terminal evidence. |
| Typed resource stream | Resource ownership/key, policy/config refs, immutable allocation state. | Executor-private resource authority. |
| EVM wallet request | Exact tenant, target, chain, public account, signer binding, policy, and unsigned transaction intent. No secret selector, key material, or signature. | Immutable effect identity and target intent. |
| EVM wallet candidate | Exact public unsigned transaction fields, fee ordinal, allocated nonce, and signed-transaction hash. No signature or raw signed bytes. | Recoverable public candidate; permits hash lookup and finality recovery without reopening the signer. |
| EVM wallet attempt evidence | Exact operation, request/result refs, and closed returned/did-not-enter/indeterminate classification. Provider diagnostics and credentials are excluded. | Audited evidence for one authorized target exchange. |
| EVM wallet terminal evidence | Exact accepted transaction hash, inclusion/finality proof refs, and closure outcome. | Content-addressed terminal proof consumed by the journal only through audited ensure. |
| PostgreSQL executor binding | One tenant, executor binding, durable generation, evidence authority, and optional resource owner in the dedicated executor schema. | Immutable strict executor authority admitted only after the independent deployment fence succeeds. |
| PostgreSQL executor records | Immutable effect frontiers, resource records, exact effect/resource links, and content-addressed closure objects. | Raw executor authority accepted only after opaque decoding and shared-engine strict refold. |
| PostgreSQL executor heads | Derived effect/resource views over immutable records. | Rebuildable acceleration only; never append, retry, target-entry, or recovery authority. |
| Memory/file checkpoints | Checksummed bounded encodings. | Qualification only; no production freshness or non-rollback authority. |

Executor delivery attempt identifiers remain valid inside this ledger only. They do not represent a
state transition or run phase.

The MFM journal retains executor evidence only after ordinary object admission through an audited
ensure observation. An executor terminal claim contains `ContentRef` values and cannot create
producer-bound journal references or append a run.

## Configuration And Routing

Deployment-provisioned current configuration contains domain intent and non-secret references.
Admission resolves one exact tenant-, entry-point-, and target-scoped canonical value, records its
complete producer-bound authority and bytes in the root, and never consults current configuration
again for that run. Runtime app and transport surfaces cannot publish, list, or export configured
values.

Runtime routing may contain RPC endpoints, authorization sources, process-local signer selectors,
keystore paths, unlock-file paths, or other process-local resources. Those values never enter a
typed semantic surface. Admission binds only immutable non-secret routing-generation and wallet
signer-binding references. The wallet request also fixes its expected public account, while the
qualified signer resolves the process-local selector and secret sources only behind the guarded
target boundary. Bootstrap resolution and source validation occur after admission through audited
access; resume resolves the exact admitted generation without fallback.

## Public App, CLI, And REST DTOs

| Surface | Disclosure |
| --- | --- |
| Entry-point discovery | Exact entry-point/profile/input/output contract; no tenant or run data. |
| Admit response | Run id, admission result, entry-point identities, invocation identity, planning-profile ref. |
| Drive response | `advanced`, `waiting`, or `closed` plus only the frozen head/closure fields. |
| Public run view | `active`, `succeeded`, or `failed`; reviewed active fields; certified public outputs. |
| Replay response | Frozen verified, reproduced, or candidate-comparison result. Reproduction `unavailable` has no reason field. |
| Transition trace | Separately authorized exact transition lineage and retained values; cross-run denial uses redacted lineage. |
| Access audit | Separately authorized safe authorization/observation chronology. |
| Portable export | Framed `mfm.portable-run-export-stream.v1` JSON text sequence plus one external `ContentRef`. |

The portable stream begins with one header, emits root-first run material and deduplicated object
payloads with every logical `ValueRef` authority, and ends with one terminal frame followed by
EOF. It contains no self-digest. REST streams those exact bytes and returns SHA-256 over every
record separator, canonical frame byte, and line feed only as external `Mfm-Content-Digest`
metadata. CLI streams the same bytes to a secure same-directory temporary file and publishes it
with its exact canonical `ContentRef` sidecar only after both files are durable.

The public transport surface is limited to entry-point discovery and exact-run admit, drive, show,
replay, trace, audit, and export operations. A run id, record ref, value ref, digest, cursor,
portable stream, or export content reference is not bearer authority.

## Operational Surfaces

Health, readiness, logs, metrics, spans, wake hints, internal queues, and advisory cursors are
operational telemetry. They may be lost, duplicated, rebuilt, or stale without changing semantic
truth. They cannot schedule, authorize, settle, skip, close, prove completeness, or grant object
access.

## Review Checklist

For every persisted or returned field:

1. Identify its exact owner and authority class.
2. Prove its schema is closed, canonical, float-free, and bounded.
3. Prove its producer binding and reachability.
4. Reject all secret classes and provider-controlled diagnostic text.
5. Verify tenant, store, run, and purpose authority before dereference.
6. Ensure no representation or routing/index row substitutes for the journal/executor source.
7. Add positive and adversarial redaction, tamper, and wrong-authority tests.
