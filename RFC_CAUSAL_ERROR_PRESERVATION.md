# RFC part 2: preserve errors on selected execution paths

Status: design draft pending accepted Part 1, reviewed 2026-09-13. **Not ready for a full owner
implementation goal.** [Part 1](RFC_AUDITABILITY_ERROR_CHAIN.md) independently delivers the current
continuation/persistence core and concrete invocation-diagnostic boundary. This RFC closes remaining causal
losses on selected State/adapter execution paths using that implementation. It is not a platform-wide
error-system rewrite. Part 1 can finish while this draft still has open design details.

Part 2 starts from the Part 1 implementation accepted at F1, currently **pending**, with this RFC's
current revision. Continue on that branch; Part 2 does not repeat or reopen Part 1's completed
cutover. Section 2 defines the required refinement before owner implementation. Earlier diagnoses
and measurements are optional [background](docs/auditability-implementation-history.md).

## 1. Objective and explicit limits

Preserve the original failure and required cause information from selected State preparation,
execution/interpretation and adapter operations through their actual Runtime/App consumers.
Include failures to load or record that execution state. Capture at the producing boundary before
information is erased; downstream layers carry it instead of reconstructing it.

Use Part 1's two adaptation boundaries. Declared failures remain the State/capability's concrete
owner errors with their actual causal payload and intrinsic classifier. Commit the original and
restore that same declared type for classification. Internal failures cross heterogeneous interfaces
as immutable InvocationDiagnostic data; Runtime/App forward it and thin clients render it. They
are not persisted or classified. Keep concrete library errors through compatible typed interfaces;
an adapter is needed only where the receiving contract changes. Sharing diagnostics does not turn
a recoverable outcome into a top-level invocation error, or promise that a failed
Store can record its own failure.

An original execution failure and a failure to record it are two related facts with independent
cause chains. Preserve both and the actual acknowledgement; do not make either the other's
fabricated source. After complete admission, reuse the existing Failure/Object with no extra
native original. If initial encoding of a declared error fails, use Part 1's ordinary internal
invocation diagnostic for the actual encoding cause/size, known operation/contract/head and
explicit unavailable original detail/identity. No dedicated recording variant or renderer branch
is needed for this case. Detail-construction failure follows Part 1's fixed omission contract.
It requires no opaque owner, serializer retry or producer fallback payload. Successful values
acquire no custody stash. Part 2 must not reintroduce NativeCause, `Box<dyn Error>` or a custom error protocol.

