# RFC part 2: preserve errors on selected execution paths

Status: design draft pending accepted Part 1, reviewed 2026-09-14. **Not ready for a full owner
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

Keep Part 1's checked Object as the only executable/persisted value eraser, confined to its
heterogeneous continuation and private registered callback connections. State/adapter code and
compatible helpers retain concrete values/errors. Reuse checked IDs and borrowed identity getters;
do not introduce raw carriers, qualification/native caches or another value eraser. InvocationDiagnostic
is final report data, with no executable-value, native-owner or classification role.
Its details field uses the same Values-owned DiagnosticEvidence nested in concrete owner errors:
one transparent JSON value with from_value/as_value and the existing PersistedSchema contract.
The diagnostics crate and source/fact/omission framework are deleted in Part 1. Add no diagnostic
quota, metadata accounting, capture factory or standalone diagnostic MfmValue identity. Reuse the
accepted trusted-text profile and ordinary whole-owner/Object admission; actual canonical/Object/
frame/run/terminal-report limits remain with their owners. Invocation field conversion alone uses
the fixed encoding_failed/panicked markers and primary SizeViolation. Those markers never replace
required cause data in a successfully admitted operational original.

Reuse Part 1's owned public RunView and FailureReport, which retain Failure/EffectCall/Settlement
and share Object storage without full-record backing or an owned incident/cause mirror. Required
public wire distinctions use borrowing serializers. Recording derives its admitted original and
return disposition from the proposed/acknowledged record; do not restore independent original or
yield_after arguments. The existing Store selected-row protocol and Journal mfm.run.frame.v6
envelope remain the inherited persistence boundary. Existing Runtime transition/dispatch selects
work from the latest acknowledged record and admission under Part 1 section 6.5, without replaying
recovery or charging it again. Part 2 adds no continuation type or dispatch layer.

Part 1 section 6.2 settles malformed framework data rejected by Runtime record Deserialize:
report parser category, available location and rejection reason through Restore/
Decode, internal/500 and CLI exit 2. This includes oversized nested stored Objects, without
structured SizeViolation. Do not recover native nested fields, parse messages or restore parent
seeds. Direct typed/slot diagnostics and live size/422 reporting remain required. This exception
does not extend to the selected State/domain/adapter originals, E5 or native Store/PostgreSQL
failures; Part 2 cannot use its producer-closure rules to reopen completed storage decoding.

An original execution failure and a failure to record it are two related facts with independent
cause chains. Preserve both and the actual acknowledgement; do not make either the other's
fabricated source. After complete admission, reuse the existing Failure/Object with no extra
native original. If initial encoding of a declared error fails, use Part 1's ordinary internal
invocation diagnostic for the actual encoding cause/size, known operation/contract/head and
explicit unavailable original detail/identity. No dedicated recording variant or renderer branch
is needed for this case. Detail-construction failure follows Part 1's fixed invocation-marker contract.
It requires no opaque owner, serializer retry or producer fallback payload. Successful values
acquire no custody stash. Reuse Part 1 section 9.4's RecordingFailure directly: BeforeAppend holds
an admitted Failure and preparation diagnostic; Store/NotInserted hold an optional original and
exact submitted candidate. Pre-append failure without an admitted original uses the ordinary
internal diagnostic. Retain no unsent candidate bytes/identity and no nested AppendFailure wrapper.
Only NotInserted probes; preserve its checked finding independently of a later latest-state
decode/projection failure. A failed probe load uses observation: None and reload_cause: Some in
that same variant; it does not establish absence. Store errors return immediately without a probe.
Runtime returns these observed facts through the existing invocation result; clients present them
without reloading or reconstructing causes. Public-view construction failure retains any known
acknowledgement under Part 1 section 9.4. Part 2 must not reintroduce NativeCause, `Box<dyn Error>`,
a custom error protocol or another reporting subsystem.

