# Recoverability Predicate Owners v1

Status: current ownership map

The exact machine-readable clauses are in `contracts/recoverability/v1/annex.json`. This map names
one primary owner and every enforcing boundary for each frozen predicate family.

## Material uncertainties

none

| Predicate | Rule | Primary owner | Enforcement |
| --- | --- | --- | --- |
| `P-HB-01` | Canonical JSON, schema ids, domain envelopes, content identity, limits, and strict decoding are one current float-free contract. | `mfm-canonical`, `mfm-ids`, `mfm-values` | Derivation constructors, annex/corpus validators, every persisted/public decoder, store object validation, replay/export. |
| `P-CA-01` | A retained value contract and component evidence descriptor bind the complete exact schema/shape and cannot be substituted by structurally similar bytes. | `mfm-values`, `mfm-capabilities` | Program registration, certification manifests, state/capability validation, store lexical/object closure. |
| `P-CF-01` | Purpose-limited prior-run fact selection binds an exact typed query/request and tenant frontier. | `mfm-facts`, store scanner port | Capability registration, Runtime authorization, scanner completeness proof, callback-free fact projection. |
| `P-FA-01` | Fact-query identity and request semantics are deterministic, bounded, ordered, and tenant scoped. | `mfm-facts`, `mfm-canonical` | Domain identity derivation, query decoder, purpose-limited scanner, corpus vectors. |
| `P-RH-01` | Run id, five-family logical keys, exact-head batches, closure, and portable history identity are one append-only structured contract. | `mfm-journal`, `mfm-store` | Store candidate validation, sole fold, memory/PostgreSQL CAS, Runtime mutation, replay/export, relational tests. |
| `P-AP-01` | Entry/admit/drive/public/replay/export DTOs are strict, reviewed, tenant-authorized, and secret-free. | `mfm-app` | App constructors and renderers, CLI/REST decoders, access policy, annex/corpus and integration tests. |

## Cross-boundary invariants

### Canonical identity

Semantic identity is `SHA-256(JCS({domain, value}))` under an exact registered domain and preimage
schema. Content identity is `SHA-256(exact retained bytes)`. Neither can substitute for the other.
Floats, duplicate keys, alternate envelopes, unregistered domains, and compatibility decoding are
illegal.

### Structured program authority

Authoring bytes have no authority until the qualified registry performs pure expansion and
certification. The certified root binds the complete component closure. Store admission extracts
the authored object, re-certifies it, and requires the exact root and document. Runtime cannot add
or repair control structure.

### Run history

The only logical keys are admission by run, transition by occurrence, authorization and
observation by access attempt, and closure by run. Admission, transition, authorization, and
observation are the only append candidates; closure is adjacent to admission/transition when the
root becomes derivable. Records and exact object closure are atomic.

### Access

Runtime alone turns a committed authorization into affine live authority and a normal completion
into pending observation. The store verifies exact request, binding, attempt, observation linkage,
and cursor legality. Public certificates are evidence only. Possible entry and integrity evidence
cannot become a returned value or safe failure.

### Configuration

Configuration uses a distinct append-only stream with exact predecessor and contract. Deployment
owns append authority; the app resolves only. A stale predecessor is rejected and old single-row
configuration bytes have no reader.

### Wallet authority

The wallet store is a separate narrow cross-run authority. Its activation and mutation protocols
use permanent semantic operation keys, one qualified domain/lineage, current target sessions, and
fresh transaction-bound permits. Copying public data or a database cannot construct a valid target
session. RunHistory never queries wallet tables.

### Public app surface

Credentials are consumed per purpose. Tenant equality is checked after store verification. DTOs,
page cursors, ids, content refs, and exports convey no authority. Errors and logs retain only
reviewed public codes/fields.

## Evidence locations

| Boundary | Primary evidence |
| --- | --- |
| Annex generation and artifact identity | `contracts/recoverability/generate.py`, canonical/ids recoverability tests |
| Structured authoring and certification | program compile-fail tests; certify structured golden/hostile tests |
| Five-family fold and atomicity | store unit tests; PostgreSQL structured-history qualification |
| Runtime affine access and ambiguity | `crates/kernel/runtime/tests/structured_runtime.rs` |
| Replay/public wire | replay and integration wire-contract tests |
| Wallet activation, roles, fencing, and nonce linearizability | `crates/storages/evm-postgres/tests/wallet_authority.rs` |
| End-to-end structured EVM submission | `tests/integration/tests/evm_postgres_submission.rs` |
| Metadata/deletion contract | integration cargo-metadata and legacy-surface tests |

No removed runtime path, retired schema reader, or generic resource lifecycle is an enforcing
boundary. If a predicate gains a new consumer, update the annex owner list and this map in the same
change.