The diagnostic trust boundary is owned by
[Part 1 section 4](RFC_AUDITABILITY_ERROR_CHAIN.md#4-error-preservation-and-the-diagnostic-trust-boundary).
MFM trusts dependency-supplied diagnostic content and does not scan or certify it as secret-free.
MFM does not deliberately attach its own secret inputs, requests, connection or keystore-command
objects. Bounds and source fidelity remain required. No generic sanitizer or native getter/source
certification project belongs to this RFC.

Configuration administration, deployment, startup/provisioning, RunId entropy, transport listener
lifecycle, generic request ingress and comprehensive constructor/native-decoder audits are outside
both deliveries. Preserve their existing ordinary error handling. A real dependency on one of those
areas does not authorize migrating the area; only a specific producer change needed by a promised
execution case can be added through an explicit architecture decision.

Only the listed execution paths and their required facts define completion. A newly discovered
loss does not create another mandatory producer migration. Reuse the accepted core's diagnostics
and rendering; no owner-specific capture or reporting-failure framework is authorized.

[Design](docs/design.md), [architecture](docs/architecture.md), [code quality](docs/code-quality.md),
[AGENTS.md](AGENTS.md) and [build and verification](docs/build-and-verification.md) apply. Update
implemented contracts with their owning cutover, including superseded diagnostic trust wording.
No cryptographic algorithm, keystore concurrency, retry/reconnect, command-authority or transport
framework redesign is implied. Simplification is measured against accepted Part 1 and cumulatively
against the original comparison baseline in Part 1 section 3. Count each delivery's actual
changes separately; Part 1 deletions cannot justify unlimited additions here.

## 2. Refine this RFC from completed Part 1

Use Part 1's F1 completion record, then make one design commit finalizing this RFC before owner
implementation. Obtain the dedicated architect's decision on the actual inherited mechanism and
complete finite plan; do not discover each row's design by recursively following callers in code.

| Required refinement | Result needed before implementation |
| --- | --- |
| Pin the baseline and inherited APIs | Record accepted Part 1 commit and actual InvocationDiagnostic/SizeViolation, DiagnosticEvidence, Failure/Object, concrete owner classification, Store and rendering signatures/source locations. Link owning definitions; do not copy a second shared API. |
| Subtract completed work | Map E1-E6/C1-C18 to already satisfied producer/consumer cases. Part 1 E5 is not repeated as a constructor migration. Reuse Part 1 admission/reporting work; no blanket JsonError/CanonicalError certification is required. |
| Freeze each error path | Name producing failure/function, selected concrete owner error, required cause layers/fields, any boundary adaptation, receiving conversions, terminal observation, finite assertions, omissions, reuse and deletions. File lists or “all consumers” alone do not pass. |
| Finalize the owner design | Specify actual changed variants/signatures and selected fields. Keep a typed durable error where classification/persistence needs it; use InvocationDiagnostic only at the internal invocation boundary. Existing compatible interfaces need no new wrapper. No new source/schema/capture mechanism follows from a caller connection. |
| Prove admitted-original completeness | Compare required causal/operation facts in the concrete owner, admitted original, restored classifier input and report. Successful serialization alone is insufficient. Keep the agreed unavailable-detail exception for first encoding failure; no generic completeness checker, extra native copy or universal fallback interface. |
| Decide any diagnostic-text requirement | Trust does not automatically add messages to the closed schema. If a case needs supplied text, specify one bounded shared field and its Values admission treatment, including secret-marker behavior. Otherwise retain existing captured fields; no per-provider scrubber, parallel report or speculative schema expansion. |
| Reconcile cumulative cost | Set coherent dependency/commit order, expected production/test/doc/file/API costs and actual deletions per case and in total. No hypothetical deletion credit or arbitrary LOC quota. |

Unexpected callers of a changed API still require correct forwarding and compilation. That does
not authorize changing every producer they call. If a promised field is already erased by an MFM
mapper, the case remains unmet until that exact producing contract is resolved. A SQLx error mapped
to a unit Unavailable is an MFM loss, not evidence that SQLSTATE was unavailable from the client.

A new producer contract, source traversal, public API/schema family, exceeded estimate or failed
claimed deletion requires a cumulative architect design/cost decision before that expansion or the
next row. Record unrelated gaps once in the existing audit inventory without making them mandatory
“deferred” delivery rows. A core limitation needs an explicit proposed change, tests and cost; it
does not authorize restarting Part 1. If the plan cannot support the required simpler design, Part 2
stays draft while completed Part 1 remains accepted.

## 3. Owner representations

These existing owner types illustrate the retained responsibilities. Finalize selected changes
against accepted Part 1 during section 2 refinement; they are not new type families or prerequisites
to completing Part 1. Reuse the type-review decisions in Part 1 section 14.

Do not put every EVM, SQL, signer, or Runtime operation into one global stage enum. The source owner
retains its own operation/stage vocabulary:

```rust
struct ProviderFailure {
    method: EvmRpcMethod,
    stage: RpcStage,
    failure: ProviderFailureKind,
    diagnostics: DiagnosticEvidence,
}

enum EvmOperationalKind { Unavailable, Timeout, RateLimited }

struct EvmOperationalError {
    kind: EvmOperationalKind,
    source: Box<ProviderFailure>,
}
```

The operational carrier factors the three legal alternatives into one closed kind and one common
payload: `Unavailable(P) | Timeout(P) | RateLimited(P)` is `Kind × P`. Its sole wire shape is
`{"kind":"timeout","source":{...}}`; boxing changes memory layout only. This avoids repeating the
same diagnostic schema three times. `new(kind, source)`, `kind()` and borrowing
`provider_failure()` are the existing accessors to reuse; no unit or compatibility constructor remains in
the completed owner cutover.

EvmTransactionOperationalError keeps its provider/authority/signer alternatives and their actual
operation/cause facts. R0 must select the existing concrete port errors or minimal durable owner
fields for O3/O6; unspecified payload names do not authorize new wrappers. A typed
`Box<ProviderFailure>` shares no erased-error behavior and adds no wire layer.

The whole concrete owner error is the classifier input. MfmValue supplies persistence/schema
capability, not type erasure; Self::Failure and C::OperationalError remain concrete. Do not add
a failure trait, per-State enum, adapter-to-State wrapper, PersistedError or second Classification
field. The same provider cause can be Retryable for a Read and OutcomeUnknown for transaction
submission; classification belongs to the enclosing owner with its operation context.

Use ordinary nested sources and borrowing. Remove incidental `Copy` requirements when errors gain
owned data. `Error::source()` should preserve a nested typed source when its type supports that
trait, but generic capability contracts remain usable with operational `MfmValue` types that do
not implement `std::error::Error`. Typed payload access remains part of the contract.

Reuse [Part 1 section 5](RFC_AUDITABILITY_ERROR_CHAIN.md#5-shared-causal-data-with-local-ownership)
and [section 9](RFC_AUDITABILITY_ERROR_CHAIN.md#9-boundary-adaptation-and-invocation-diagnostics).
Capture only the selected facts once while the concrete owner is known. A typed source needed for
a declared operational error must be retained in that error before invocation-only adaptation;
do not reconstruct a classifiable or durable error from diagnostic JSON. One concrete port error
may serve both routes through their explicit adapters, without copying a native owner into each.

Internal boundary adapters supply the inherited InvocationDiagnostic and primary SizeViolation
facts directly. Receivers neither downcast native sources nor rediscover fields or size from JSON.
Use Part 1's bounded field construction and terminal omission behavior. Ordinary private Serialize
helpers are allowed when existing serializable owner data does not already supply the selected
field contract; no generic from_error projector, per-owner report
schema, secondary serialization service or global owner-error enum is permitted. Invocation
diagnostics have no MfmValue identity and cannot replace the classifier's concrete error.

The provider error types and DiagnosticEvidence vocabulary are existing shared contracts. Changes
within the retained rows must reuse or delete actual mechanisms, not count those types as new concepts or earn
hypothetical deletion credit. No new nominal wrapper is required where the selected owner already
carries the same cause and operation.

## 4. Finite execution producer matrix

The five groups below are the complete producer scope. R0 must freeze exact cases and source
paths against accepted Part 1 and remove work already completed. The rows are not
permission to enrich every error in the named modules.

| ID | Selected producing boundary and facts | Consumer and stopping boundary |
| --- | --- | --- |
| O1 | Read/read_anchored and existing json_rpc send/body/RPC failure conversions on selected shipping Read calls: method/stage, status/code, exposed source prefix, actual local mismatch and bounds. | Runtime failed Read and cold App observation; internal preflight stays internal. Reuse existing RPC capture. No HTTP client, retry or decoder-family rewrite. |
| O2 | Selected transaction reserve_nonce/prepare_transaction/execute_transaction calls and map_provider_error/map_authority_error/signer.sign conversions: actual operation and supplied provider/authority/signing cause. | Pending Effect failure through Runtime/App, preserving command/EffectId. O3/O6 own their producer changes; this row forwards their data without a second capture tree or new authority model. |
| O3 | signer.sign and the executing keystore sign request/reply, plus only signature/evidence helpers required by a frozen transaction case: actual sign operation, returned cause and existing invalid/failed/closed distinctions. | O2's concrete operational error or InvocationDiagnostic route according to meaning. No key creation/import/startup/shutdown, comprehensive signing constructors, secret-command capture, cryptographic or concurrency redesign. |
| O4 | Run Store load/append and PostgreSQL precommit/COMMIT conversions required by the selected Runtime recording/load cases: disposition, returned SQLx/source facts and actual query/transaction stage. | Existing load invocation report, or append recording report with failed outcome/candidate when available and actual acknowledgement. A load failure invents no outcome/candidate or recording phase. Affected memory run-Store consumers forward existing causes. No config/index query migration, connection/provisioning audit, append planner or reconnect protocol. |
| O6 | Selected execution authority load/reserve_or_compare/retain_prepared calls and their required begin/commit/load_state conversions: operation, source evidence and existing acknowledgement category. | Authority port to O2; reuse O4's selected shared database extraction. No general retained-row/identity constructor audit, nonce/replacement or startup/provisioning redesign. |

Changed run-Store consumers belong to O4. Runtime/App/CLI/REST only forward and render selected
results and actual terminal encoding/write/flush failures through the accepted core route. Shared
extraction is a named dependency of a selected case, not an independent producer backlog. Part 1
E5 remains completed work. Configuration/index, general ingress and other excluded producers do
not become mandatory through these consumer dependencies.

For each row, preserve all already supplied causes through changed receivers. Typed error/checked
constructor changes are allowed only where the frozen execution facts require them. Reaching a
constructor from a caller is insufficient. Hidden dependency attempts are unavailable; our own
lossy conversion is a gap to fix when its evidence is promised. No row may claim completion by
inventing a source layer, discarding a supplied cause, or substituting a classification for it.

Preserve existing schema/canonical bounds. Any required operational schema change uses the shared
captured representation; do not build another mechanism for persistence. Neither the trust
policy nor this matrix requires arbitrary raw client object dumps or every dependency message.

## 5. Ordered work and completion

### R0: accept the finite execution design

Complete section 2 after Part 1 F1. The architect records accepted or rejected readiness against
the actual inherited APIs, frozen paths, required deletions and cumulative cost. No implementation
row starts with unresolved architecture. At this revision: **R0 is pending Part 1 acceptance.**

### R1: preserve selected recording causes

Complete O4's required run Store conversions and shared database extraction once, reusing Part 1
recording/reporting and admitted-original ownership. Supply size facts at the concrete owner;
do not add a new source-discovery or erased-cause adapter to Store consumers. Cover failed execution plus failed append, success
plus failed append, actual disposition and independent cause chains. Do not repeat the core load
redesign or migrate configuration/index/startup errors to make the test pass.

### R2: preserve execution authority and signing causes

Complete O6 using the shared database extraction and O3 using the existing signing/keystore
operation. Keep each producing contract and its receiving conversions together. Stronger tests
cover the changed signing boundary and MFM's deliberate secret-input handling; they do not audit
every arbitrary dependency diagnostic for credentials.

### R3: complete adapter propagation

Complete O1's remaining selected Read losses and O2's transaction propagation. Reuse O3/O6 data;
prove the complete required operational original through cold observation and unchanged command
authority. Preserve internal failure routing without adding fault records or extra capture layers.

### R4: verify the shared consumer route

Verify Runtime/App/CLI/REST forwarding and rendering for the selected paths using the same
InvocationDiagnostic or admitted original data, with no consumer recapture or JSON-to-text reparse.
Required caller migrations belong in their producing API cutovers, not this final
step. Reuse Part 1 cases/statuses and add only missing observable boundary coverage. No cross-product
of every producer and transport, general ingress audit or new successful-result reporting service.

### F2: accept the complete Part 2 implementation

Each coherent cutover deletes superseded code/API/tests/docs and retains required behavior coverage.
Use section 2's refined dependency order if it differs from R1-R4; keep inseparable changes together.
Reconcile B1-B8 and every frozen case against the complete candidate. Use scope-driven focused
checks, managed PostgreSQL/SQLx and client scenarios when affected, then one final `nix run .#ci`
for Part 2. Part 1 CI does not substitute for Part 2 integration. Direct Rust tooling runs in the
default Nix shell; subsequent fixes require affected checks and review of changed contracts.

Report production, tests/docs, total churn, public API/schema changes and actual responsibility
removals against accepted Part 1 and the original comparison baseline identified in Part 1
section 3. Count a deletion only against a baseline where the code exists, and never count Part 1
deletions twice. Review the aggregate design; unresolved simplification objections keep Part 2
unaccepted and require a design decision before further owner migrations.

## 6. Acceptance evidence

B1-B8 define Part 2 acceptance. Part 1 C1-C18 remain retained behavior; its completed migration
is not performed again.

| ID | Observable acceptance |
| --- | --- |
| B1 | R0 pins accepted Part 1, actual inherited APIs and every finite producing failure, required fact, changed conversion, consumer assertion, omission, reuse/deletion and cumulative cost. No broad owner or deferred nonexecution row remains mandatory. |
| B2 | Each selected concrete owner error retains its promised cause/operation fields. Declared State/adapter failures restore the same declared type for classification; neither the deepest source alone nor invocation JSON/classification codes substitute for that owner. Internal load/recording errors remain invocation-only. Existing acknowledgement and operational/internal routing hold. New producer contracts are explicit; completed work is closed with evidence. |
| B3 | O1/O2 operational errors, including required O3/O6 causes, survive complete admission and cold observation. Internal execution/recording failures use invocation reports without a fault append. Command/EffectId authority is unchanged. |
| B4 | Upstream diagnostic content follows the trust contract without credential scanning or blanket source/downcast certification. MFM does not deliberately attach its own secret inputs. Bounds/omissions and any explicitly required shared text/admission change are exercised at the selected boundary. |
| B5 | Capture occurs once at the required boundary. Receivers forward immutable InvocationDiagnostic/admitted data and supplied size facts without downcasting, JSON field discovery or recapture. Required original facts are compared across admission/restoration/reporting. No NativeCause, projector/custody tree, native-success stash, new size hierarchy, per-owner reporting schema or generic completeness checker remains. |
| B6 | An admitted execution failure plus recording failure retains two independent causes and actual candidate/acknowledgement. Failed initial error encoding uses Part 1's ordinary internal diagnostic with known context, encoding cause and explicit unavailable-original detail under its fixed omission contract; no special Runtime variant, opaque custody or fallback payload. Success plus recording failure invents no original error. Reports preserve actual delivery stage; no second audit sink, append retry or recursive reporting. |
| B7 | Cutovers delete superseded paths/APIs/tests/docs while preserving meaningful behavior coverage. Actual aggregate complexity and production/test/doc/file/API costs are reviewed against both baselines; no compression, hypothetical deletion credit or duplicate native representation hides growth. |
| B8 | All frozen cases and relevant retained Part 1 behavior pass required checks and final CI. Completion names the accepted commit, evidence and limits; it does not claim platform-wide provenance or secret-free certification of upstream diagnostics. |

## 7. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| The accepted core supplies sufficient boundary diagnostics | K1-K3 and final APIs are not complete | Owner rows could recreate projectors, size discovery or duplicate originals | R0 inspects accepted signatures and E1-E6 evidence, including bounded construction and terminal omission; resolve one concrete missing contract before coding. |
| Concrete execution error payloads retain required causal and classification facts | Correct associated types can still contain lossy variants or serializers | Cold classification/reporting could lose a promised fact | Compare selected concrete errors, admitted payloads, restored classifier inputs and consuming assertions. The first-encoding unavailable-detail exception remains settled; do not add generic custody. |
| Existing shared fields suffice under the upstream trust contract | DiagnosticEvidence is closed and ordinary Values strings are marker-scanned | A required text field could be rejected or cause per-provider workarounds | Freeze the needed facts and one shared bounded field/admission change if needed; otherwise preserve existing schema. |
| The selected paths and cost are finite and simpler overall | Exact conversions and accepted Part 1 baseline remain unproved | Hidden dependencies or locally justified growth could recur | R0 freezes semantics, conversions and cost; exceeded estimates trigger review and F2 judges the actual aggregate result. |

The trust/scope, concrete-owner classification and failed-initial-encoding information contracts
are settled; bounded implementation, exact remaining producer contracts and LOC improvement are
still proofs to deliver.