The diagnostic trust boundary is owned by
[Part 1 section 4](RFC_AUDITABILITY_ERROR_CHAIN.md#4-error-preservation-and-the-diagnostic-trust-boundary).
MFM trusts dependency-supplied diagnostic content and does not scan or certify it as secret-free.
MFM does not deliberately attach its own secret inputs, requests, connection or keystore-command
objects. Actual admission limits and source fidelity remain required. No generic sanitizer or native getter/source
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
| Pin the baseline and inherited APIs | Record accepted Part 1 commit and actual Values DiagnosticEvidence/InvocationDiagnostic/SizeViolation, trusted-text profile, Failure/checked Object, concrete owner classification, Store and rendering definitions. Link those owning APIs; do not copy a second schema or reintroduce the deleted diagnostics crate. |
| Subtract completed work | Map E1-E6/C1-C18 and Part 1 sections 5.3/9.6 to completed cases. EVM response messages/data_json, exposed native-source fields, CLI output sources and the shared admission profile are already in the core cutover. Inherit E4's ordinary structural decoding and diagnostic/status exception; do not reopen seeds/map-only validation. E5 is not another constructor migration. |
| Freeze each error path | Name producing failure/function, selected concrete owner error, required cause layers/fields, boundary adaptation, receiving conversions, terminal observation, finite assertions, information limits, reuse and deletions. File lists or “all consumers” alone do not pass. No omission ledger is required. |
| Finalize the owner design | Specify actual changed variants/signatures and selected fields. The complete declared owner remains typed; facts used by classify() remain typed and any DiagnosticEvidence is nested data. Handlers receive Classification/RecoveryContext. Internal errors use InvocationDiagnostic at their heterogeneous boundary. Existing compatible interfaces need no wrapper or new source/schema/capture framework. |
| Prove admitted-original completeness | Compare required causal/operation facts in the concrete owner, admitted original, restored classifier input and report. Successful serialization alone is insufficient. Keep the agreed unavailable-detail exception for first encoding failure; no generic completeness checker, extra native copy or universal fallback interface. |
| Reuse diagnostic admission | Use Part 1's DiagnosticFloatFree profile and shared nested field. Freeze selected fields and preserve raw protocol error data as explicit data_json text where numeric spelling matters. Do not create per-owner text types, new number/secret policies, a second JSON validator or quota. Verify the whole owner and resulting report through the accepted admission route. |
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
    #[mfm(persisted)]
    diagnostics: mfm_values::DiagnosticEvidence,
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
DiagnosticEvidence travels inside that owner through Object admission and restoration. Restoration
reconstructs the same MFM owner with the selected foreign cause facts, not a live client error.
The classifier uses its typed semantic fields; the handler receives Classification/RecoveryContext,
not diagnostic JSON or the original error directly. A domain-only failure needs no diagnostic field.

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
Use Part 1's shared data without a diagnostic quota. Operational adapters build their selected
scalar/source fields directly; existing from_fields is the only generic invocation serializer.
Its two fixed construction-failure markers are invocation-only. Ordinary private Serialize helpers
are allowed where existing owner data lacks the selected field contract; no generic from_error
projector, per-owner report schema, second serializer service or global owner-error enum is needed.
Neither DiagnosticEvidence nor InvocationDiagnostic supplies a standalone MfmValue identity or
replaces the classifier's concrete error.

Provider error types retain their concrete roles; the inherited DiagnosticEvidence is the small
Values data wrapper, with no shared source vocabulary. Reuse its constructor, getter and nested
schema. New required native facts are supplied only by the selected producer's local recipe; they
do not require extending a global enum. Count actual changes and deletions against accepted Part 1,
including owner-local code; no nominal wrapper is needed where an owner already carries the facts.

## 4. Finite execution producer matrix

The five groups below are the complete producer scope. R0 must freeze exact cases and source
paths against accepted Part 1 and remove work already completed. The rows are not
permission to enrich every error in the named modules.

| ID | Selected producing boundary and facts | Consumer and stopping boundary |
| --- | --- | --- |
| O1 | Read/read_anchored and existing json_rpc send/body/RPC failure conversions on selected shipping Read calls: method/stage, status/code, exposed source chain, actual local mismatch and limits. | Runtime failed Read and cold App observation; internal preflight stays internal. Reuse Part 1's local RPC source-data recipe; R0 subtracts its completed cases. No HTTP client, retry or decoder-family rewrite. |
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

Preserve actual canonical/Object/frame/report limits. A required operational schema change uses
the same shared diagnostic field and ordinary admission; do not add a diagnostics quota, truncation
policy, omission accounting or another persistence mechanism. Neither the trust
policy nor this matrix requires arbitrary raw client object dumps or every dependency message.

## 5. Ordered work and completion

### R0: accept the finite execution design

Complete section 2 after Part 1 F1. The architect records accepted or rejected readiness against
the actual inherited APIs, frozen paths, required deletions and cumulative cost. No implementation
row starts with unresolved architecture. At this revision: **R0 is pending Part 1 acceptance.**

### R1: preserve selected recording causes

Complete O4's required run Store conversions and shared database extraction once, reusing Part 1
recording/reporting and admitted-original ownership, including Part 1 section 9.4's direct Store
variant without probe fields. Supply size facts at the concrete owner; do not add a new
source-discovery or erased-cause adapter to Store consumers. Cover failed execution plus failed
append, success plus failed append, actual disposition and independent cause chains. Do not repeat the core load
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
| B1 | R0 pins accepted Part 1, inherited APIs and every finite producing failure, required fact, conversion, consumer assertion, information limit, reuse/deletion and cumulative cost. Subtract completed EVM/CLI source-data cases; no broad owner or deferred nonexecution row remains mandatory. |
| B2 | Each selected concrete owner retains promised cause/operation fields. Declared errors restore the same MFM type with nested diagnostic data for classification. Semantic decision fields remain typed; neither the deepest source nor JSON/classification codes replace the owner. Handlers receive Classification/RecoveryContext. |
| B3 | O1/O2 operational errors, including required O3/O6 causes, survive complete admission and cold observation. Internal execution/recording failures use invocation reports without a fault append. Command/EffectId authority is unchanged. |
| B4 | Selected dependency messages/data follow the inherited trusted-text profile without credential scanning or source certification. MFM does not deliberately attach its own secret inputs. Whole-owner and terminal-report admission preserves floats/actual-limit rejection and ordinary-input policy; no diagnostic quota, truncation or omission ledger returns. |
| B5 | Capture occurs once at the selected producer. Receivers forward the same Values DiagnosticEvidence/InvocationDiagnostic data or admitted Object and primary size facts without downcasting or recapture. Compare required facts across admission/restoration/reporting. No mfm-diagnostics, global source/fact vocabulary, capture factory, second value eraser, NativeCause, seed/projector/custody tree, new size hierarchy, per-owner reporting schema or generic completeness checker remains. |
| B6 | An admitted execution failure plus recording failure retains two independent causes under Part 1 section 9.4: BeforeAppend requires an admitted original, while direct Store/NotInserted variants retain the exact submitted candidate and actual acknowledgement. No unsent-candidate custody or AppendFailure wrapper returns. Failed initial error encoding uses Part 1's ordinary internal diagnostic with known context, encoding cause and explicit unavailable-original detail under its fixed invocation-marker contract; no special Runtime variant, opaque custody or fallback payload. Success plus recording failure invents no original error. Preserve immediate Store-error return and independent NotInserted finding/reload cause. Reports preserve actual delivery stage; no second audit sink, append retry or recursive reporting. |
| B7 | Cutovers delete superseded paths/APIs/tests/docs while preserving meaningful behavior coverage. Actual aggregate complexity and production/test/doc/file/API costs are reviewed against both baselines; no compression, hypothetical deletion credit or duplicate native representation hides growth. |
| B8 | All frozen cases and relevant retained Part 1 behavior pass required checks and final CI. Completion names the accepted commit, evidence and limits; it does not claim platform-wide provenance or secret-free certification of upstream diagnostics. |

## 7. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| The accepted core supplies sufficient boundary diagnostics | K1-K4 implementation is not complete | Owner rows could recreate projectors, size discovery or duplicate originals | R0 inspects the implemented section 9 signatures/recipes and E1-E6 evidence. Reuse the no-budget constructor, fixed fallbacks and minimal typed errors; resolve only an actual missing producer contract. |
| Concrete execution error payloads retain required causal and classification facts | Correct associated types can still contain lossy variants or serializers | Cold classification/reporting could lose a promised fact | Compare selected concrete errors, admitted payloads, restored classifier inputs and consuming assertions. The first-encoding unavailable-detail exception remains settled; do not add generic custody. |
| Selected owner facts fit the inherited shared-data admission route | Exact remaining producers are frozen only at R0 | A row might duplicate admission, lose required facts or weaken ordinary-input checks | Verify the real concrete owner, restored classifier input and complete report with the required source data, floats/limits and marker cases. Reuse the accepted profile rather than reopening shared representation design. |
| The selected paths and cost are finite and simpler overall | Exact conversions and accepted Part 1 baseline remain unproved | Hidden dependencies or locally justified growth could recur | R0 freezes semantics, conversions and cost; exceeded estimates trigger review and F2 judges the actual aggregate result. |

The trust/scope, shared Values diagnostic data, concrete-owner classification, failed-initial-
encoding and checked-Object/stored-data diagnostic contracts are settled. Part 1's deletion-first K1-K4 sequence precedes R0;
implementation, exact remaining producer contracts and LOC improvement remain to be proved.
