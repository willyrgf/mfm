# RFC part 2: preserve errors on selected execution paths

Status: **complete and accepted, 2026-09-14**. R1 run-Store, R2 authority/schema and R3
executing-signing cutovers are implemented. B1–B8 evidence, aggregate architect acceptance and
exact-candidate final CI are recorded below.
Continue from accepted Part 1 production `87f198a9` and completion record `0a7543ec`,
with this RFC. Do not restart or reopen that cutover.

The [Part 1 completion record](RFC_AUDITABILITY_ERROR_CHAIN.md#part-1-completion-record--2026-09-14)
and [acceptance evidence](docs/measurements/auditability-core-acceptance.md) own the inherited
guarantees. Production baseline is 27,026 lines under that evidence's counting convention;
original `7f71beef` is 26,831. Measure Part 2 separately and cumulatively.

## 1. Objective and inherited contracts

Preserve causes before the selected MFM producer conversions discard them. Use the two existing
receiving routes:

```text
execution signer / authority failure
  -> concrete transaction operational error
  -> Failure/Object admission and append
  -> same concrete error restored for classification
  -> existing App observation and client presentation

run Store failure / internal authority failure
  -> existing concrete Runtime error / immutable InvocationDiagnostic
  -> invocation report only
```

The classifier reads typed semantic alternatives, not diagnostic JSON. Internal failure adds no
history entry; explicit resume retries from the last committed state. Accepted Effect settlement
remains committed before interpretation. Failed recording retains its cause independently of an
admitted execution failure and does not prove it recorded either one.

| Inherited owner | Reuse; no replacement |
| --- | --- |
| [Values diagnostics](crates/kernel/values/src/diagnostic.rs) | DiagnosticEvidence::from_value/as_value: one transparent nested JSON field, diagnostic_float_free profile, no standalone MfmValue. InvocationDiagnostic::from_fields: immutable invocation data with the accepted two encoding-failure markers. |
| [Values Object](crates/kernel/values/src/object.rs) | The only executable/persisted value eraser, at existing heterogeneous Runtime boundaries. Concrete errors stay concrete; ordinary whole-owner/Object admission and restoration apply. |
| [Runtime](crates/kernel/runtime/src/state.rs) | One RunRecord and borrowed continuation selection, without another state, native cache, history fold or recovery replay. |
| [Recording/reporting](RFC_AUDITABILITY_ERROR_CHAIN.md#9-boundary-adaptation-and-invocation-diagnostics) | Existing Failure, direct RecordingFailure variants, boxed concrete RuntimeError causes, acknowledgement facts and finite App/CLI/REST presentation. Store errors return without a probe; only NotInserted probes. No consumer recapture. |
| [Store](crates/kernel/store/src/lib.rs) / [Journal](crates/kernel/journal/src/lib.rs) | Admission/latest/optional-probe snapshot, atomic exact-head append, opaque frame v6. No new query protocol or historical validation. |

The [diagnostic trust boundary](RFC_AUDITABILITY_ERROR_CHAIN.md#4-error-preservation-and-the-diagnostic-trust-boundary)
applies to selected SQLx and signing messages too. Trust dependency-supplied diagnostic text;
do not scan or certify it. Do not deliberately attach MFM secret inputs, complete requests,
connections, keystore commands or panic payloads. Actual Object/frame/run/report limits remain;
add no diagnostic quota, truncation or omission framework.

First-original encoding failure, malformed stored framework data and terminal delivery keep
Part 1's contracts. Source_cycle retains its accepted meaning: repeated complete error-interface
pointer detection, not concrete-object identity.

### Completed work subtracted

Former O1 is complete: shipping Reads, JSON-RPC send/body/response causes, method/stage, supplied
messages/data_json and exposed source chains use the accepted provider carrier. The provider
branch of former O2 already forwards that carrier with its transaction operation. Part 1 E1-E6,
C1-C18 and terminal reporting are retained behavior, not another migration backlog.

Only O2's authority/signing receiving conversions remain. They belong in their producing
cutovers, not a later “complete all consumers” phase.

### Inherited example: an RPC request receives HTTP 429

The existing [HTTP-status branch](crates/live/evm/src/json_rpc.rs) returns a declared operational
provider error. For eth_getBalance, its representation is:

```rust
let error = EvmOperationalError::new(
    EvmOperationalKind::RateLimited,
    ProviderFailure {
        method: EvmRpcMethod::GetBalance,
        stage: RpcStage::Status,
        failure: ProviderFailureKind::HttpStatus,
        diagnostics: DiagnosticEvidence::from_value(serde_json::json!({
            "response": { "status": 429, "rpc_code": null },
            "sources": []
        })),
    },
);
// The adapter returns this concrete error through its existing operational branch.
let adapter_error = AdapterError::Operational(error);
```

RateLimited, GetBalance, Status and HttpStatus are concrete typed alternatives. DiagnosticEvidence
contains the observed status and available source data; it does not replace the classifiable
EvmOperationalError. The Box inside that error contains a concrete ProviderFailure, not dyn Error.
An HTTP rejection need not have a native client error: sources is empty here because the client
received a response. Do not invent another cause to populate it.

Runtime admits the complete original through Failure/Object and commits it before classification.
Cold restoration reconstructs EvmOperationalError with the same diagnostic data. For this Read,
classify() returns Retryable; the handler and allowances decide whether recovery retries.
RateLimited does not itself trigger a retry, sleep or a new adapter request.

For eth_sendRawTransaction, the same provider cause is nested in
EvmTransactionOperationalError::Provider with operation Submit. That enclosing error retains the
current OutcomeUnknown classification and command-authority semantics.

The current non-success HTTP-status branch returns before reading the body and does not capture
Retry-After. rpc_code: null means no JSON-RPC code was captured, not proof that the response body
contained none. Supplied message/data_json preservation applies when the JSON-RPC error envelope
is actually parsed. Body/header capture on HTTP rejection is not a guarantee of this example
or an additional Part 2 producer requirement.

## 2. Finite scope and stopping rule

| Slice | Selected producers | Required receiving boundary |
| --- | --- | --- |
| R1 / former O4 | PostgreSQL run load_run/append_run, their transaction/query helpers and precommit/COMMIT mappings; existing run-Store blocking/allocation helpers in PostgreSQL and MemoryStore | StoreError -> existing Runtime load/recording error -> App/client report |
| R2 / former O6 | PostgreSQL execution authority load/reserve_or_compare/retain_prepared and the finite helpers in section 5 | AuthorityError -> Live map_authority_error -> operational original or internal invocation |
| R3 / former O3 | Executing KeystoreSigner::sign, private Keystore::sign and Command::Sign reply | SigningError -> Live prepare_transaction -> SignerUnavailable original |

These are producing calls, not whole modules to audit. Changed public error types require their
existing constructors, matches and tests to compile and forward correctly. That does not authorize
changing another producing API. Required facts stop at the concrete error actually returned by
each named call; do not reconstruct data erased inside an excluded API.

Excluded: configuration/index error enrichment; connection gates, pool policy, bootstrap and
provisioning; key import/startup/shutdown; general signing/identity constructors; deployment,
entropy, request ingress, transport lifecycle; provider/HTTP, nonce/replacement, crypto or keystore
concurrency redesign. A shared helper's excluded caller does not bring its callers into scope.
Third-party signer implementations are not a migration target.

Comparison-only NotInserted and local corrupt-state rejection keep their current cleanup behavior.
Their ignored rollback errors remain outside the selected guarantee; add no diagnostic success
result or changed adoption semantics. If insert_frame/update_head already failed and its explicit
rollback also fails, preserve that secondary failure as specified below. Background/drop cleanup
and hidden pool attempts cannot be reported as observed failures.

Record a newly discovered loss once in the existing [audit inventory](docs/adapter-error-audit.md);
do not add it to this implementation plan. A new producer, required field, source recipe, public
type/schema family or exceeded estimate requires one cumulative architect decision before expansion.
A compiling caller chain does not prove that another migration is necessary.

## 3. Minimal type changes

### 3.1 Store: data on existing dispositions

Keep the existing size/arithmetic variants and their payloads. Replace these three unit variants:

```rust
enum StoreError {
    // Existing FrameSize, HistorySize, FrameCount, ArithmeticOverflow unchanged.
    CorruptPhysicalState(DiagnosticEvidence),
    Unavailable(DiagnosticEvidence),
    Indeterminate(DiagnosticEvidence),
}
```

Keep externally tagged snake_case serialization, for example:
`{"unavailable":{"operation":"run.append","stage":"begin", ...}}`.
Remove Copy; retain Clone/Eq/Serialize where supported. Receivers borrow or move.
Store method signatures, AppendResult and RuntimeError remain unchanged. All three payloads are
mandatory because selected SQL errors also enter CorruptPhysicalState. No optional-detail route,
legacy unit variant, backend wrapper or StoreFailureKind is needed.

Local checks supply the existing function/operation and fixed violated-check reason. Keep combined
predicates combined; no per-invariant type or constructor audit. A check without a source invents
none. Test doubles identify injected failures. No default/empty payload may conceal a discarded
SQLx, allocation or task error.

### 3.2 Authority: operational evidence or final internal diagnostic

```rust
enum AuthorityError {
    Unavailable(DiagnosticEvidence),
    Internal(InvocationDiagnostic),
}
```

Keep port method signatures. Remove incidental Copy/Clone/Eq requirements instead of adding them
to InvocationDiagnostic. Delete AuthorityError's Serialize implementation without replacement:
its only production serialization consumer is removed by this cutover. Live moves Unavailable
evidence into the durable owner and forwards Internal directly as
`AdapterError::Invariant(diagnostic)`, without recapturing it.
Remove the AuthorityError serializer-only assertion in Live's error_tests.rs; retain tests of
these two actual receiving branches. The enum needs neither a wire contract nor a persistence identity.

### 3.3 Signing: remove the executing error roundtrip

```rust
enum SigningError {
    Invalid,
    Failed,
    SignFailed(DiagnosticEvidence),
}

// Existing private method and Command::Sign reply use this result.
fn sign(
    &self, slot: KeySlot, digest: SigningDigest,
) -> Result<CompactRecoverableSignature, SigningError>;
// Command::Sign.response:
// oneshot::Sender<Result<CompactRecoverableSignature, SigningError>>
```

Invalid/Failed keep their checked primitive meanings. SignFailed carries execution-stage evidence.
Remove Copy; retain the existing owned-data-compatible traits. No new SignerError or expanded
KeystoreError family is needed. Use one tagged serialization with
kind invalid, failed or sign_failed; only sign_failed has cause. Delete the blanket
unavailable-detail serializer.

Private signing returns SigningError directly through its reply. The signer forwards that result,
adapting only request-send and reply-receive failure. Remove the executing
SigningError -> KeystoreError -> SigningError conversions. Import/start/shutdown retain
KeystoreError, including the import-only checked-error mapper.

Signing and keystore may depend directly on existing mfm-values to construct DiagnosticEvidence.
This adds no persistence identity or IO responsibility. EVM gains no signing/keystore dependency.

### 3.4 Durable transaction owner

```rust
enum EvmTransactionOperationalError {
    Provider {
        operation: TransactionProviderOperation,
        cause: EvmOperationalError,
    },
    AuthorityUnavailable {
        #[mfm(persisted)]
        cause: DiagnosticEvidence,
    },
    SignerUnavailable {
        #[mfm(persisted)]
        cause: DiagnosticEvidence,
    },
}
```

Keep existing snake_case kind tags and Provider's typed source. AuthorityUnavailable remains
OutcomeUnknown; SignerUnavailable remains Retryable. Neither classifier reads cause JSON.
Restoration reconstructs this concrete MFM owner, not a live SQLx/keystore error.
DiagnosticEvidence needs no Error implementation or nominal source wrapper.

Perform one transaction-error schema cutover, v2 -> v3, in R2. R2 includes the small signer
receiving conversion using then-current Invalid/Failed data. R3 adds SignFailed forwarding.
This avoids two schema migrations. A unit signer result is represented honestly as
`{"operation":"sign","stage":"signer","kind":"invalid"}` or failed, without invented deeper causes.

Update affected capability/State ABI expectations and fixtures in the same cutover. This changes
content-addressed Program contracts using the error. Inherit the one-current-schema policy:
v2 contracts are not promised executable by the v3-only assembly. Do not rewrite acknowledged
history or add a compatibility decoder. Unaffected Program schemas and Journal v6 do not change.
Before replacing a deployed assembly, identify any existing v2 runs and resolve their deployment
handling explicitly; this RFC authorizes no deletion or rewriting of those runs.

## 4. R1: run storage and one private PostgreSQL recipe

### 4.1 SQLx extraction

One private helper in crates/storages/postgres/src/diagnostic.rs serves run storage and authority:

```rust
fn sqlx_fields(
    operation: &'static str,
    stage: &'static str,
    error: &sqlx::Error,
) -> serde_json::Value;
```

The producer assembles these fields and any separate rollback failure, then calls
DiagnosticEvidence::from_value exactly once. The helper returns ordinary JSON so this needs no
mutation API on immutable DiagnosticEvidence, serialization roundtrip or second capture.
JSON contains operation, stage and an ordered sources array starting with the SQLx error itself.
Every layer retains its actual Display message in message. Each SQLx layer, including sources[0],
also has kind: the snake_case SQLx 0.9 variant name. A future unrecognized variant uses other
and retains message/sources. Select these additional fields:

| Known layer | Fields when supplied |
| --- | --- |
| SQLx | ColumnDecode: index; ColumnIndexOutOfBounds: index/len; ColumnNotFound: column; TypeNotFound: type_name |
| PostgreSQL Database error | sqlstate, severity, server_message, detail, hint, schema, table, column, constraint |
| std::io::Error | os_kind, os_code |

Null means an optional field was not supplied. Do not add query arguments, SQL statement copies,
connections or rejected rows. Other PostgreSQL fields (data_type, position, internal query/context,
server file/line/routine) are outside this selected contract. No metadata ledger is needed;
do not claim preservation of every native field. server_message is PgDatabaseError::message();
keep it separate from Display, which can add a line suffix. Do not strip that suffix from message
or parse it to reconstruct a field.

Follow one path without Debug dumps, parsing Display or a separate cause tree. At each visited
layer, select the next layer in this order:

1. SQLx::Database: database.as_error(); read PostgreSQL fields on the visited concrete PgDatabaseError.
2. SQLx::Io: the contained io::Error.
3. SQLx Configuration/Tls/ColumnDecode/Encode/Decode/AnyDriverError: the contained source.as_ref().
4. io::Error: its get_ref() child if present, otherwise Error::source().
5. Other layers: Error::source().

Apply Part 1's full-interface-pointer repetition guard before emitting each layer. Retain its
source_cycle meaning; do not compare messages or thin addresses. These branches stay inside
sqlx_fields, without a global downcast registry or generic Values capture API.

| Operation | Stages at the current producing calls |
| --- | --- |
| run.load | begin, isolation, head, dangling_frames, selected_frames, decode_head, decode_selected_frame, commit |
| run.append | begin, isolation, synchronous_commit, advisory_lock, head, target, dangling_frames, decode_head, decode_target, insert_frame, update_head, commit |

Preserve disposition exactly: acquisition/query execution and load COMMIT failures are Unavailable;
existing row try_get failures are CorruptPhysicalState; append precommit write SQLSTATE class 23
is CorruptPhysicalState and other errors Unavailable; append COMMIT database rejection is
Unavailable and non-database failure Indeterminate. Derive disposition from the producing call
and concrete error before producing JSON. Do not infer retry, rollback or candidate absence from it.

For insert_frame/update_head failure, retain an error from the already attempted explicit
rollback in a separate rollback evidence field alongside the primary sources. It is not the
primary failure's source and does not change its disposition. Successful rollback adds no error.
No extra rollback attempt or transaction controller is needed.

Extend the existing mapper, rather than introducing another failure-assembly abstraction:

```rust
fn classify_precommit_sql(
    stage: &'static str,
    primary: sqlx::Error,
    rollback: Option<sqlx::Error>,
) -> StoreError;
```

The two failed-write branches pass their primary and transaction.rollback().await.err().
The mapper selects disposition from primary, assembles sqlx_fields("run.append", stage, &primary)
and optional rollback fields with stage "rollback", then wraps once. With no rollback error,
omit the rollback key. Existing injected precommit callers pass None.

### 4.2 Necessary local constructors

Mandatory Store payloads also affect LoadedRun/MemoryStore and PostgreSQL physical validation,
planning and their existing own_candidate_bytes/run_pure_blocking helpers. Do not merge backends
or refactor planning. Local checks retain their reason; size/arithmetic variants stay unchanged.
Allocation failures retain message and requested byte count; unavailable runtime handles retain
their message; joins retain task stage and cancelled/panicked distinction, never panic payload.
The direct ContentDigest::parse and RunSummary::new mappings retain their returned error and
field/check through the exact leaf recipes below. Do not change those constructors or follow their
callers upstream. These and the row try_get calls above are the complete additional producers
selected by this result-type cutover.

For the already selected ContentDigest::parse, SchemaId::parse, ContentRef::new and EffectId::parse
calls in R1/R2, source is {"kind":"identity_error","message":error.to_string()}.
IdentityError's current Serialize replaces the message with "withheld"; it is not the recipe here.
The returned IdentityError exposes no child source. Preserve its actual Display without attempting
to reconstruct any earlier checked-string error. For RunSummaryError use
{"kind":"run_summary_error","message":error.to_string()}. These are local leaf fields, not new
types or generic capture functions. Do not change the IDs serializer or add rejected input.

Runtime/App matches retain the payload and current public code/status, including special handling
of Indeterminate. Update fixtures/borrowing; add no Store-to-diagnostic recapture or new renderer.

## 5. R2: authority and receiving schema cutover

The shared SQLx recipe uses authority.load, authority.reserve_or_compare or
authority.retain_prepared. Pass that static operation into existing shared helpers where needed;
do not store an operation context or introduce an operation enum.

| Existing producer/helper | Stages or local facts |
| --- | --- |
| Port methods, begin_authority, commit_authority | acquire, begin, isolation, synchronous_commit, advisory_lock, commit |
| load_state, latest_reserved_nonce, insert_reservation, prepared insert | load_state, latest_reserved_nonce, insert_reservation, retain_prepared |
| ensure_reservation and local predicates | fixed violated contract: epoch/binding, missing/partial reservation or prepared record, marker count, zero chain ID, nonce overflow/exhaustion |
| parse_content_ref, EffectId::parse, Reservation::new | helper/field plus section 4.2's IdentityError leaf or the existing serialized EvmDomainError kind; no upstream constructor changes |
| parse_u64; epoch/hash/address byte helpers | parse kind/message or local syntax reason; field and expected/observed length for byte conversion, no rejected bytes |
| ExactRawTransaction::new | local length failure, existing expected/observed lengths, no raw signed wire |

The nonce-domain lock key reads eight bytes directly from its statically sized 32-byte digest.
This extraction is infallible and adds no diagnostic branch.

SQLx failures remain Unavailable, including ambiguous authority COMMIT. Existing nonce exhaustion
remains Unavailable with its own reason. Other local/retained-data failures remain Internal.
Construct InvocationDiagnostic once, code authority_internal, named operation and stage/check/source
fields. For local helpers without port context, operation names that helper; do not extend a public
constructor just to supply the outer port name. Use section 4.2 for IdentityError. Other selected
serializable concrete errors use from_fields directly. Foreign scalar parse/length errors use
the fixed facts above, not a generic serializer family. Forward existing AuthorityError/
InvocationDiagnostic rather than nesting another capture.

Delete unavailable(_: impl Sized) and internal(_: impl Sized); no renamed input-discarding helpers.
The bootstrap caller of epoch_from_bytes keeps its GateError conversion without gaining bootstrap
scope. Pool errors stop at what SQLx returns, including an existing Protocol gate marker; do not
recover earlier discarded gate data.

Live performs only section 3's move. Authority operation/stage already travels inside the original.
Reservation/EffectId, prepared bytes, reconciliation and command-authority behavior remain unchanged.

## 6. R3: executing signing

| Selected producer | SignFailed evidence |
| --- | --- |
| Command::Sign send fails | operation sign, stage request_send, kind channel_closed, actual channel-error message |
| Sign reply receiver fails | operation sign, stage reply_receive, kind channel_closed, actual receiver-error message |
| Private owner cannot find slot | operation sign, stage key_lookup, kind missing_key; no native source exists |
| sign_prehash_recoverable fails | operation sign, stage sign_prehash_recoverable, kind signature_error, actual primitive-error message |

The pinned in-process k256/ecdsa signing path returns opaque signature errors with no child source.
No algorithm-specific detail can be recovered and no signing source walker is needed. This fact
is specific to that producing call, not arbitrary external signer errors.
The checked compact-signature conversion returns its concrete SigningError unchanged; do not
rewrite signature/public-key constructors or recover_public_key.

Live moves SignFailed evidence into SignerUnavailable unchanged. Keep R2's honest Invalid/Failed
representation for an implementation supplying those concrete unit errors. Supplied SignFailed
data cannot become Failed/Invalid or an invocation-only marker. Classification stays Retryable.

Construct channel detail from the error message only, never the SendError's Command. Reply closure
does not establish why the owner stopped. Do not infer a panic or change channel/cancellation/
owner-thread mechanics. Error enrichment does not authorize another sign call or a new command.

## 7. Ordered commits, cost and verification

R0 is complete: the dedicated architect accepted sections 2-6 as one target, including the
immutable-evidence assembly, row-decoding dispositions, finite cleanup limit and estimates below.
Final handoff review also accepted the local identity-error recipe, removal of AuthorityError
serialization, exact source traversal and existing-mapper rollback tests; no core design choice
remains delegated to the implementation.
This accepts the bounded design, not the unimplemented result.
Order: R1 storage; R2 authority plus the single durable schema/receiver cutover;
R3 executing signing; final acceptance. Each cutover includes its receiving conversions, tests
and owning documentation. There is no separate owner-wide consumer completion phase.

Delete superseded mappers/serializers with their replacements. Update the audit inventory's
selected gaps, docs/design.md trust/error wire contract, docs/architecture.md ownership, and
signing/transport documentation with their implemented cutover. Do not describe Part 2 as already
implemented in those documents during R0.

Necessary additions: three existing Store payloads, two authority payload routes, one SigningError
variant, two existing durable owner payloads and one private SQLx recipe. There is no new public
named error type, source trait, reporting service, Runtime layer or value eraser. Actual deletions:
the authority discard helpers, executing signing roundtrip and two unavailable-detail serializers.
Do not credit completed provider/core removals again.

Provisional production estimate: **+350–700 lines against Part 1**, not a promised reduction.
Unit mappers contain little code to delete; the justification is required information retained
using the accepted mechanism. Baseline references span 18 Rust files for StoreError, seven for
AuthorityError and five for SigningError, overlapping and including tests. Consumer churn counts.

The architect's provisional R1-R3 implementation estimates are:

| Category | Net added LOC | Additions + deletions |
| --- | ---: | ---: |
| Production Rust | 350–700 | 850–1,700 |
| Tests and fixtures | 300–700 | 700–1,500 |
| Implementation documentation | 100–300 | 300–800 |
| Manifests/lockfile | 10–30 | 20–60 |
| Total | 760–1,730 | 1,870–4,060 |

Expect 30–45 distinct changed files, including tests/docs/consumers, and one new private PostgreSQL
source file. These ranges estimate the work; they are not targets to consume or a guarantee.
Report R0's RFC/inventory editing cost separately and include it in the final cumulative comparison
against Part 1. Exclude inline tests consistently from production counts and count each file once.

Measure after each cutover, including untracked files and the estimated remaining work.
Before R2, obtain the dedicated architect's review of the actual R1 diff, retained guarantees,
new/deleted responsibilities and forecast for the complete delivery. This is an agent design
checkpoint, not another user approval or an instruction to run full CI early.
An actual or forecast exceeded estimate, extra type/source recipe or failed claimed deletion stops owner
fanout for cumulative design review. Passing tests or small local cleanups do not resolve aggregate
design objections. Do not compress code or weaken coverage to satisfy a line count.

Use [build and verification](docs/build-and-verification.md); direct Rust tools run through
`nix develop -c`. R1 selects Store/PostgreSQL/Runtime/App and affected clients; R2 selects
PostgreSQL authority/EVM/Live and affected ABI consumers; R3 selects signing/keystore/Live.
Use managed postgres-test for real SQLx/COMMIT behavior. SQL text/metadata should not change;
sqlx-prepare is only needed for a separately justified query change. Run one final
`nix run .#ci` on the complete candidate.

### R1 checkpoint — 2026-09-14

R1 continues from R0 `70200449` and implements the selected run-Store boundary. The three
existing dispositions now require evidence; SQLx extraction is private to PostgreSQL. Selected
local checks and allocation/task/identity failures retain facts without another error family or
capture service. Existing SQL, isolation, locking, synchronous COMMIT and ambiguity semantics
remain unchanged. App and clients forward the enriched existing wire; obsolete unit fixtures are
replaced. R2/R3's authority/signing deletions remain outstanding and earn no R1 deletion credit.

The dedicated architect reviewed the actual diff and returned **ACCEPTED** for the bounded design
and revised aggregate forecast. The review required readable multiline JSON and an actual closed
PostgreSQL backend through Runtime::read; both are implemented. The latter preserves the exact
run.load/begin/pool_closed evidence and has no last observation. A previously misleading rejected
COMMIT fixture failed at denied DELETE; its test-only injection now uses an authorized head update
and proves deferred foreign-key rejection at COMMIT with SQLSTATE 23503.

Focused `nix develop -c cargo test --target-dir target/verification` with
`-p mfm-store -p mfm-storage-postgres -p mfm-runtime -p mfm-app -p mfm -p mfm-rest-api --all-targets`
passed. New SQLx regressions cover nested/inline/cyclic sources, IO children, column facts and
separate rollback. Existing Runtime recording tests now assert enriched evidence alongside the
admitted original, submitted candidate and previous observation; the no-original case remains
explicit. Focused App/CLI/REST assertions cover the changed wire. Selected all-target Clippy passes
with `--no-deps -- -D warnings`. Managed `nix run .#run -- --task postgres-test --slot 1` passed
run `run-3362366-1789412275902410274`, including real database fields, identity rejection,
COMMIT outcomes and the actual Runtime load failure. Full CI is reserved for the final R3 candidate.

Using Part 1's production convention (including untracked sources and excluding trailing inline
tests), R1 is **27,510 production lines: +484 against Part 1, +679 against original `7f71beef`**.
Before this checkpoint record, R1 changes 30 files: production physical +651/-163, tests/fixtures
+472/-61, implementation docs +69/-20, manifests/lockfile +7/-1. R0 is separate: two documentation
files, +445/-286 physical lines. The diagnostic facts account for the increase; completed provider
and core deletions are not counted again.

The architect replaces the provisional production/test estimates with this cumulative forecast:

| Category/slice | Revised net addition against Part 1 |
| --- | ---: |
| R1 production, actual | +484 |
| R2 production | +220–350 |
| R3 production | +50–90 |
| Complete production planning range | +750–1,050 |
| Complete tests/fixtures | +800–1,200 |
| Implementation documentation, excluding R0 | +100–300 |
| Manifests/lockfile | +10–30 |
| Distinct changed files | approximately 40–55 |

This estimates 27,776–28,076 production lines, +945–1,245 against the original. These are reviewed
estimates, not allowances to consume. Additional producers, recipes, types or forecast growth
outside the revised range still require cumulative review. Remaining material uncertainties are
R2/R3's actual cost, new-owner admission/cold reporting and v3 deployment handling; validate them
through the finite acceptance cases below before final sign-off.

### R2 cutover — 2026-09-14

R1 is committed as `fa042add`. R2 removes both authority discard helpers and AuthorityError's
serializer, preserves the selected SQL/local facts at their producers, and forwards the two
routes unchanged through Live. The transaction-error schema moves once to v3, with mandatory
authority/signer payloads and unchanged typed classification. Existing signer Invalid/Failed
receiving data is honest; executing SignFailed forwarding remains R3. Current assembly tests
reject the old v2 contract before admission/provider entry. No deployment is performed; existing
v2-run handling remains a rollout obligation rather than an automatic rewrite or dual decoder.

All EVM targets pass, including trusted-text/float/size admission for the two new payload branches.
The targeted Live v3 rejection and custody acknowledgement-recovery cases pass. Selected
PostgreSQL/EVM/Live/App all-target Clippy passes. Managed PostgreSQL passed
`run-3376539-1789413131492202944`, retaining distinguishable load/reserve/retain stages and both
ambiguous authority COMMIT outcomes. The managed Effect test passed
`run-3374133-1789412900364383372`: real SQL 42501 reaches the operational original, cold assembly
and App view; retained epoch mismatch reaches internal invocation with unchanged head and prior
observation. The final explicit App payload assertion also passed in managed Effect run
`run-3378618-1789413163856077069` (171.71 seconds).
The test reuses the existing wallet/provider/signer fixture, with test-only SQLx/App dependencies;
no second provider/signer harness or production hook is added.

R2 adds 334 production lines. Current total is 27,844: +818 against Part 1 and +1,013 against
original `7f71beef`. Remaining R3 production forecast is +50–90, within the reviewed cumulative
+750–1,050 range. Tests and documentation remain separately measured at final acceptance.

### R3 cutover — 2026-09-14

R2 is committed as `f97b5d3d`. Executing sign now returns SigningError directly through the existing
owner reply, removing the KeystoreError roundtrip and blanket unavailable-detail serializer.
SignFailed adds only the selected operation/stage/local fact or dependency message. Live moves its
evidence unchanged into the already-current v3 SignerUnavailable owner; no further schema cutover,
concurrency change, source walker or new public named error type is needed. Import/startup/shutdown
and general checked cryptographic constructors retain their excluded contracts.

Focused signing/keystore all-target tests, Live signer tests and selected all-target Clippy pass.
Real request closure after
owner shutdown and accepted-request/reply closure retain distinct stages; private missing-slot and
pinned primitive-error tests assert exactly available nonsecret facts. Existing valid signing,
low-S/recovery, secret non-renderability and non-Send/non-Sync custody coverage is retained.
Managed Effect run `run-3385277-1789413601544019249` passed in 212.25 seconds: after the existing wallet
scenario shuts down its actual owner, a fresh run admits the real closed-signer failure, preserves
Retryable classification and exact sign/request_send/channel_closed evidence, and restores the
same App view with a fresh assembly. Provider pending nonce remains unchanged at four.

R3 adds 19 production lines, bringing the total to **27,863: +837 against Part 1 and +1,032 against
original `7f71beef`**. The smaller increase comes from deleting the executing-sign roundtrip and
serializer while reusing the existing result channel and v3 payload. Complete measured costs and
final B1–B8 acceptance follow the remaining thin-client coverage and aggregate review.

### B7 client evidence and aggregate candidate — 2026-09-14

R3 is committed as `b6e72d0d`. The architect identified the remaining thin-client coverage gap and
selected one additional case in the existing managed client test. Its single loopback fixture now
supports either the retained interrupted-request case or one supplied JSON-RPC error response;
no new production path, dependency or separate fixture family is added.

Managed client run `run-3393312-1789414260919479735` passed in 112.66 seconds. The new case checks the
actual admitted Read original's Unavailable kind, chain_id/envelope/rpc_error provenance, HTTP 200,
RPC -32073, supplied message and exact data_json. A fresh REST process returns the entire same
failed view/head; fresh CLI output is identical with exit 1 and empty stderr. Focused test-target
Clippy passes. The initial test expectation incorrectly treated the tagged rpc_error field as a
string; the corrected assertion uses the owning wire contract and the complete scenario passes.

Aggregate implementation reconciliation uses Part 1's nonblank/non-comment Rust convention,
excluding separate and trailing inline tests. Physical additions/deletions remain separate:

| Category | Counted net LOC | Physical added/deleted |
| --- | ---: | ---: |
| Production Rust | +837 | +1,132 / -291 |
| Tests/fixtures | +914 | +1,067 / -131 |
| Implementation documentation, excluding R0 | +246 | +365 / -75 |
| Manifests/lockfile | +16 | +18 / -2 |

There are 49 distinct changed files. Production is 27,863: +837 against Part 1's 27,026 and +1,032
against original `7f71beef`'s 26,831. R0's two documentation files cost +445/-286 physical lines
(counted net +119); cumulative documentation including R0 is +798/-349 physical lines.
These counts include the current acceptance record and remain within the revised R1 forecast.

Necessary type changes are three existing Store payloads, two authority payload routes, one new
SigningError variant, and two payloads on the existing transaction-error owner. Named public error
types added: zero. Persisted schema cutovers: one, transaction-error v3. Private extraction recipes
added: one, SQLx. Deleted: two authority input-discarding helpers, AuthorityError's serializer,
executing signing's KeystoreError roundtrip and SigningError's unavailable-detail serializer.
No extra Runtime API, source registry, recovery layer, diagnostic quota or compatibility reader
remains. Production growth buys the selected causal facts at their actual producers; earlier
Part 1 deletions receive no second credit.

| Acceptance | Current authoritative evidence |
| --- | --- |
| B1 | R0 design commits `49605208` through `70200449`, accepted baseline and R1 checkpoint above. |
| B2–B3 | [PostgreSQL tests](crates/storages/postgres/src/tests.rs): native nested/inline/cyclic/IO/column sources, real DB fields and identity message, primary/rollback mapper, actual Runtime load and COMMIT dispositions; managed R1/R2 runs above. |
| B4 | [Runtime contract](crates/kernel/runtime/tests/runtime_contract.rs): declared original plus failed Store, success without invented original, previous observation/candidate and immediate return. Existing producing projection regression remains intact. |
| B5 | [Effect e2e](crates/live/evm/tests/evm_contract_effect_e2e.rs): real authority SQL error admitted and restored, malformed retained epoch internal without append, previous observation unchanged. Managed authority COMMIT and retained-wire recovery cases also pass. |
| B6 | [Keystore unit tests](crates/keystore/src/lib.rs), [public contract](crates/keystore/tests/api_contract.rs), and signing contract tests retain request/reply/missing-slot/primitive facts and crypto/custody guarantees. |
| B7 | Effect e2e asserts actual authority/signing originals and exact cold App data/classification; [owner tests](crates/domains/evm/tests/provider_failure_contract.rs) cover trusted text, floats, size and v2 rejection. CLI/REST recording tests and the managed client case above cover shared transport presentation. |
| B8 | Aggregate reconciliation above, dedicated architect acceptance and all nine final CI stages passed on the exact source candidate, recorded below. |

### Final acceptance — 2026-09-14

The dedicated architect returned **ACCEPTED** for exact candidate
`b704a05e82e4e1a9e2180556b455c6a80a28f997`, with no concrete correction remaining. The review
independently reproduced the aggregate production/test/manifest counts and pre-completion
implementation documentation (+225 counted lines), inspected all selected producer/receiver
contracts, and accepted the actual deletion scope, information limits and boundary tests.
The table above includes this subsequent documentation-only completion record.

Final `nix run .#ci -- --slot 3` passed on that unchanged, clean source candidate:
**9 passed, 0 failed in 752.82 seconds**. Evidence is
`run-3405782-1789415140838762760` under the local Nixfied `mfm/dev/3` state root, including
`artifacts/run-summary.json` and task logs. Format, SQLx metadata, Clippy, workspace compilation,
workspace tests, rustdoc, managed PostgreSQL, client e2e and Effect e2e all passed. The managed
client/Effect stages include the final B5–B7 regressions, not only earlier successful scenarios.

The first final-CI attempt, `run-3398062-1789414567999792718` in slot 1, passed the six Rust/metadata
stages but failed before database tests began because port 23180 refused connections. PostgreSQL
had received a fast-shutdown request during the unit suite, without a matching registry stop event;
the sender was not established. The retry used unused declared slot 3 endpoints 23380/23382.
No source, task-graph or framework change was made, and no other project's service was stopped.

R1–R3 and B1–B8 are complete within this RFC's finite scope. The completion edit changes only
owning documentation; local link/command review and `git diff --check` pass. No additional Rust gate
is required for that record. No deployment, existing-run rewrite or excluded producer enrichment
is claimed. Section 8 retains the explicit pre-rollout inventory obligation.

### Review corrections — 2026-09-15

Follow-up review found that the byte decoders retained length facts without identifying the
failing field. Their existing private helpers now accept a static field name: admitted_epoch,
reservation_epoch, genesis_hash, transaction_hash or sender at the executing call sites. The
bootstrap caller supplies authority_epoch while keeping its excluded GateError conversion.
Focused malformed-byte assertions distinguish all five executing fields and retain Internal
classification, expected/observed length and the supplied conversion message.

The nonce-domain lock key now extracts eight bytes directly from the fixed 32-byte digest and
returns i64. Its impossible conversion-error branch and caller's question mark are removed;
section 5 no longer requires that diagnostic. An independently calculated frozen SHA-256 fixture
checks the unchanged lock key. MemoryStore's successful append path now constructs its byte-total
error lazily with ok_or_else (commit `8616d1e6`). No new abstraction or public API is introduced.

Focused Store/PostgreSQL library tests, the two new authority tests and scoped all-target Clippy
pass under the default Nix shell. Managed PostgreSQL run `run-3542510-1789464048033613475` passed
in 13.65 seconds, including the authority/COMMIT contract. Formatting and `git diff --check` pass.
These local corrections do not change the wire schema, SQL, persistence or concurrency semantics;
verification is scoped to the affected owners. No new full CI run is claimed; the earlier complete
CI evidence remains tied to its named source candidate.

Against completion `8f65db70`, production decreases by nine lines and tests add 59 counted lines.
Production is now 27,854: +828 against Part 1 and +1,023 against the original baseline. The earlier
aggregate table records the September 14 completion snapshot. The net reduction removes the
impossible error construction while adding the required static field provenance.

### Finite acceptance cases

| ID | Producing behavior and required observation |
| --- | --- |
| B1 | R0 records accepted baseline, exact type/producer/receiver design, information limits, reviewed estimates and architect verdict. Completed O1/provider/core work is absent from the migration sequence. |
| B2 | SQLx evidence retains ordered nested layers, inline-child/interface-pointer regression and repetition marker, selected database/SQLSTATE fields and IO kind/code. Use real PostgreSQL errors for DB facts; wrap small nested/inline/cyclic fixtures in SQLx::Decode and use SQLx::Io(io::Error::new(...)) for the exposed-inner-layer case. Selected identity rejection must report the actual constraint message through its caller, not "withheld". No mock SQLx framework or IDs rewrite. |
| B3 | Actual load failure is invocation-only. Precommit class-23 rejection, definite COMMIT rejection and both ambiguous acknowledgement outcomes preserve disposition and facts. Reuse managed PostgreSQL/fault cases; identify injected facts without fabricating native sources. Test the production classify_precommit_sql mapper with distinguishable native primary/rollback fixtures and unchanged primary disposition. No connection-kill harness, rollback fault framework or additional transaction layer is required. |
| B4 | Declared failure plus Store failure retains admitted original, Store cause and submitted candidate. Success plus Store failure invents no original. Preserve previous observation/acknowledgement, immediate Store return and existing NotInserted/projection behavior. Extend Part 1 producer assertions with enriched payloads. |
| B5 | Authority load/reserve/retain errors have distinguishable operation/stage. One actual SQL error reaches the operational original; one malformed retained fact reaches internal invocation without append. Ambiguous authority COMMIT stays OutcomeUnknown and existing reservation/prepared-wire recovery tests pass. |
| B6 | Real closed sign request/reply and private missing-slot cases remain distinguishable without inferring panic; check the primitive-boundary message. Retain valid signing, low-S/recovery, secret non-renderability and non-Send/non-Sync custody tests. No crypto fault harness or concurrency redesign. |
| B7 | Both authority/signing evidence pass through actual Live adapter, Runtime admission, fresh assembly restoration and App observation unchanged, with OutcomeUnknown/Retryable classifiers. Compare required facts, not serialization success alone. Preserve whole-owner float/size/trusted-text and first-encoding behavior; add new-owner cases only where coverage is missing. One Store recording result and one persisted operational result reach both thin clients through shared presentation; no producer-by-transport cross-product. Check v3 ABI and old-contract rejection. |
| B8 | R1-R3 remove superseded paths and reconcile production/test/doc/file/API/schema costs against both baselines. Final CI passes; architect accepts actual aggregate design. Record exact commit/evidence here without claiming platform-wide closure or certification of dependency text. |

## 8. Material uncertainties

No implementation design uncertainty remains within the selected scope. R1–R3 preserve the fixed
producer/receiver contracts, SQLx exposure is exercised by native and managed cases, and both new
owner branches pass Object admission and cold App observation. Final aggregate acceptance and CI
are complete, as recorded above; they do not expand the selected producer set.

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| No deployed v2 run requires continuation under a replacement assembly | Deployment inventory is outside this source implementation | A v3-only assembly cannot execute old contracts | Old-contract rejection is tested. Inventory runs and resolve their handling before rollout; do not rewrite history or add a dual decoder. No rollout is performed here. |

The internal/operational split, trusted dependency text, repeated-interface-pointer information
limit, no-budget diagnostics, classifier ownership and Store/authority acknowledgement semantics
remain explicit accepted limits. Completion of this finite RFC does not certify dependency text
or close excluded platform source-owner gaps.
