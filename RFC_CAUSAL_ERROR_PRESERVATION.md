# RFC part 2: preserve errors on selected execution paths

Status: R0 design accepted by the dedicated architect, 2026-09-14; implementation and B2-B8
acceptance remain pending.
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
to InvocationDiagnostic. Use ordinary serialization of these variants; delete the current blanket
unavailable-detail serializer. Live moves Unavailable evidence into the durable owner and forwards
Internal directly as `AdapterError::Invariant(diagnostic)`, without recapturing it.

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
Every layer retains its actual Display message. Root kind is the snake_case SQLx 0.9 variant name;
a future unrecognized variant uses other and retains message/sources. Select these additional fields:

| Known layer | Fields when supplied |
| --- | --- |
| SQLx | ColumnDecode: index; ColumnIndexOutOfBounds: index/len; ColumnNotFound: column; TypeNotFound: type_name |
| PostgreSQL Database error | sqlstate, severity, message, detail, hint, schema, table, column, constraint |
| std::io::Error | os_kind, os_code |

Null means an optional field was not supplied. Do not add query arguments, SQL statement copies,
connections or rejected rows. Other PostgreSQL fields (data_type, position, internal query/context,
server file/line/routine) are outside this selected contract. No metadata ledger is needed;
do not claim preservation of every native field.

Follow the actual exposed chain, without Debug dumps or parsing Display. Visit SQLx::Database's
concrete database error through its public error interface so a Box wrapper cannot hide it.
At io::Error, retain an exposed inner error if ordinary source traversal skips its wrapper.
Keep one path, not a separately captured tree plus a source chain. Use Part 1's full-interface-
pointer repetition guard and source_cycle meaning. Other source types retain exposed messages
and ancestry; no global downcast registry or generic Values capture API is introduced.

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

### 4.2 Necessary local constructors

Mandatory Store payloads also affect LoadedRun/MemoryStore and PostgreSQL physical validation,
planning and their existing own_candidate_bytes/run_pure_blocking helpers. Do not merge backends
or refactor planning. Local checks retain their reason; size/arithmetic variants stay unchanged.
Allocation failures retain message and requested byte count; unavailable runtime handles retain
their message; joins retain task stage and cancelled/panicked distinction, never panic payload.
The direct ContentDigest::parse and RunSummary::new mappings in these physical helpers retain
the existing returned error and field/check through ordinary serialization; do not change those
constructors or follow their callers upstream. These and the row try_get calls above are the
complete additional producers selected by this result-type cutover.

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
| parse_content_ref, EffectId::parse, Reservation::new | exact concrete error already returned, with helper/field; no upstream constructor changes |
| parse_u64; epoch/hash/address byte helpers | parse kind/message or local syntax reason; field and expected/observed length for byte conversion, no rejected bytes |
| nonce_domain_lock_key; ExactRawTransaction::new | local conversion/length failure, existing expected/observed lengths, no raw signed wire |

SQLx failures remain Unavailable, including ambiguous authority COMMIT. Existing nonce exhaustion
remains Unavailable with its own reason. Other local/retained-data failures remain Internal.
Construct InvocationDiagnostic once, code authority_internal, named operation and stage/check/source
fields. For local helpers without port context, operation names that helper; do not extend a public
constructor just to supply the outer port name. Serializable concrete errors use from_fields directly. Foreign scalar parse/length errors
use the fixed facts above, not a generic serializer family. Forward existing AuthorityError/
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

Measure after each cutover, including untracked files.
An exceeded estimate, extra type/source recipe or failed claimed deletion stops owner
fanout for cumulative design review. Passing tests or small local cleanups do not resolve aggregate
design objections. Do not compress code or weaken coverage to satisfy a line count.

