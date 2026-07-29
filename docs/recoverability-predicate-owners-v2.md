# Recoverability Predicate Owners v2

Status: schema-frozen predicate-ownership inventory for the current contract

Contract id: `mfm.recoverability-predicate-owners.v2`

This document records the predicate-ownership inventory that closed before the recoverability
schema freeze. The frozen target encodings and shared vectors are
`contracts/recoverability/v2/annex.json`, `contracts/recoverability/v2/corpus.json`, and
`contracts/recoverability/v2/README.md`; exact artifact hashes and counts are recorded by
`C11_ARTIFACT_METADATA` in the RFC's
[Canonical Schema and Golden-Vector Gate](../RFC_REFACTOR_RECOVERABILITY.md#canonical-schema-and-golden-vector-gate).
It refines the accepted target in
[`RFC_REFACTOR_RECOVERABILITY.md`](../RFC_REFACTOR_RECOVERABILITY.md), especially
[Predicate ownership](../RFC_REFACTOR_RECOVERABILITY.md#predicate-ownership), without creating a
second runtime or persisted-data contract.
[`design.md`](design.md) and [`architecture.md`](architecture.md) are authoritative for the current
implementation. Retained gates and deletion inventory stay in
[`recoverability-cutover-gates-v2.md`](recoverability-cutover-gates-v2.md).

## How to read the matrix

Each `P-*` row below is one atomic predicate family with one decision owner. A row may contain
several inseparable checks only when they have the same owner, consume the same authority, and
produce one closed claim. The row id is the stable review key. Coverage tables may reference an id,
but no predicate is normatively defined twice.

The columns have precise meanings:

- **Sole owner** decides the predicate and is the only boundary allowed to construct its positive
  authority or semantic verdict.
- **Seal or enforcement point** makes the decision non-forgeable, durable, or race-safe.
- **Required redundant verification** reconstructs the same predicate after hostile bytes,
  type erasure, storage reload, or concurrency. A verifier may reject; it cannot choose another
  request, policy, result, tenant, grant, resource owner, or finality claim.

Redundant verification is required defense in depth. Duplicated authority exists when two
components can independently make different positive decisions for the same row. The design is
invalid if both decisions could survive. In particular:

```text
owner decision + exact-bound recheck = redundant verification
owner decision + independently selectable alternative = duplicated authority
```

Persisted bytes are evidence, not self-authenticating authority. A sealed in-memory proof is also
not persisted truth: it must bind the exact retained inputs and candidate, and the store must
rederive every structural predicate available from those inputs.

## Hostile bytes, canonical decoding, and retained values

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-HB-01` | Bytes claiming a versioned canonical schema have the exact version, tag, fields, ordering, integer widths, optional/empty semantics, bounds, canonical JSON, and no floats. | The frozen canonical schema annex and shared domain-free codec. | Closed encoders and strict decoders construct the typed schema value or reject it. | Certifier, memory/PostgreSQL loaders, replay, trace/export codecs, and golden-vector consumers rerun the same codec; none may accept a backend-specific or serde-default variant. |
| `P-HB-02` | Every semantic identity equals `SHA-256(JCS({"domain": exact_versioned_domain, "value": exact_schema_preimage}))`; raw content addressing alone is `SHA-256(exact retained bytes)`. Prefix/NUL framing, alternate envelopes, floats, and compatibility decoding are illegal. | The shared `mfm-canonical` digest/reference implementation and frozen annex domains. | Private reference constructors derive identities from validated canonical values; callers cannot supply a claimed digest independently. | Certifier, store append/load, object loader, replay, executor, and export rederive the same value. Store-assigned coordinates are covered separately by `P-ST-04`. |
| `P-HB-03` | A lightweight `mfm_ids::ContentRef { schema_id, content_digest }` identifies reviewed content but grants no retention or producer authority. Retained object bytes exactly match their full journal `ValueRef`: artifact id, content digest, evidence hash, length, media type, schema, semantic type, producer binding, and role. | `mfm-ids` owns content identity; the `RunJournalStore` object-authority boundary owns journal admission/reachability. | Typed executor references wrap `ContentRef`; atomic object admission plus a verified-object constructor ties each `ValueRef` to one committed path binding. | Runtime materialization, recorded-history verification, fact scan, trace/export dereference, memory, and PostgreSQL recheck bytes and evidence; neither reference is bearer authority and a `ContentRef` cannot substitute for a `ValueRef`. |
| `P-HB-04` | A persisted or public typed shape has only reviewed, bounded, non-secret fields and contains no prohibited surrogate hash, arbitrary error/debug text, endpoint, path, credential, bearer, or raw provider body. | The exact certified value/capability/executor schema selected for that field. | Closed constructors, private fields, bounded canonicalizers/classifiers, certification qualification, and adversarial leakage tests. | Every decoder and renderer enforces the closed schema and size bounds. Rechecking representation is not an independent claim that arbitrary semantic bytes are non-secret. |
| `P-HB-05` | Hostile provider or executor response bytes become either the exact reviewed typed result or the shared `mfm-capabilities::SafeFailure` envelope selected by `safe_failure_contract_ref`; unrepresentable bytes are neither retained nor fingerprinted. The closed universal class includes cooperative `cancellation`, and a result that survives before/after possible entry maps to the exact `DidNotEnter`/`Indeterminate` stage. | The admitted capability or executor classifier/canonicalizer executed inside the audited wrapper. | Only the wrapper can construct `UncommittedAccessObservation`; raw bytes and open diagnostic maps have no persistence path. | Callback-free observation append/load and replay verify contract reference, code, universal class/stage/size combination, bounds, and wrapper binding for every observation. Classifier-bound reads additionally strict-decode the classifier and verify its exact embedded diagnostic `SchemaIdentity`, derived schema id, and complete structural `SchemaShape`; reference-executor failures instead verify their exact contract-specific tuple and carry no diagnostic identity. Neither path reparses discarded bytes or invents a better classification. |

The universal safe-failure class vocabulary is exactly `authorization | configuration | request |
cancellation | transport | destination | unrepresentable_response | integrity |
resource_conflict | unclassified`. Its stages are exactly `before_boundary_entry |
boundary_entry | boundary_observation`; optional size buckets are `zero | up_to_16_kib |
up_to_1_mib | over_1_mib`: `zero` is exactly 0 bytes, `up_to_16_kib` is 1 through 16,384,
`up_to_1_mib` is 16,385 through 1,048,576, and `over_1_mib` begins at 1,048,577. Null means the
size was unavailable. A safe diagnostic is at most 16,384 canonical bytes.

EVM read contracts allow exactly `routing_generation_unavailable`, `configuration_invalid`,
`request_invalid`, `access_cancelled`, `transport_failed`, `http_status`, `json_rpc_error`,
`response_invalid`, `response_missing_result`, `response_too_large`, and
`unclassified_failure`. Bitcoin permits the same list plus `scan_busy`. Their only diagnostics
are `HttpStatus { status: u16 }`, `JsonRpcError { code: i64 }`, and `ResponseInvalid { kind:
malformed_envelope | missing_result | invalid_result | too_large }`; Bitcoin additionally permits
the fieldless `ScanBusy`. The accepted RFC's security table owns the exhaustive observation,
class, and stage mapping. Source/chain/network mismatch, anchor drift, and Bitcoin
`scantxoutset.success = false` remain returned typed semantic failures, not safe-failure codes.

The same RFC table also owns the exact `P-AR-06` callback verdict for every structurally
admissible EVM or Bitcoin read failure. Route-generation, configuration, and request-invalid
observations are `InvalidEvidence` because they violate the pre-authorization catalog and total
request-author invariants. Unrepresentable-response observations are also `InvalidEvidence`.
Cancellation, transport, unclassified failures, the exact retryable numeric sets, and exact
Bitcoin `scan_busy` are `InsufficientEvidence`; every other classifier-admitted numeric
destination rejection becomes the frozen typed terminal read-validation failure. Exact semantic
reproduction reruns those verdicts under `P-AR-09`; structural append/load/replay verifies the
envelope relation but never chooses a verdict.

## Typed construction and certification

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-TC-01` | State, config, context, input, output, fact, request, evidence, capability, and typed-handle schemas are mutually compatible, with valid lineage and non-forgeable producer positions. | Typed program builders and sealed typed constructors. | Branded handles and closed constructors exclude invalid combinations before an authored program exists. | The certifier independently validates the retained authored program and expanded graph; decoders reconstruct the same branded facts only from verified references. |
| `P-TC-02` | Each state contract selects exactly one closed execution kind—pure, read, or recoverable effect—with the exact callback surface, schemas, capability or executor contract, canonicalizers, failure policy, and total request author. | The registered state contract accepted by `mfm-certify`. | Closed `StateExecution` construction and registration qualification; no custom runner/lifecycle variant exists. | Certification and exact qualified-registry binding compare the complete contract. Runtime dispatch and reproduction verify the selected reference but cannot alter the execution kind or callback surface. |
| `P-TC-03` | The canonical authored program contains the operation-owned static topology, typed bindings, child composition, domain lineage, and public entry-point objective intended by the operation contract. | The selected operation contract and its deterministic authoring implementation. | The authored program is canonical and content-addressed before framework/executor expansion. | The certifier checks the entry-point/operation relationship and retains the authored artifact. Runtime never repairs or adds topology. |
| `P-TC-04` | One exact `PlanningProfile` and planner produce the byte-identical composite expansion once, in child-composition → framework-outer → executor-inner order, with stable node identities and complete effective-output rewiring. | The bound composite planner contract, executed as certification authority by `mfm-certify`. | The certificate binds authored program, profile, planner contract/implementation, expansion contracts, and final spec hash. | Certificate verification reruns the pure planner. Admission compares the retained result. Runtime executes the expanded graph and has no wrapper/origin branch. |
| `P-TC-05` | The expanded graph has no unresolved, recursive, or cyclic executor expansion; every input dependency, skip outcome, effective output, public output, and terminal combination is total. | `mfm-certify` graph and terminal-contract validation. | Certification refuses to mint `CertifiedTypedSpec` for a partial truth table, bypassed post-chain, raw protected output, or incompatible fact emission. | Shared structural fold and replay recheck the certified dependency/terminal consequences against history; they never add a missing rule. |
| `P-TC-06` | A `ComponentImplementationRef` denotes the exact canonical semantic contract, callback surface, and qualification artifact, and the exact `(executable_identity_ref, component_ref)` pair is approved without duplicate registration. | The certification qualification boundary. | The certificate and state/capability implementation manifests retain the exact approved pair and reject ambiguity. | Admission, qualified program-registry assembly, resume, exact reproduction, and candidate comparison require reference equality. None may treat a component descriptor as code equivalence. |
| `P-TC-07` | `executable_identity_ref` is the content address of the exact bounded, stably opened whole executable bytes under `mfm.executable-bytes.v1`. | The platform self-attestation implementation in app bootstrap. | One fail-closed bounded file read produces the identity; there is no caller override, label identity, or equivalence alias. | Admission retains it; live drive/resume and exact reproduction self-attest and compare before callbacks. Structural verification checks only the retained binding and never opens an executable. |
| `P-TC-08` | The capability binding manifest selects exactly one complete read binding or executor binding for every used contract, including admitted local implementation and all required source/deployment identities. `ExecutorDeployment.tenant_scope_id` is the scalar `TenantScopeId`, not a `ContentRef`, and equals the run tenant and executor-ledger partition. | Certification and admission manifest validation. | Canonical content-addressed manifests are bound into the certificate and `RunAdmitted`; incomplete, duplicate, wrong-tenant, or mismatched variants reject before executor lookup or mutation. | Qualified-registry binding, authorization append, executor, replay, and exact reproduction compare the same references and scalar tenant. They cannot fill a missing source scope, routing generation, executor deployment, evidence authority, or resource owner. |
| `P-TC-09` | A `CrossRunSourceRef` has the certified effective-output or evidence-only role, exact closed source lineage, acyclic dependency closure, and same store/epoch/tenant scope required by the baseline. | Certifier/app admission source verification. | The exact source manifest and transitive proof-closure root are bound into the root candidate before `AdmitRun` is sealed; transfer segmentation never changes that root. | Store verifies root hashes/scopes/closure coordinates; recorded-history verification checks the complete retained proof DAG through bounded deterministic steps. Neither an incomplete step nor a copied object reference is source authority. |

## Admission and runtime semantics

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-AR-01` | A normal `run_id` and logical admission identity are derived from exact store scope, tenant, stable entry-point id, and non-secret invocation identity; acknowledgement retry denotes that same logical start. | The published app entry-point admission contract. | App derives the root candidate and sealed `AdmitRun`; raw caller run ids are not admission authority. | Store rederives the id and enforces root equality/uniqueness under `P-ST-03`; replay verifies the retained root. |
| `P-AR-02` | A correction family that claims at-most-once derives its invocation identity from exact source bindings, typed correction purpose, affected resource/subject, and any legitimate explicit sequence. | The certified correction operation contract. | App/certifier rejects caller substitution before using the ordinary admission path. | Store applies only ordinary logical-start uniqueness; replay checks the retained derivation. No correction-specific store key or alternate runtime exists. |
| `P-AR-03` | A root candidate has a valid certificate, exact profile/spec/manifests, config/seeds/context, cross-run proof closure, object evidence, genesis predecessor, and initial semantic-state digest. | The certifier/app admission verifier. | Successful verification constructs the sealed `AdmitRun` candidate. | Shared structural verifier and store rederive all byte-, scope-, object-, genesis-, and digest-level predicates; they do not rerun planners or business correction logic during append. |
| `P-AR-04` | A live callback or capability uses the exact admitted whole executable, selected component entry, schema, immutable routing generation, and binding; missing or mismatched deployment material blocks before semantic execution. | Exact runtime selection through the sole `QualifiedProgramRegistry`. | The registry seals the admitted support graph, candidate callbacks, semantic read/effect entries, and process-private live invokers as one selection. Exact candidate and entry resolution precede callback or live-access construction and perform no semantic probe. | Authorization append verifies selected references. Exact reproduction repeats the equality gate through the same sealed candidate callbacks. Store and structural replay never construct a second registry or live invoker. |
| `P-AR-05` | One `StateFrame` contains the exact predecessor-visible config, context, input tree, typed bindings, object evidence, and source lineage for that node occurrence. | The runtime `VerifiedRunView`/`StateFrame` construction boundary using the shared structural verifier. | Only verified view authority can construct the private proof behind the program-owned value view. | Store checks the sealed semantic proof binds the exact predecessor and input manifest; recorded-history verification reconstructs the same manifest callback-free. State code cannot add ambient inputs. |
| `P-AR-06` | Request authorship, observation acceptance verdicts, domain failure policy, reduction/settlement, typed outputs, and fact emissions for an occurrence follow the exact admitted state contract. | The state callback selected by the exact qualified program registry. | Runtime invokes it with sealed committed proofs behind borrowed views, validates canonical settlement, and binds the result into one transition candidate. | Store verifies exact proof/candidate/reference coverage without running the callback. Exact reproduction may rerun the same callback; structural replay never substitutes one. |
| `P-AR-07` | Structural readiness, node-slot legality, immutable frozen-read-intent identity, effect-awaiting identity, and candidate availability follow only from the certified graph and predecessor-visible committed bindings. | The shared domain-free structural verifier. | `VerifiedRunView` exposes closed structural candidates; the first winning authorization atomically establishes the frozen intent in the fold. | Runtime consumes candidates but cannot broaden them. Store rederives them at the candidate predecessor; replay uses each historical predecessor rather than final state. |
| `P-AR-08` | `compatible(observation, occurrence)` and `structurally_consumable_at(observation, occurrence, predecessor_head)` use the exact authorization/observation chain, occurrence, frozen read or effect identity, operation, binding, request, schemas, own fact frontier, prefix visibility, open slot, callback-input kind, and admissible role. | The shared domain-free structural verifier. | One canonical implementation produces the predecessor-relative ordered candidate sequence for live runtime, append, and replay. | Runtime may decide callback sufficiency only over this sequence. Store independently rederives it and checks proof coverage. No final-head recomputation may rewrite historical eligibility. |
| `P-AR-09` | Runtime scans the complete physically ordered structurally consumable observation suffix. Every item before the first `Settlement` is `InsufficientEvidence`; that first settlement is remembered and no later settlement replaces it, but any `InvalidEvidence` anywhere in the suffix—including after the remembered settlement—blocks it. Only a complete all-insufficient scan may authorize another access. | Runtime invoking the exact state callback. | The sealed semantic proof binds the full suffix and every verdict without short-circuiting at a settlement winner or evidence gap. | Store rederives sequence completeness and proof binding without deciding sufficiency. Exact reproduction reruns verdicts; recorded-history verification checks only retained structural legality. |
| `P-AR-10` | `drive_once` chooses the one closed action priority: block on any full-suffix or other integrity finding; report an existing closure; block the impossible open/all-terminal fold as an integrity failure; then, for an open run, settle retained evidence, commit ready local work, or authorize the least-authorized eligible occurrence; finally report an operational block or ordinary waiting. Closure is attached only to the final semantic transition and is never derived as a standalone action from an open/all-terminal fold. | Stateless runtime action derivation. | One verified view produces at most one transition or one live application-protocol operation; stale append reloads. | Scheduler model/conformance tests rederive the ordering. Store admits only a legal candidate but does not choose among multiple legal runtime actions. |
| `P-AR-11` | Exact admitted `routing_generation_ref` resolution is non-semantic binding work; inability to resolve blocks, and no pre-admission probe, fallback, failover, provider reselection, or newer-generation substitution is legal. | App/runtime live binding for the admitted capability manifest. | Qualified program-registry construction resolves only the immutable generation; semantic source/chain validation is an audited post-admission read. | Authorization records and replay compare the retained binding. Store validates reference equality but never resolves routes. |

## Sealed journal append and shared structural store

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-ST-01` | A batch has exactly one of eight frozen `BatchPurpose` tags: `run_admission`, `pure_settlement`, `read_settlement`, `dependency_skip`, `external_access_authorization`, `external_access_observation`, `effect_request`, or `effect_settlement`. Those purposes project onto four exhaustive `LegalCommitBatchFields` structural variants: `RunAdmission`, one `Transition` with optional inseparable closure, `Authorization`, or `Observation`. Effect request and effect settlement are distinct purpose tags within the transition variant; prohibited combinations are absent. | The sealed `RunJournalStore` append contract. | Variant-specific `PreparedJournalAppend` construction plus whole-batch validation under the store transaction. | Memory and PostgreSQL decode and run the same structural verifier; replay rejects any loaded illegal shape. |
| `P-ST-02` | The append names the exact current per-run predecessor, and its semantic proof is sealed, in-process, exact-candidate/executable/component/manifest/input/evidence bound, and not client-supplied bearer authority. | Runtime/certifier for semantic proof construction; the store owns acceptance of that exact binding. | Private candidate constructors hand one opaque proof to a sealed append method; store matches it byte-for-byte to the candidate and predecessor. | Both backends rederive structural inputs and reject stale or mismatched proof. This split is semantic ownership plus structural enforcement, not two semantic decisions. |
| `P-ST-03` | Per-run compare-and-swap, append-request idempotency/conflict, logical admission uniqueness, record/slot uniqueness, one closure, and one observation per authorization are enforced atomically. | `RunJournalStore`. | Stable run-root/admission-key locking, exact predecessor reload, unique keys, and one transaction or infallible memory swap. | Load/replay verify immutable uniqueness and predecessor linkage. Runtime retries but cannot declare a stale append successful. |
| `P-ST-04` | Store-assigned run sequence, record ordinal/id, tagged tenant fact coordinate, routing copies, candidate/record/commit digests, and final `JournalHead` are derived from the validated batch under the exact canonical preimages. | `RunJournalStore`. | Store-only coordinate assignment occurs after validation and before all-or-nothing publication. | Shared codec, memory/PostgreSQL parity, replay, and trace/export rederive hashes and routing. Callers never author assigned coordinates. |
| `P-ST-05` | Object-path bindings and artifact-admission intents are complete, exact, sorted, mode-consistent, and atomic with the first authoritative record reference; preexisting input cannot self-admit. | `RunJournalStore` object-admission contract. | `RequireExisting` versus `AdmitOrVerifyExact` is derived from closed field paths and committed with the batch. | Both backends and replay compare bound identity sets, evidence, bytes, modes, producer role, and no extras/omissions. |
| `P-ST-06` | A live access permit exists only for a directly observed winning `NewlyAppended` authorization; `ExistingSame`, reload, stale loss, commit ambiguity, or reconciliation mints none. | `RunJournalStore` authorization-result constructor. | Non-cloneable `NewlyAppendedAuthorization` is returned only on the positive commit path and consumed by runtime. | Runtime checks its exact binding before constructing affine access. Tests reconcile ambiguous appends without authority. Persisted records alone cannot recreate the permit. |
| `P-ST-07` | An observation has one committed matching authorization, exact wrapper/binding/request/schema, at most one result, idempotent same-content retry, conflict on changed content, and legal pre-closure or constrained late-tail placement. | `RunJournalStore` observation append contract. | Only the sealed wrapper result can build `ObserveExternalAccess`; the store validates under the current run transaction. | Structural reload/replay verifies chain, role, safe schema, uniqueness, and late-tail legality. It cannot synthesize an observation for an unmatched authorization. |
| `P-ST-08` | The canonical node/run fold, state digest, binding delta, effect slot, deterministic skip reason, terminality, closure digest, and post-closure audit-only behavior agree for the whole batch. | The shared structural verifier executed by `RunJournalStore`. | Store applies the complete candidate fold or nothing; human-readable phases/deltas are checked redundancy only. | Runtime derives candidates from the same fold; replay reconstructs every historical prefix. Projection/checkpoint data cannot override it. |
| `P-ST-09` | A backend publishes no head, idempotency entry, object authority, coordinate, or partial row set before every fallible validation succeeds. | Each `RunJournalStore` backend implementation under the common atomicity contract. | Memory performs one infallible staged swap; PostgreSQL performs one transaction and commits last. | Injected-failure parity compares memory and PostgreSQL for every legal batch and coordinate. |

## Audited capabilities and recoverable executors

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-EX-01` | One affine MFM access authority is consumed before local validation and permits zero or one independently meaningful application-protocol operation with no hidden retry, redirect, failover, provider reselection, adaptive subcall, or independent batch member. | The qualified audited capability wrapper. | Private `AuthorizedReadAccess`/`AuthorizedEnsureAccess` can enter only the registered wrapper once; local rejection yields `DidNotEnter`. | Capability conformance and adversarial transport tests verify one-entry behavior. Journal audit proves prior authorization, not exact packet or remote-call count. |
| `P-EX-02` | A read capability performs protocol encoding/decoding, exact source/session validation, and safe classification for only the state-authored typed request; it cannot reduce state, construct outputs/facts/failures, or define replay. | The exact admitted read capability contract and implementation. | Runtime passes an affine authority bound to immutable request/binding/operation identity. | Observation verification checks the binding and result schema; state callback owns interpretation under `P-AR-06`. |
| `P-EX-03` | An effect request is immutable once committed, and its kernel-derived effect key binds exact executor binding, store scope, run, and node; another request digest or binding under that key conflicts. | Runtime's effect-request constructor using the certified state request and kernel key derivation. | `EffectRequested` commits before any `ensure`; later authorization accepts only its `CommittedEffectRequest`. | Store checks slot/key/request/binding immutability. Executor ledger independently requires the exact same identity; it cannot choose another MFM request. |
| `P-EX-04` | A pending effect is routed only to its exact admitted executor contract, local client/verifier, deployment namespace/generation, scalar `TenantScopeId`, evidence authority, and required resource owner. For the EVM wallet, one sealed qualification additionally fixes the actual routing catalog and complete generation-to-chain map, selected route/chain, exact executor semantic/evidence closure, and derived signer/policy identities before admission or allocation. | The certified `ExecutorBinding` selected at admission; `mfm-evm-live` solely owns the wallet-specific cross-field qualification. | Content-addressed binding/deployment/resource objects and exact run/ledger tenant equality are resolved and verified before `ensure`. Application admission and the wallet executor share one qualification `Arc`; admission checks it before certification/append and execution checks it before effect binding/resource allocation. | Runtime, executor ledger, authorization, terminal evidence, restore/failover, and replay compare the one `ExecutorBindingRef`; wallet admission/execution compare the same sealed qualification. Copied component tuples, caller-supplied route descriptors/policy pairs, or tenant-as-content-reference encodings have no authority. |
| `P-EX-05` | Executor delivery evidence is one bounded immutable prefix chain from `EffectBound` through allocation, target authorizations/observations, and optional terminal tombstone; same/equal/ancestor/descendant handling, finite bounds, and fork rejection follow the exact executor contract. | The qualified executor ledger and its admitted pure frontier verifier. | Positively acknowledged executor-internal append mints target-entry authority; each result returns the complete new suffix/frontier and proof. | MFM runtime verifies before observation admission; store retains exact objects; replay verifies predecessor, proof, bounds, identity, and tombstone without querying mutable executor state. |
| `P-EX-06` | Terminal executor evidence states one exact external operation/outcome and explicit proof basis/authority, and its tombstone, delivery frontier, domain proof, assurance policy, and executor binding are mutually consistent. For the EVM wallet, the complete deterministic attempt history must derive the exact terminal request, ordered prior-result references, candidate lineage, transaction, receipt, finalized head, canonical inclusion, outcome, executor generation, fence, and assurance; the tombstone must name that terminal attempt's exact returned result and observation. Immutable append order governs availability: each authorization freezes its plan from authorization-ordered results observed by then, the first observed valid terminal is selected, and later observations are validation-only. | The generic qualified executor terminal-evidence contract owns the exact proof tuple; the single `mfm-evm-live` wallet-history fold solely owns the stronger EVM history relation. | After qualification and permanent allocation, the wallet fold runs before target authorization/observation, signer/RPC entry, terminal append, pending/terminal return, and authorization/terminalization-conflict result. Only its first observed derived terminal attempt may authorize the tombstone, which the executor durably commits before returning it; legal later and post-tombstone observations are checked against their authorization-time frozen plans without changing that selection. | Runtime runs the admitted pure verifier; the effect state's `settle` callback separately decides the MFM domain settlement under `P-AR-06`. Restored-history tests inject generic-valid hostile descriptors, terminal composites, tombstones, and a two-inclusion tail observed/tombstoned out of authorization order; rejection is byte-identical with zero signer/RPC calls, while the valid late tail replays read-only. Exact proof-tuple corruption remains rejected by the generic frontier verifier. |
| `P-EX-07` | Cross-effect resource key, permanent allocation, fencing, safe reuse, and ownership against every capable actor are defined by one typed executor policy and authoritative executor/destination owner; delayed old authority cannot conflict with a new allocation or generation. The EVM initial-nonce configuration binds nonce, source-attestation identity, wallet domain, chain, sender, and durable generation. | The bound executor/resource-ownership domain and destination fencing authority; `mfm-evm` owns the canonical nonce policy and initial-nonce descriptor. | Immutable resource stream CAS and allocation commit precede target IO; the wallet derives the policy/configuration pair only from the sealed qualification. Generation transfer preserves the ledger or permanently fences the old generation. | Admission verifies required ownership identity and the complete wallet qualification; MFM retains allocation/fence evidence; replay verifies it. MFM settlement, local locks, worker leases, transport state, or caller-supplied policy references cannot release or reassign the resource. |
| `P-EX-08` | The generic executor substrate defines immutable keyed effect/resource streams, append/CAS, bounds, and target-entry authority independently of any backend. A PostgreSQL backend implements that contract but is not itself non-rollback, stale-writer, or destination-convergence authority. | `mfm-executor` owns the generic contract; the concrete executor backend owns transactional enforcement; the deployment's external generation/destination fence owns promotion safety. | Backend commit precedes target entry, while the independently qualified deployment fence preserves the exact ledger generation and permanently excludes stale/sibling writers and unsafe resource actors. | Durable reference-executor and restore/failover/split-brain tests must pass before production registration. The MFM store's separate `P-PG-06` writer fence cannot substitute, even when both use one PostgreSQL deployment. |

## Facts and cross-run completeness

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-FA-01` | A fact-selection query is authored from certified base inputs, and the response is interpreted into state output/failure/facts under the exact query contract; same-run facts use graph edges. | The fact-selection read state callback. | One frozen `FactSelectionRequest` follows the ordinary audited read protocol and reducer. | Runtime binds request/response observations; exact reproduction reauthors and reruns interpretation. The reserved capability cannot author state output. |
| `P-FA-02` | The reserved scan returns exactly the qualifying facts from other runs in the admitted tenant through its authorization frontier, with verified producer transitions/objects/content identities and deterministic predicate, ordering, limit, and tie-break. | The non-overridable `mfm.journal.fact-selection.v1` capability over the same `RunJournalStore`. | One winning affine authorization creates a private, non-cloneable, non-serializable `FactScanSession` containing the authorization, request, exact frontier, next `(fact_order, fact_ordinal)`, and bounded top-K accumulator. Each contiguous writer step consumes the session and returns only private `More`; only reaching the frontier constructs the one reviewed response. Step budgets never truncate or invalidate the total prefix, and app cannot inject another fact store. | Recorded-history verification uses the same scanner from sealed replay authority to detect selected-item alteration or omission; exact reproduction may compare the request. Any future candidate index must rehydrate from this authority and cannot decide completeness. |
| `P-FA-03` | A fact-emitting transition gets exactly one positive dense tenant publication order; a selection authorization snapshots the current tenant head without advancing it; rollback creates no gap; same-tenant publication/barrier commits are totally ordered and different tenants are independent. | `RunJournalStore` tenant fact-coordinate allocator. | The fact publication or barrier is assigned under one tenant-head lock held through commit and covered by `commit_digest`. | Store open verifies head/publication/barrier union, density, range, tenant and routing copies. Memory/PostgreSQL parity rechecks structure; no caller or replica assigns a coordinate. |
| `P-FA-04` | `SameStoreVerified` means an exact response was omission-checked through its own qualified barrier on the fenced writer; portable or unavailable-prefix verification is explicitly `Unverified` and cannot be upgraded by density or reproduction. | The sealed store-owned fact-prefix verifier reached through the consuming run's `Replay` authority. | It combines the qualified append-time barrier attestation with the same authoritative scanner and returns the closed completeness verdict only from `Complete` at the exact frontier; `More`, cancellation, or discarded scratch has no authority. | Reload verifies immutable barrier binding/range/dense prefix and response frontier. Portable verification checks included facts only; it never mints absence proof or exposes unselected producers. |

## Recorded history, reproduction, trace, and presentation

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-RV-01` | Supplied journal, objects, certificate/proof closure, executor evidence, cross-run sources, fact assurances, folds, and closure form one callback-free `VerifiedRunView`; a portable cross-run closure is acyclic, complete, and deduplicates payload bytes without erasing any logical `ValueRef` binding. | The shared recorded-history verifier. | A private, non-cloneable closure-verification session contains the root, deterministic pending traversal, active/verified sets, object offsets, and counters. Each step consumes it; bounded `More` results have no positive meaning, and only an empty pending set after every reachable typed object verifies constructs the opaque view. Missing/conflicting/cyclic/tampered data rejects, while total source/object/byte size does not. | Store-backed loading may supply writer-qualified bytes and same-store fact completeness, but cannot skip verification. Lost scratch restarts from immutable inputs; runtime consumes only the complete view and cannot repair it. |
| `P-RV-02` | Exact semantic reproduction is attempted only after `P-RV-01`, exact executable/component equality, and OS-enforced capability denial; it reports `Matched`, `Mismatch`, or `Unavailable` without changing history or fact assurance. | The exact-reproduction service. | A separate process/boundary gates identity and invokes only admitted pure planner/state callbacks over retained inputs. | Sandbox attestation and reproduction tests check no live/store mutation authority. `Matched` is evidence about that run, not executable equivalence or authority to append. |
| `P-RV-03` | Cross-version comparison binds one explicit candidate executable/profile/manifests and reports per-plan/per-transition `Agrees`, `Differs`, or `NotComparable` without feeding candidate outputs forward or minting authority. | The candidate-comparison diagnostic service. | It consumes an existing `VerifiedRunView`, gates candidate identity, and stops before evidence use on request/schema mismatch. | Tests verify no live capability, store append, public output, correction, fact, resume, or completeness-upgrade path exists. |
| `P-RV-04` | Transition trace, run status, ready-node and pending-effect views, certified public output, and render payload are exact purpose-specific derivations from one verified journal view and certified graph; rendered JSON and projections are non-authoritative. | The corresponding purpose-specific reader over `VerifiedRunView`. | Readers expose only their closed DTO/proof and never persist a lifecycle snapshot. | App/binary rendering compares typed evidence and access scope. Rebuilding after projection/cache loss must produce the same result. |

## App authorization and purpose-bound access

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-ACL-01` | An authenticated principal may receive a particular `Admit`, `Drive`, `Replay`, `ReadPublic`, `InspectTrace`, `InspectAudit`, or `Export` grant for an exact store/tenant/run or admission candidate. | App authentication and authorization policy. | App mints a sealed, non-serializable `RunAccessAuthority<Grant>`; credentials, principals, and ACL snapshots do not enter the journal. | Kernel/store validate only token type and binding. They never consult another policy oracle or broaden a grant. Revocation prevents future minting rather than rewriting history. |
| `P-ACL-02` | A presented run authority's grant, store scope, tenant, run/admission identity, and requested operation match exactly. | Kernel/store sealed authority-binding validator. | Every authority-bearing entry point validates before loading records, objects, or constructing drive/replay authority. | Purpose-specific app services may precheck for UX, but cannot mint a positive result when the sealed binding fails. |
| `P-ACL-03` | Each grant dereferences only objects reachable in its reviewed closure: the single folded aggregate `PublicOutputAssembly` for `ReadPublic`; exact run history for `Replay`; reviewed trace/audit/export closure for privileged grants; no arbitrary object, producer-output, source-run, or unselected-fact access. | The purpose-specific app/store reader. | Reachability is checked before each record/object load and result construction; public-output subtrees are projected in certified binding order from the aggregate, and record ids, digests, fact refs, run ids, portable stream headers, and export content references are not bearer tokens. | Trace/export/replay codecs recheck closure membership. Cross-run source dereference requires its own authority except the sealed non-disclosing fact verifier. |
| `P-ACL-04` | An offline `mfm.portable-run-export-stream.v1` sequence may be verified only from its supplied frames and external `ContentRef` and conveys no live store/object dereference, drive, append, or same-store completeness authority. No frame contains a stream digest. Its sole transport integrity value is `SHA-256(every record separator, canonical frame byte, and line feed)`, returned externally as transport metadata and `Mfm-Content-Digest`. | The offline verification and purpose-bound export entry points. | Export authorizes and verifies the complete deterministic closure before writing any byte; verification checks the external digest, exact framing/order/EOF, and complete closure before constructing only a callback-free view with portable fact assurance and no store handle or run token. | API/compile-time tests keep live store methods and `AuthorizedAccess` unreachable and reject internal digests, legacy schemas, missing/extra/reordered frames or authorities, truncated streams, trailing bytes, or partial closure. |
| `P-ACL-05` | Ordinary public status exposes only `active|succeeded|failed` plus reviewed active/public-output fields; trace, audit, facts, cross-run list/watch, arbitrary objects, and manual semantic overrides are absent without separate contracts. | App public-surface contract, rendered by CLI/REST. | Opaque facade methods require `ReadPublic` and return closed DTOs; binaries only decode and render. | CLI/REST contract tests reject deleted fact/list/watch routes, implementation construction, generic object access, and “mark successful/not applied” endpoints. |

## PostgreSQL, database ACL, and deployment fencing

| Id | Predicate | Sole owner | Seal or enforcement point | Required redundant verification |
| --- | --- | --- | --- | --- |
| `P-PG-01` | Normalized PostgreSQL rows are a lossless physical representation of canonical commits, records, objects, bindings, logical keys, routing fields, and schema version—not a second lifecycle model. | `mfm-storage-postgres` schema/migration and row codec. | Closed columns, constraints, foreign keys, unique keys, and canonical payloads round-trip through the shared verifier. | Schema validation, online SQLx checks, golden vectors, load/replay, and memory parity reject missing/extra/reordered or backend-invented authority. |
| `P-PG-02` | Concurrent appends serialize on one stable run root/admission logical key, resolve idempotency and reload the head after that lock, validate before coordinate assignment, and commit all rows atomically. | PostgreSQL store append transaction. | `READ COMMITTED`, stable root/admission locking, final unique predecessor checks, and transaction commit implement `P-ST-03`/`P-ST-09`; the mutable latest row is never the mutex. | Concurrency and injected-failure tests compare the shared store contract. The SQL mechanism enforces, but does not reinterpret, runtime semantic proof. |
| `P-PG-03` | The tenant fact head is protected, updated only by `+1` with one fact publication, snapshotted without change by one barrier, locked after the run lock and through commit, and structurally reconciled on open. | PostgreSQL store fact-coordinate transaction and protected schema path. | Row locks, restricted update procedure/trigger, uniqueness, range constraints, and rollback provide the qualified append-time barrier attestation. | Dense-prefix/open checks and PostgreSQL parity tests rederive retained structure. Historical head equality is preserved as qualified attestation, not invented as portable proof. |
| `P-PG-04` | The application database role can insert/select only through approved paths and cannot update/delete/truncate immutable store identity, heads, journal, records, blobs, admissions, or bindings; migration ownership is separate. | PostgreSQL role/ownership/guard-trigger configuration. | Database privileges and owner-only guards fail even if application SQL regresses. | Deployment checks exercise forbidden statements under the real app role. Rust API privacy is additional defense, not the database authority. |
| `P-PG-05` | `store_scope_id` and `store_epoch` are singleton immutable lineage identity after first admission; destructive reset creates a never-reused scope and fresh epoch, while only verified continuation preserves them. | PostgreSQL store bootstrap/reset authority. | Immutable singleton rows, schema validation, and controlled reset/migration tooling. | Admission, journal/fact/effect identity, replay, and deployment inventory compare the retained identity. No app caller can rotate or preserve it by assertion. |
| `P-PG-06` | Exactly one non-rollback writable database/WAL lineage may use a store scope/epoch; promotion permanently fences old/sibling writers and contains every published run suffix and tenant fact head. | External HA/WAL consensus and generation-fencing authority. | Deployment promotion/restore procedure supplies the irreducible fence before the store opens for authority-bearing work. | Store startup fails closed on missing proof or possible rollback. Split-brain/stale-primary/restore tests validate the deployment mechanism. Worker or executor fences cannot substitute. |
| `P-PG-07` | Every live authority-bearing read and all writes use the fenced authoritative writer; a replica, apparent catch-up, or connection option cannot construct store-backed authority in recoverability v2. | Deployment/store connection authority qualified by `P-PG-06`. | App/store assembly exposes only the writer pool for admission, journal/object/fact reads, status, replay, trace/audit/export, resume, and writes. | Tests reject replica-configured authority. Offline verification remains `P-ACL-04`; a future replica contract would need exact lineage plus applied barrier/publication proof. |
| `P-PG-08` | Failed or acknowledgement-ambiguous PostgreSQL commit returns `OutcomeUnknown`, never positive append/admission/access authority; reconciliation uses the original append identity and immutable candidate. | PostgreSQL store result classifier. | Only a directly observed successful commit maps to `NewlyAppended`; connection ambiguity discards transient authority. | Store prototype/parity tests reconcile by `append_request_id`, enforce same-content identity, and prove no second authorization permit or fact coordinate is minted. |

## Boundaries that are deliberately verified more than once

These pairs are not duplicated authority because they own different predicates or because the
second boundary can only reject an exact-bound claim:

| Boundary pair | Authority split |
| --- | --- |
| State callback and store | The callback owns domain request/verdict/settlement (`P-AR-06`, `P-AR-09`); the store owns exact structural binding and atomic append (`P-ST-01`–`P-ST-09`). The store cannot run another reducer. |
| Typed builder and certifier | The builder excludes invalid authored combinations (`P-TC-01`, `P-TC-03`); the certifier independently validates hostile retained program bytes and owns certified expansion/totality (`P-TC-04`, `P-TC-05`). |
| Certifier and qualified program registry | Certification selects and qualifies component/binding identities (`P-TC-06`, `P-TC-08`); runtime may only require exact admitted equality (`P-AR-04`). |
| MFM journal and executor ledger | The journal owns MFM request and settlement truth (`P-EX-03`, `P-ST-08`); the executor owns delivery/resource truth and its backend/deployment split (`P-EX-05`–`P-EX-08`). Neither can settle the other's state. |
| Fact state, reserved capability, and store | The state owns query and interpretation (`P-FA-01`); the capability owns exact scan response (`P-FA-02`); the store owns coordinate/barrier attestation (`P-FA-03`) and the sealed verifier combines retained store authority into the closed completeness claim (`P-FA-04`). |
| App ACL and kernel/store | App policy decides who gets a grant (`P-ACL-01`); kernel/store validate that the sealed grant was not substituted or broadened (`P-ACL-02`). |
| Shared verifier and PostgreSQL constraints | The verifier defines retained structural truth (`P-AR-07`, `P-AR-08`, `P-ST-08`); PostgreSQL constraints and locks make the same truth atomic under hostile rows/concurrency (`P-PG-01`–`P-PG-03`). |
| Store lineage fence and executor resource fence | HA fencing preserves the MFM journal lineage (`P-PG-06`); executor/backend/deployment and destination fencing preserve external delivery/resource ownership (`P-EX-07`, `P-EX-08`). Neither is evidence for the other. |
| Recorded-history verification and exact reproduction | Structural verification constructs authority from retained bytes (`P-RV-01`); reproduction is an optional callback diagnostic over that authority (`P-RV-02`) and cannot repair or upgrade it. |

## Coverage crosswalk

The requested pre-freeze boundaries are closed by the following unique rows:

| Required boundary | Predicate ids |
| --- | --- |
| Hostile bytes and canonical persisted schemas | `P-HB-01`–`P-HB-05` |
| Typed construction and sealed value/authority surfaces | `P-TC-01`, `P-TC-02`, `P-AR-05`, `P-ST-02`, `P-ST-06`, `P-EX-01` |
| Decoder and object reconstruction | `P-HB-01`–`P-HB-03`, `P-PG-01`, `P-RV-01` |
| Certifier, planner, graph, component, and executable identity | `P-TC-03`–`P-TC-09` |
| Admission and runtime | `P-AR-01`–`P-AR-11` |
| Observation eligibility and head-relative structural consumability | `P-AR-07`–`P-AR-09`, `P-ST-07` |
| Sealed store append, hashes, objects, closure, and concurrency | `P-ST-01`–`P-ST-09` |
| Audited read/effect boundary | `P-HB-05`, `P-ST-06`, `P-ST-07`, `P-EX-01`–`P-EX-04` |
| Executor evidence, convergence, resource ownership, and backend/deployment fencing | `P-EX-03`–`P-EX-08` |
| Fact publication, selection, and completeness | `P-FA-01`–`P-FA-04` |
| Generic typed fact-response materialization and object reachability | `P-HB-01`–`P-HB-03`, `P-FA-02`, `P-ACL-03` |
| Bounded, deduplicated portable cross-run source closure | `P-TC-09`, `P-RV-01`, `P-ACL-04` |
| Recorded replay, exact reproduction, candidate comparison, and trace | `P-RV-01`–`P-RV-04` |
| App ACL and purpose-bound access | `P-ACL-01`–`P-ACL-05` |
| PostgreSQL schema, transaction, ACL, identity, and fencing | `P-PG-01`–`P-PG-08` |

The accepted RFC's original ownership families map without remainder:

| RFC predicate-ownership family | Atomic rows here |
| --- | --- |
| Run admission, logical-start/corrective uniqueness, cross-run source authority | `P-AR-01`–`P-AR-03`, `P-TC-09`, `P-ST-03` |
| Executable self-attestation and qualified callback registry | `P-TC-06`, `P-TC-07`, `P-AR-04` |
| Composite planning, typing, applicability, terminal/dependency totality | `P-TC-01`–`P-TC-05` |
| Request authorship, evidence semantics, failure, reduction, outputs, facts | `P-AR-06`, `P-FA-01` |
| Read readiness and frozen intent | `P-AR-07` |
| Observation selection and evidence-gap retry | `P-AR-08`, `P-AR-09` |
| Effect request and settlement | `P-EX-03`, `P-AR-06`, `P-ST-08` |
| Executor delivery frontier, evidence, resource ownership, and deployment fencing | `P-EX-04`–`P-EX-08` |
| Dependency failure, skip, closure, and audit tail | `P-TC-05`, `P-ST-07`, `P-ST-08` |
| Reserved fact query and completeness frontier | `P-FA-01`–`P-FA-04`, `P-PG-03`, `P-PG-07` |
| Access-boundary result | `P-HB-05`, `P-EX-01`, `P-ST-07` |
| Atomic authority and concurrency | `P-ST-01`–`P-ST-09`, `P-PG-02`, `P-PG-03`, `P-PG-08` |

## Retained contract-shaping evidence

These prototypes exercised the ownership cuts that shaped the frozen annex and corpus. They do not
themselves mint production authority, override the frozen schema/golden-vector artifacts, or make
the target the current implementation:

- The retained
  [composite-planner prototype](../crates/kernel/certify/src/tests/composite_planner_prototype.rs)
  covers exact authored shapes, stable paths, framework-outer/executor-inner expansion, effective
  output rewiring, recursive/cyclic expansion rejection, and dependency/terminal totality for
  `P-TC-04` and `P-TC-05`.
- The retained
  [store-contract prototype](../crates/kernel/store/tests/recoverability_store_contract_prototype.rs)
  covers logical admission equality, per-run CAS/idempotency, `OutcomeUnknown`, positive-only
  authorization permits, purpose-bound run access, and fixed closure with a constrained audit tail
  for `P-AR-01`, `P-ST-03`, `P-ST-06`–`P-ST-08`, and `P-ACL-02`.
- The retained
  [in-memory fact-frontier prototype](../crates/kernel/store/tests/commit_contract/tenant_fact_frontier_prototype.rs)
  covers publication/barrier ordering, tenant independence, dense rollback-free coordinates,
  multi-fact publication, zero frontier, structural contention, source verdicts, and
  same-store-versus-portable completeness for `P-FA-02`–`P-FA-04`.
- The retained
  [generic response-materializer and bounded-scan prototype](../crates/kernel/store/tests/commit_contract/fact_response_materializer_prototype.rs)
  resolves typed fact subject/response values through the ordinary retained-object and exact
  object-path authority, preserving authorization, request, frontier, order, schema, and evidence
  bindings. It proves explicit empty and rejection of missing, wrong-schema, tampered,
  wrong-evidence, and unreachable objects for `P-HB-03`, `P-FA-02`, and `P-ACL-03`; there is no
  `FactQueryStore` or fact-only loader in the proof.
  The same prototype scans a representative 1,984-publication/3,968-fact tenant prefix, verifies
  exact selection and omission rejection, and records elapsed time without a timing assertion.
  The private scan session uses 4,096-publication/8,192-fact steps and a 128-item semantic result
  limit. A 4,097-publication vector crosses the publication budget, and a separate
  4,097-publication/12,291-fact vector crosses the fact budget mid-publication. Both reach the same
  final response after `More`; one-step and exact-boundary many-step executions produce identical
  deterministic response bytes, and a publication appended between steps above the fixed frontier
  remains excluded. The proof also keeps the consuming position private, non-cloneable, and
  non-serializable and restarts deterministically from immutable inputs after it is dropped. The
  numbers bound one step, not total validity, so no tenant becomes permanently unqueryable.
  The same prototype builds a 128-source closure with 384 logical object references deduplicated to
  130 verified objects and 62,741 canonical bytes. Its consuming `More`/`Complete` session produces
  identical closure bytes under one-step and many-step budgets, then crosses 256-source,
  512-unique-object, and 256-KiB verification-step budgets without invalidating the total.
  Counts, bytes, and elapsed time are measurement-only. Missing dependencies and objects, cycles,
  wrong bindings, and tampering still reject for `P-TC-09` and `P-RV-01`.
- The retained
  [PostgreSQL fact-frontier prototype](../crates/storages/postgres/src/run_store/tests/tenant_fact_frontier_postgres_prototype.rs)
  supplies real row-lock ordering, cross-tenant independence, dense rollback, hot-tenant
  operation/row counts, non-threshold timing evidence, and writer/replica failure modeling for
  `P-PG-03`, `P-PG-07`, and `P-PG-08`.
- The retained
  [historical-executable isolation prototype](../crates/kernel/replay/tests/historical_executable_isolation.rs)
  separates callback-free verification, exact executable identity, OS-enforced capability denial,
  unavailable artifacts, and non-authoritative candidate comparison for `P-TC-07` and
  `P-RV-01`–`P-RV-03`.
- The retained
  [managed-PostgreSQL executor prototype](../crates/storages/postgres/tests/recoverability_postgres_executor_prototype.rs)
  closes the logical
  [durable reference-executor gate](../RFC_REFACTOR_RECOVERABILITY.md#durable-reference-executor-gate)
  for `P-EX-04`–`P-EX-08`. It covers the closed five-record delivery algebra, immutable
  effect/resource streams, atomic compare-and-swap, positive-only target-entry authority,
  exact-attempt destination receipts including one exact legal post-tombstone observation,
  cumulative reserve for every unmatched attempt plus the tombstone, exact terminal proofs, the
  pre-persistence and authoritative-refold 256 UTF-8-byte effect-identifier bound, two materially
  different typed resource policies, strict refolded restore, modeled generation races, bounded
  hostile decoding, and secret-free retained surfaces. At the identifier boundary the retained
  accounting yields a worst-case 1,041-byte ordinal-63 observation and 1,331-byte tombstone within
  each 16,384-byte completion slot. Memory and file prototypes remain supporting conformance
  evidence only. This logical closure does not claim physical non-rollback fencing, WAL/backup
  lineage, real commit ambiguity, stale/sibling exclusion across failure domains,
  destination-owner qualification, or production promotion/failover; those are rollout evidence
  obligations under `P-EX-08`.

## Frozen-schema rule

At schema freeze, every candidate field, constructor, decoder, verification pass, database
constraint, and vector named the `P-*` row it enforces. During implementation, a proposed check
with no row is either:

1. a missing predicate that requires a deliberate new contract version rather than a silent
   change to recoverability v2;
2. non-authoritative defense or telemetry that must be labeled as such; or
3. a duplicate authority path that must be deleted.

Every row has positive and hostile/negative vector obligations at its unavoidable trust boundary
in the frozen corpus; implementation consumers must preserve and execute that coverage. Temporal
claims that retained bytes cannot rederive—currently the fact-barrier freshness attestation in
`P-FA-03`/`P-PG-03` and deployment writer lineage in `P-PG-06`—remain explicitly qualified
attestations. They must not be relabeled as portable proof.

## Material uncertainties

None. The dedicated bounds architecture review selected private affine, deterministic `More` /
`Complete` sessions and prohibited total-prefix or total-closure validity ceilings. Step budgets
are operational work limits whose only permitted effect is the number of private steps; they are
not persisted semantic limits or public continuation positions. Lost scan state requires the
ordinary unmatched-authorization retry path and a fresh scan; lost portable-verifier scratch
restarts from immutable inputs.

The rollout-only privacy, capacity, long-term executable-retention, and deployment-specific HA
qualification gates remain classified in
[`recoverability-cutover-gates-v2.md`](recoverability-cutover-gates-v2.md); they constrain
deployment but do not authorize an alternate predicate owner.
