# Persisted and Public Surfaces

Status: current authority and redaction inventory

## Material uncertainties

none

Persisted and public values are strict, bounded, float-free canonical JSON with denied unknown
fields unless the frozen schema explicitly says otherwise. A content reference identifies exact
bytes; it is never bearer authority.

## RunHistory

| Family | Persisted meaning |
| --- | --- |
| `RunAdmitted` | Store/tenant/run/invocation identity, exact certified program and audit refs, immutable admission material, initial lexical bindings, and genesis semantic digest. |
| `StateTransitionCommitted` | Exact occurrence/path/call, input binding, optional consumed observation, nominal outcome, facts, and before/after semantic digests. |
| `ExternalAccessAuthorized` | Exact occurrence, attempt ordinal, semantic head, capability/adapter contracts and implementations, request/digest, public binding, and optional stable resource lineage. |
| `ExternalAccessObserved` | Exact authorization/access-attempt linkage and one closed observation outcome. |
| `RunClosed` | Exact terminal nominal operation-outcome reference. |

Each record is assigned a run sequence, ordinal, and record hash inside a predecessor-linked atomic
batch. A batch also binds append request id, candidate digest, commit digest, store scope, writer
epoch, and the exact new object set. No control record exists for Match, FanOut, lanes, joins,
fragments, handlers, or result expressions.

## Observation outcomes

| Variant | Public/persisted fields | State-consumable |
| --- | --- | --- |
| `Returned` | Exact typed value reference | yes |
| `SafeFailure` | Exact reviewed typed failure reference | yes |
| `SupersededBeforeEntry` | Public lineage-head and evidence references | no; Effect refresh only |
| `EntryUnknown` | Fixed bounded fault code | no; parks Effect |
| `IntegrityFault` | Fixed bounded fault code | no; blocks |

Raw provider bodies, arbitrary diagnostic text, endpoints, credentials, signed bytes, private
sessions, fence keys, and mutation permits are never fields of these records or their object
closure.

## Content-addressed objects

Every `HistoryObject` contains an object-type tag, schema id plus raw canonical-byte digest, and
the exact canonical JSON. The store validates bytes and identity on admission and load. Each append
must introduce exactly the newly reachable closure required by its records—no missing member and no
unreferenced extra object.

The closure includes certified program components, immutable admission roots, lexical values,
state outcomes, facts, access request/response values, and purpose-limited public physical
evidence. Secret-free implementation descriptors identify semantic contract, implementation id,
executable identity, and qualification artifact; they never identify credentials or endpoints.

## Facts

A committed fact binds dense emission ordinal, certified slot ordinal, descriptor, exact typed
subject/response, and claim reference. Facts are transition members, not an independent mutable
table of semantic truth. Backend indexes are rebuildable projections only.

## Configuration history

Configured values use a separate append-only stream keyed by store, tenant, entry operation, and
target. A revision contains sequence, predecessor, append request id, exact configured-value
contract, content reference, canonical bytes, and writer lineage. Application paths can resolve but
cannot append. Configuration is not a RunHistory record family.

## Certified program

The certified root content-addresses the exact authored program, expanded program, expansion
profile/proof, policy proof, state/capability/adapter/signer/resource manifest closure, and
secret-free implementation manifest. Repeated refs in `RunAdmitted` are audit projections and must
equal the certified root. Serialized authored or component bytes have no authority independently.

## Wallet authority

Wallet PostgreSQL persists only current-schema public semantic state:

- permanent chain/domain activation and store-incarnation lineage;
- stable intent/reservation identity and canonical intent closure;
- nonce high water and one incomplete intent;
- bounded candidate family and contiguous activated prefix;
- permanent operation-key results; and
- canonical terminal completion closure.

Target-held private keys, live sessions, transaction permits, database passwords, signer secrets,
signatures, and signed transaction bytes are process/deployment authority and are not persisted in
semantic tables. SQL role and session metadata is infrastructure, not portable semantic evidence.

## Public application DTOs

The reviewed application surfaces are:

- complete entry-point contracts;
- admission response with logical run identity and admission disposition;
- one-action drive response;
- public run view with verified status/head and terminal output when closed;
- fixed-head transition-trace pages;
- fixed-head access-audit pages;
- callback-free replay results; and
- streamed current structured portable exports.

All JSON rendering uses a strict one-current schema. Cursor and content-reference strings are
opaque identity, not authorization. Every protected method independently authenticates and
authorizes its exact purpose and tenant/run target.

## Portable export

The current media type is:

```text
application/vnd.mfm.structured-run-export-stream.v2
```

Each newline-delimited canonical frame binds an ordinal, kind, predecessor digest, and payload. The
terminal seal binds version, requested semantic/audit kind, every run fixation, source closure,
per-run authenticated principal, fixed `export` grant, content-addressed policy-decision
references, assigned prefixes, chain digest, and exact byte/frame counts. Replay input checks the supplied
content digest, exact current recoverability schemas, store/tenant/run identity, and per-frame and
total bounds. Former monolithic object bytes are rejected; no compatibility decoder exists.

Semantic exports authorize only the exact selected cutoff and recursively required producer heads;
later audit-only suffixes are not semantic dependencies. The retained decision references are
opaque content-addressed policy evidence, and offline verification accepts them only when its
explicit trust snapshot binds the exact closure digest.

## Error and logging contract

CLI/REST/application errors use a non-empty code bounded to 128 UTF-8 bytes and a non-empty reviewed
message bounded to 4096 UTF-8 bytes. Transport class is carried by HTTP status or process exit
status, not JSON. A Runtime-classified error may additionally carry the frozen, secret-free
`runtime_fault` attribution: phase, run, nullable verified pre-fault head, nullable occurrence, and
either the semantic process contract or store scope and epoch. This attribution is public context,
not persisted history or bearer authority; it excludes private implementation identity and
diagnostics. JSON uses `mfm.public-error.v1` inside `mfm.error-response.v1`.

Typed library errors may preserve internal sources, but the public boundary classifies and redacts
them. Logs contain route, method, status, bounded latency, and reviewed operational identifiers
only. They must not contain:

- credentials, passwords, mnemonics, private keys, API tokens, or headers;
- endpoints, filesystem paths, database URLs, raw SQL parameters, or provider response text;
- signature material or signed transaction bytes; or
- arbitrary `Debug` output from secret-bearing/private authority values.

## Review checklist

For every new persisted or public field:

1. identify its semantic owner and exact schema;
2. prove it is canonical, bounded, float-free, strict, and content-addressed where required;
3. prove it is public evidence rather than a disguised credential or bearer;
4. add hostile unknown-field, contract/provenance substitution, and redaction tests;
5. update recoverability annex/corpus and regenerate them in place; and
6. reject old bytes rather than adding a compatibility path.