Use [build and verification](docs/build-and-verification.md); direct Rust tools run through
`nix develop -c`. R1 selects Store/PostgreSQL/Runtime/App and affected clients; R2 selects
PostgreSQL authority/EVM/Live and affected ABI consumers; R3 selects signing/keystore/Live.
Use managed postgres-test for real SQLx/COMMIT behavior. SQL text/metadata should not change;
sqlx-prepare is only needed for a separately justified query change. Run one final
`nix run .#ci` on the complete candidate.

### Finite acceptance cases

| ID | Producing behavior and required observation |
| --- | --- |
| B1 | R0 records accepted baseline, exact type/producer/receiver design, information limits, reviewed estimates and architect verdict. Completed O1/provider/core work is absent from the migration sequence. |
| B2 | SQLx evidence retains ordered nested layers, inline-child/interface-pointer regression and repetition marker, selected database/SQLSTATE fields and IO kind/code. Real PostgreSQL errors test DB facts; small local foreign-error fixtures test exposed nested sources. No mock SQLx framework. |
| B3 | Actual load failure is invocation-only. Precommit class-23 rejection, definite COMMIT rejection and ambiguous acknowledgement preserve disposition and facts. Reuse existing fault support; identify injected facts without fabricating native sources. Preserve both primary query and explicit rollback failure when both occur. |
| B4 | Declared failure plus Store failure retains admitted original, Store cause and submitted candidate. Success plus Store failure invents no original. Preserve previous observation/acknowledgement, immediate Store return and existing NotInserted/projection behavior. Extend Part 1 producer assertions with enriched payloads. |
| B5 | Authority load/reserve/retain errors have distinguishable operation/stage. One actual SQL error reaches the operational original; one malformed retained fact reaches internal invocation without append. Ambiguous authority COMMIT stays OutcomeUnknown and existing reservation/prepared-wire recovery tests pass. |
| B6 | Real closed sign request/reply and private missing-slot cases remain distinguishable without inferring panic; check the primitive-boundary message. Retain valid signing, low-S/recovery, secret non-renderability and non-Send/non-Sync custody tests. No crypto fault harness or concurrency redesign. |
| B7 | Both authority/signing evidence pass through actual Live adapter, Runtime admission, fresh assembly restoration and App observation unchanged, with OutcomeUnknown/Retryable classifiers. Compare required facts, not serialization success alone. Preserve whole-owner float/size/trusted-text and first-encoding behavior; add new-owner cases only where coverage is missing. One Store recording result and one persisted operational result reach both thin clients through shared presentation; no producer-by-transport cross-product. Check v3 ABI and old-contract rejection. |
| B8 | R1-R3 remove superseded paths and reconcile production/test/doc/file/API/schema costs against both baselines. Final CI passes; architect accepts actual aggregate design. Record exact commit/evidence here without claiming platform-wide closure or certification of dependency text. |

## 8. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| Selected extraction and payloads stay small in aggregate | Implementation and full consumer cutover are unmeasured | Finite scope could still generate excessive plumbing | Compare with R0 estimates, inspect R1 before R2, reconcile at B8; redesign instead of opening more rows. |
| New owner payloads fit inherited admission/reporting | New persisted fields are not implemented | Causes could fail admission or cold observation | B7 exercises actual owner/Object/report and ABI; preserve limits/first-encoding behavior instead of adding a carrier. |
| SQLx exposure matches the selected recipe | Foreign Box/IO source interfaces can skip layers | Root-message tests could falsely certify fidelity | B2 uses actual variants and nested/inline/cyclic fixtures; retain the accepted pointer guarantee. |
| No deployed v2 run requires continuation under a replacement assembly | Deployed runs have not been inventoried | v3-only assembly cannot promise to execute old contracts | Test rejection and resolve existing-run deployment handling before rollout, without automatic rewrites or dual decoders. |

The internal/operational split, trust policy, no-budget diagnostics, classifier ownership, Store/
authority acknowledgement semantics and current-schema policy are settled. Implementation,
aggregate cost and B2-B8 evidence remain to be proved.
