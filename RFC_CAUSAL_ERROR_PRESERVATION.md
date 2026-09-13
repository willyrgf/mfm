# RFC part 2: causal-error preservation across remaining owners

Status: design draft pending accepted Part 1, reviewed 2026-09-13. **Not ready for a full owner
implementation goal.** [Part 1](RFC_AUDITABILITY_ERROR_CHAIN.md) delivers and independently accepts
the continuation/persistence core, shared causal carrier and bounded consuming cases. This RFC then
closes the named remaining producer losses using that implementation. Part 1 may finish while this
draft still has open design details; this RFC does not reopen Part 1's acceptance retroactively.

Part 2's implementation baseline will be the accepted Part 1 commit, currently **pending**.
`7f71beef` remains historical evidence/comparison, not a restart point. Preserve
`/tmp/mfm-audit-core-first`, its commits after `aae81795` and all dirty/untracked work. Review a
specific later change or test only when its owner row is reached; it must fit the accepted core
and current row. Do not merge/cherry-pick the accumulated owner series as a substitute for that
review, or rebuild the persistence design from scratch.

## 1. Objective, dependency and scope

Preserve the complete available, reviewed causal chain at the producing and consuming boundaries
in section 4. Preserve classification and acknowledgement semantics while deleting lossy
conversions and redundant capture/serialization paths. Simplification remains an objective of this
part, measured against its own accepted baseline; Part 1's deletions cannot excuse unlimited growth.
This is a broad owner migration, not an assumed small cleanup.

The third attempt grew because the combined RFC made every upstream loss part of a single delivery
and combined extraction, custody, schemas, callers and tests per owner. Reordering that requirement
was insufficient. [Part 1 section 3](RFC_AUDITABILITY_ERROR_CHAIN.md#3-why-the-third-attempt-expanded)
retains the measured diagnosis. This split changes the acceptance dependency: deliver the core,
learn from its actual APIs and cost, refine this RFC, then execute its finite owner steps.

Part 2 does not repeat the fold/state/persistence replacement, introduce another native carrier,
or perform historical transition validation. Internal failures remain invocation reports; declared
State-domain and adapter operational failures retain their originals durably before recovery.
Accepted settlement still precedes interpretation; explicit resume uses acknowledged continuation.

[Design](docs/design.md), [architecture](docs/architecture.md), [code quality](docs/code-quality.md),
[AGENTS.md](AGENTS.md) and [build and verification](docs/build-and-verification.md) apply. Update
affected authoritative contracts with implementation. No dependency upgrade, cryptographic,
keystore concurrency, retry/reconnect, command-authority or transport framework redesign is implied.

## 2. Refine this RFC from completed Part 1

Part 1's F1 completion record supplies the accepted commit, finalized API/source locations,
observable cases, removals, measurements and known gaps. After it exists, make one design commit
updating this RFC with the following, then obtain the dedicated architect's readiness decision:

| Required refinement | Concrete result needed before owner implementation |
| --- | --- |
| Pin the real baseline and reusable contracts | Record accepted Part 1 commit and actual NativeCause capture/access, DiagnosticEvidence, operational carrier, Store and reporting signatures/source locations. Link the owning definitions; do not copy a second set of shared type declarations. |
| Reconcile delivered evidence | Map Part 1 E1-E6/C1-C18 to rows already satisfied or partly satisfied. Mark O11 delivered in Part 1 E5; remove completed JsonError/CanonicalError work from O12. Verify status from the accepted code, not the abandoned branch. |
| Freeze the remaining source-to-consumer work | For every row, record exact repository paths and symbols, producing failure and endpoint, fields/layers retained, omissions/upstream limits, classification/routing, reuse, required deletion and stopping boundary. Broad crate names alone do not pass. |
| Resolve each row's design | Specify final owner types/variants, changed signatures/conversions, persisted schema changes when needed and native custody at those sites. State how captured child data is reused. List finite observable tests without a cross-product of all owners/transports. |
| Reconcile dependency and cumulative cost | Set coherent commit order and expected production, tests/docs, changed-file and API/schema costs for each row and the total. Identify duplicated machinery actually removed; no hypothetical deletion credit or arbitrary LOC ceiling. |
| Close material architecture gaps | Decide the safe available facts at each client boundary and any remaining constructor/custody questions. Record explicit unavailable/withheld/deferred facts and the corresponding limits on the completion claim. |

The matrix below is the existing finite draft scope, not a claim these details are already frozen.
Do not start each implementation row and then discover its design through recursive caller tracing.
A row may be subdivided into named finite commits; this must not add upstream families implicitly.
A completed or unnecessary row can be closed with evidence, without implementing it again.

A core limitation revealed here requires a specific proposed contract change, affected tests,
removals and cost. Review it explicitly before implementation; acceptance of Part 1 is not a claim
that future changes are impossible, and a Part 2 gap is not authorization to restart the core.
If this refinement cannot support a simpler consistent design and finite completion claim, Part 2
stays draft. That does not prevent release/acceptance of the completed Part 1.

## 3. Owner representations

These proposed owner shapes came from the combined RFC. Finalize them against accepted Part 1
during section 2 refinement; they are not prerequisites to completing Part 1.

Do not put every EVM, SQL, signer, or Runtime operation into one global stage enum. The source owner
retains its own operation/stage vocabulary:

```rust
struct ProviderFailure {
    method: EvmRpcMethod,
    stage: RpcStage,
    diagnostics: DiagnosticEvidence,
}

enum EvmOperationalKind { Unavailable, Timeout, RateLimited }

struct EvmOperationalError {
    kind: EvmOperationalKind,
    source: Box<ProviderFailure>,
}

enum EvmTransactionOperationalError {
    Provider {
        operation: TransactionProviderOperation,
        cause: EvmOperationalError,
    },
    Authority {
        operation: AuthorityOperation,
        cause: ReviewedAuthorityFailure,
    },
    Signer { cause: ReviewedSigningFailure },
}
```

The operational carrier factors the three legal alternatives into one closed kind and one common
payload: `Unavailable(P) | Timeout(P) | RateLimited(P)` is `Kind × P`. Its sole wire shape is
`{"kind":"timeout","source":{...}}`; boxing changes memory layout only. This avoids repeating the
same diagnostic schema three times. `new(kind, source)`, `kind()` and borrowing
`provider_failure()` are the proposed accessors; no unit or compatibility constructor remains in
the completed owner cutover.
Transaction provider, authority and signer alternatives retain their distinct typed payloads.

These sketches preserve the current semantic categories while adding evidence. Do not introduce
a second `Classification` field that can disagree with the intrinsic classifier.

Use ordinary nested sources and borrowing. Remove incidental `Copy` requirements when errors gain
owned data. `Error::source()` should preserve a nested typed source when its type supports that
trait, but generic capability contracts remain usable with operational `MfmValue` types that do
not implement `std::error::Error`. Typed payload access remains part of the contract.

The shared custody/data/capture contract has one owner in
[Part 1 section 4](RFC_AUDITABILITY_ERROR_CHAIN.md#4-preservation-and-disclosure-are-separate-responsibilities),
[section 5](RFC_AUDITABILITY_ERROR_CHAIN.md#5-shared-causal-data-with-local-ownership) and
[section 9](RFC_AUDITABILITY_ERROR_CHAIN.md#9-capture-once-report-existing-data).
Raw/native access, public output and persistence obey that same custody rule. Redacted Serialize
alone is insufficient. Capture reviewed facts once at the first owner; outer errors retain that
data. There is no fallible child projector, independent reporting tree, client downcaster registry,
internal-error MfmValue identity or decoder-error schema family to implement for each row.

## 4. Finite owner-to-consumer matrix

Source sites were identified against `7f71beef` and the third attempt. Section 2 must update them to
the accepted Part 1 commit and subtract completed work before implementation. Existing row IDs
remain for traceability; O11 is fully assigned to Part 1 E5 and is not a Part 2 prerequisite task.

| ID | Producing boundary and required facts | Consuming endpoint and stopping boundary |
| --- | --- | --- |
| O1 | Existing json_rpc.rs send/body/RPC capture and lib.rs read/read_anchored bridges: retain existing method/stage, status/code, source prefix, bounds and local route mismatch facts. | One actual Read failure through Runtime/App cold observation plus internal preflight rejection. Reuse the baseline capture; no HTTP client, retry or RPC decoder rewrite. |
| O2 | transaction.rs reserve_nonce/prepare_transaction/execute_transaction, map_provider_error/map_authority_error and signer.sign conversion: preserve provider, authority or signing source with that transaction operation once. | Pending Effect failure through cold App inspection and unchanged command/EffectId. Authority/signing extraction belongs to O3/O4/O6; this row forwards their reviewed data, not a second capture tree. |
| O3 | signing public-key/signature constructors and recover_public_key; keystore start/import_secp256k1/sign/shutdown and their owner/channel conversions: operation, invalid-vs-failed/closed-vs-owner-failed, exposed safe crypto/source facts. | Native signing error and the transaction operational result from O2. No secret command, key material or panic payload; no signing/keystore concurrency, authority, or cryptographic algorithm redesign. |
| O4 | PostgreSQL load_run/append_run/configure_append_transaction and precommit/COMMIT mappers: existing disposition, SQLx category/SQLSTATE, query/transaction stage and exposed safe nested sources. | Runtime recording failure and App report, including original/candidate and ambiguity. Reuse one SQLx extraction owner; no new Store lifecycle, reconnect protocol or append planner. |
| O5 | PostgreSQL config load/list/import/delete, decode_revision/commit_mutation/classify_revision_query; index list_runs/classify_index_query: operation, client cause, existing absence/corruption/indeterminate semantics. | Existing App config/list response or native invocation error. Forward checked reference errors through the owner custody rule; do not expand into all identity/value constructors. |
| O6 | PostgreSQL evm_tx load/reserve_or_compare/retain_prepared, begin_authority/commit_authority/load_state and its source-erasing helpers: authority operation, SQLx/source evidence, retained-row field and existing acknowledgement category. | Authority port to O2's persisted operational failure or internal invocation report. Share O4's SQLx extraction and domain-owned reviewed authority data; no nonce/replacement authority redesign. |
| O7 | PostgreSQL connect/classify_open_error/verify_connection/verify_evm_connection and their baseline marker/durability/role/catalog checks; provision_postgres/provision_evm_transaction_authority: query source or actual failed gate. | Existing open/provision result through App startup. Preserve a gate failure before pool acquisition can hide it. Reuse shared extraction; no general catalogue-diff/constraint-diagnostics framework. |
| O8 | Memory Store/config/index existing task/allocation and local physical-check failure conversions affected by the new public error payload. | Existing native port/Runtime/App error. Preserve available causes and named check facts; no invented provider chain or new memory-backend failure simulator. |
| O9 | Deployment::load/parse/resolve_environment; Application::open/ComposedRuntime::compose, map_config_repository_error/map_run_index_error/map_runtime_error and existing request task joins. | Existing startup or request response in CLI/REST. Retain the returned child and operation; optional source text is withheld at its owner. No global Runtime assembly-rejection or configuration-field taxonomy migration. |
| O10 | generate_run_id; CLI read_config_input/write_stdout/emit_error; REST listener/serve/shutdown and existing json_rejection/config_json_rejection/checked ingress conversions. | Existing reviewed request/output error. Retain entropy/IO/parser or concrete framework rejection category and location, omissions and true delivery stage. Stop at the client/framework's exposed source; no clap/axum private diagnostic reconstruction, comprehensive extractor replacement, or logging service. |
| O12 | Shared IO/client extraction reached by O1/O4/O9/O10, excluding the JsonError/CanonicalError owner correction completed in Part 1: safe category/location/code, available reviewed constructor facts, explicit text/payload withholding. | The returned carrier itself, including source()/downcast/getters, followed by the selected consumer. Reuse Part 1 parser/custody APIs. Correct each remaining listed source owner once; remove caller-specific sanitizers. No blanket new parser/error schema family. |

Persisted schema work is required only for actual declared operational/domain fields in
O1/O2/O3/O6. Native-only detail has no MfmValue identity or independent schema registry. Use the
same captured evidence for the native route and the admitted operational value as applicable;
do not create a second capture subsystem for persistence. Unknown source types retain the exposed
safe opaque layer/deeper links within the bound. Client-private attempts or prohibited text remain
unavailable/withheld; they do not authorize client changes.

Consumer closure forwards the supplied cause through callers of an API changed by that row. It
does not add new variants or facts to every reachable producer. Record a newly found upstream loss
once in the [existing audit inventory](docs/adapter-error-audit.md): producer, consumer, missing fact
and affected guarantee. It does not become a new row automatically. A change in source contract,
new public API/schema/constructor family, exceeded reviewed estimate or failed claimed deletion
requires a cumulative architect scope/cost decision before continuing that expansion or another
row. Update this RFC with the decision; a local cleanup or cost explanation alone is not acceptance.

## 5. Ordered work and completion

### R0: refine and accept the Part 2 design

Complete section 2 after Part 1 F1. The architect reviews the actual inherited APIs, frozen rows,
reuse/deletion plan, acceptance cases and cumulative cost, and records an accepted or rejected
readiness decision here. No owner implementation begins with an unresolved architectural row.
Keep that one current record concise. At this revision: **R0 is pending Part 1 acceptance.**

### R1: complete shared extraction at the selected owners

Finish only the remaining O12 sites and O4 SQLx extraction, with the core-provided carrier and
Store semantics. Reuse Part 1 custody/admission rather than reimplementing parser sanitizers.
Prove a real O4 recording consumer, including the existing ambiguity/category contract.

### R2: close the database owner rows

O5/O6/O7 reuse O4's extraction and existing port classifications. Each is a coherent owner cutover
with its named consumer and superseded path removed. O8 covers only remaining memory-port causes;
no new backend simulator or extra persistence protocol. Do not repeat Part 1 Store load redesign.

### R3: close signing and operational adapter rows

Complete O3's reviewed signing/keystore causes, then O2 forwarding O3/O6 evidence without a second
capture tree. Complete O1's remaining gaps using its existing RPC capture. Prove cold Read and
pending Effect preservation and unchanged command authority. Signing/keystore changes require
the repository's stronger boundary/secret tests; no cryptographic or concurrency redesign.

### R4: close Application and transport owner rows

O9/O10 retain causes supplied by earlier rows at startup/request/IO/ingress boundaries. Reuse Part
1's invocation and public status projection; only the named remaining losses require changes.
Keep true acknowledgement and delivery stage. A final serializer does not justify changing every
upstream constructor or transport extractor.

### F2: accept the complete Part 2 implementation

Each row's commit includes its finite consumer tests, affected documentation and deletion of
superseded implementations. Use section 2's refined dependency order if it differs from R1-R4;
keep inseparable cutovers together and do not retain two current designs to split commits.

Reconcile B1-B8, all frozen rows and known omissions against the exact complete candidate. Apply
scope-driven focused checks during work, managed PostgreSQL/SQLx and client scenarios when those
boundaries change, and one final `nix run .#ci` for Part 2. Part 1's CI is evidence for its accepted
baseline, not a substitute for integration of Part 2's changes. All direct Rust tooling runs in
the default Nix shell. Final verification and architect review must cover the accepted candidate;
subsequent fixes need affected checks and review of changed contracts.

Report actual production change, tests/docs, total churn, public API/schema changes, removals and
necessary additions against accepted Part 1, plus the cumulative result against `7f71beef`.
Separate reusable completed work from new additions; do not count Part 1 deletions twice or credit
removal of abandoned-only machinery as an original-baseline deletion. Review the actual aggregate
design, not merely whether each row has a rationale. An unresolved simplification objection keeps
Part 2 unaccepted rather than authorizing unrelated cleanup or another owner cycle.

## 6. Acceptance evidence

B1-B8 replace the combined RFC's broad C15/final-owner obligation. Part 1 C1-C18 remain retained
behavior; its completed migration is not performed again.

| ID | Observable acceptance |
| --- | --- |
| B1 | R0 names the accepted Part 1 baseline, final inherited APIs and finite rows with exact symbols, cases, facts, omissions, dependencies, required deletions and cumulative cost. No unresolved source/custody design is delegated to implementation. |
| B2 | Each remaining row preserves its named available layers, operation and reviewed fields at its actual consumer. Existing classification, absence/corruption/acknowledgement and internal/operational routing remain correct. Previously satisfied rows are closed with baseline evidence. |
| B3 | O1/O2 and the O3/O6 causes they carry survive admitted operational persistence and cold inspection. Internal Store/config/signing/request failures use native reports and gain no history fault record. Pending Effect identity/authority is unchanged. |
| B4 | Source/downcast/getters/Debug/Display and public/persisted output expose only permitted reviewed data. Withholding, unavailable layers and bounds are explicit. Native safe original preservation and unsafe-original rejection reuse the core contract. |
| B5 | Owner errors reuse captured child detail and shared checked diagnostics. Capture limits/encoding failure do not recursively create projectors or arbitrary secondary error trees. No repeated owner Wire, caller sanitizer or native-only schema registry remains in a migrated row. |
| B6 | App/CLI/REST retain the supplied original and separate recording cause, classification/status, acknowledgement and actual delivery failure. No dropped primary cause, speculative retry/append or hidden framework diagnostic reconstruction is introduced. |
| B7 | Each row deletes its superseded path/API/tests/docs in the coherent cutover while retaining meaningful behavior coverage. Cumulative production/test/doc/file/API cost and actual complexity reduction are reviewed against accepted Part 1 and the original baseline; neither LOC compression nor hypothetical deletions count. |
| B8 | All frozen rows and relevant retained Part 1 behavior pass required checks and final CI on the complete candidate. The completion record names the accepted commit, evidence and withheld/unavailable/deferred facts; it makes no broader provenance claim than the tested source contracts. |

The goal remains complete available causal preservation at the named boundaries. Neither this
matrix nor ordinary Serde guarantees arbitrary downstream Deserialize provenance, inaccessible
client attempts, raw secret retention, or audit through a failed Store. State those limits without
silently presenting partial capture as lossless or extending implementation scope to hide them.

## 7. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| The accepted core supplies enough reusable capture/admission/reporting machinery | Part 1 K1-K3 and final APIs are not complete | Owner rows could recreate projectors or require a core contract change | R0 inspects accepted APIs and actual E1-E6 evidence; decide a concrete missing contract before owner implementation. |
| The draft rows identify all intended remaining work without unnecessary repeats | They were identified against the original/third-attempt sources, not accepted Part 1 | Duplicate work or hidden upstream fanout | R0 pins paths/symbols and subtracts delivered cases; newly discovered losses require an explicit scope/cost decision. |
| Named client APIs expose enough safe facts | Clients may hide source layers/attempts or retain prohibited input | Some details cannot be recovered or preserved safely | Review each named source and native access path in R0; specify safe facts and explicit omissions before implementation. |
| The remaining owner design achieves simpler overall code at acceptable cost | No accepted Part 1 baseline or complete row design/cost exists yet | Another collection of locally justified changes could grow without convergence | R0 reviews the cumulative reuse/deletion plan; exceeded estimates trigger review, and F2 accepts only the actual aggregate result. |

This split is documentation only. It implements neither part, claims no new Rust/CI evidence and
discards no work. Part 1's pending proofs are not answered by the existence of this second RFC.
