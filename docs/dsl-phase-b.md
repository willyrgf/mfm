# DSL Phase B production cutover

**Verdict: the amended Phase B design passes within the agreed scope.** Production migration,
managed acceptance and final composed CI are complete. B2 is `472f950a3c42e3a9f503bf10e92b5cd454dd21aa`;
combined-service isolation is `4defb1299a79cc12c5de7bf8f568072eeb03c74f`, the exact clean candidate
that passed final CI. Subsequent changes only record evidence and correct two Markdown descriptions;
no Rust, schemas, tests, manifests or task definitions changed after that passing run.

The source baseline is `660285d0`; isolated proof `c4d97c9d` includes scalar-route enforcement
`d7a42c78`. [dsl-phase-a.md](dsl-phase-a.md) remains the historical proof record. The chronological
checkpoints below describe what was incomplete at each stage, not outstanding migration work or
parallel supported designs. Final commands, results and LOC follow the historical ledger.

Work is committed in `refactor/dsl-phase-b`, in the sibling `mfm2-phase-b` worktree. The proof changes
were applied relative to `b49c90b8`, excluding its older AGENTS and RFC files. The updated RFC and
repository rules remain authoritative. B1 is omitted: native interfaces, products, resource binding,
consumers, schemas, tests and deletions form one inseparable B2 cutover. No production compatibility
API or consumer-disable workaround is an acceptable intermediate committed design.

## Implemented contract and evidence requirements

| RFC adjustment | Implementation and required evidence |
| --- | --- |
| Network boundary | Native client admits Portfolio configuration, renders semantic results and reconstructs publication from retained public route descriptors; second native implementation uses the same Portfolio Pure States. |
| Pending reconciliation | Native receipt/known/exact rebroadcast ordering; private awaited one-second pacing; paused-clock cancellation, known/receipt race, unchanged provider errors and real delayed settlement. |
| Codec phases | Move CallbackFailure to Capabilities; native nested decode/encode scopes retain phase and causes; Runtime supplies invocation provenance. Existing original-custody, panic and acknowledgement tests remain mandatory. |
| Complete Program bound | Complete ProgramWire uses bounded serialization; aggregate-binding overflow short-circuits, ordinary serializer errors retain causes, cold byte length rejects before parsing, nested Objects retain identity. |
| Resources and inspection | EvmResources implements existing bind/discovery contracts; remove Application registration, composed runtime, duplicate caches and manual State/Operation tables. |
| Product failures | Native owner declarations derive exact native decoder dispatch; native client projects both continuations via checked Portfolio context/ordinal validation. Reject forged ABI/context instead of fabricating domain failures. |
| Construction diagnostics | Selection, association and checkpoint rejection retain reviewed identities, positions and reason through existing diagnostics. |
| Operation construction/depth | From<Definition> retains defaults without requiring Default; one guard precedes planning/injection; 16/17, sibling and mixed native-support boundaries. |

## Historical verification ledger

All commands below run from the Phase B worktree through the pinned default Nix shell. The caller
supplied the CARGO_TARGET_DIR environment value shown below, but the pinned shell explicitly unsets
it (`flake.nix`); these executions therefore used Phase B's local `target/`. No proof source or build
artifact was modified by that attempted cache selection:

```sh
export CARGO_TARGET_DIR=/home/willyrgf.linux/dev/mfm2-phase-a/target
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-program --test recovery_scopes --test phase_a_source --test native_construction
nix develop -c cargo test -p mfm-program --test operation_depth
nix develop -c cargo test -p mfm-program --lib
nix develop -c cargo check -p mfm-program -p mfm-runtime --lib
```

The first focused test command passed 10 tests (including four compile-fail cases inside the source
test). Depth passed one boundary test covering pure and mixed native nesting. Program library
passed six tests, including four complete-document serializer/admission regressions. Library check
passed after moving CallbackFailure inward. These results do not establish migrated production,
managed acceptance or final CI. Exact final revisions and final commands/results will replace this
provisional ledger when the complete candidate exists.

Additional completed focused commands (the later commands omit the ineffective caller-side cache
variable):

```sh
nix develop -c cargo test -p mfm-capabilities --lib
TRYBUILD=overwrite nix develop -c cargo test -p mfm-program --test authoring_boundaries
nix develop -c cargo test -p mfm-program --all-targets
nix develop -c cargo test -p mfm-program --test native_callbacks --test native_construction
nix develop -c cargo test -p mfm-program --test native_callbacks
nix develop -c cargo check -p mfm-evm --lib --message-format short
nix develop -c cargo test -p mfm-evm --test scalar_read_evidence --test balance_construction --test native_preparation --message-format short
nix develop -c cargo test -p mfm-runtime --lib
nix develop -c cargo clippy -p mfm-program -p mfm-capabilities --all-targets -- -D warnings
nix develop -c cargo clippy -p mfm-evm --lib -- -D warnings
nix develop -c cargo test -p mfm-capabilities -p mfm-program --all-targets
nix develop -c cargo test -p mfm-program --test native_construction --test recovery_scopes
```

Capabilities initially passed 3 tests; nested codec-to-construction/State diagnostic custody adds a
fourth passing test, retaining the nested phase, cause fields and size. The first full Program suite passed 25 tests plus its embedded UI
cases; the added nested-native-phase regression subsequently brings Program to 26. Native callback
translation and projection preserve Decode/Encode errors and panic phases; uncaught hook panics
remain Execute. Focused EVM passed 3 tests (cold balance route mismatch, native preparation, scalar
outcomes including malformed nested native Objects). Runtime passed 23 library tests, retaining
original encoding, acknowledgement, cancellation, pending authority and cold recovery coverage.
Listed Clippy commands passed. After replacing bare selection/association/checkpoint rejections,
focused construction and recovery passed 5 tests asserting the retained reasons, identities and
positions. These are intermediate results, not the final exact-candidate verification.

The initial full Program attempt failed stale mutable-authoring UI diagnostics; those fixtures were
replaced with current API boundaries and reviewed expected diagnostics, then rerun successfully.
Native hook return-type changes initially exposed diagnostic assertions in EVM tests; they now
assert Decode for malformed native codecs and Execute for route/semantic mismatches. The checkpoint
test initially expected bare InvalidContract; it now checks the specific reason and positions.

## Coverage replacement ledger

Old mutable OperationExpansion tests cannot be compiled against the sole typed-source API. Their
retained guarantees move to the following current boundary tests, without retaining a second API:

| Removed or superseded fixture | Current coverage / remaining requirement |
| --- | --- |
| Old root expansion/input commitment and superseded Program descriptors | `program/src/tests.rs::current_program_commits_input_and_rejects_retired_or_forged_contracts`; typed planning tests in `phase_a_source`. |
| Mutable nested policy/checkpoint fixtures | `recovery_scopes`: inherited versus replaced target units, foreign/forward marker rejection, exact explicit non-default handler parameters and cold identity. |
| Old capability operational-error identity and injected expansion | `native_construction`: exact selected native ABI, resolved supporting Read, selected-only binding, cold loading without Resolve, forged mode/ABI rejection. |
| Old dynamic depth fixture | `operation_depth`: 16 accepted, 17 rejected before plan, siblings unwind, mixed native/support scopes share the guard; rejection yields no Program or provider call. The sealed typed source cannot catch and merge a private mutable draft. |
| Standard recovery phase/classification table | Retained unchanged in Program unit tests. |
| Old scoped mutable-authoring UI fixtures | `authoring_boundaries` now checks private Draft, sealed AuthoringSource and unforgeable Program/StateDeclaration. The stale diagnostic mismatch was observed, replaced and regenerated through pinned trybuild. |
| All Runtime/Store/custody/managed tests | Not waived. Production migration and retained-suite verification remain outstanding. |

## Native boundary implementation checkpoint

Chain now owns `BalanceExecutionConfig` (independent expected route reference plus opaque native
Object) and `BalanceSourceDefinition<K>` (source, ordinal, scale, execution). EVM owns the exact
`EvmBalanceRoute` with chain ID and checked public endpoint name, and qualifies the derived physical
route/reference against the caller expectation. Native binding resolution accepts the shared source
facts and checks ledger and asset agreement. The current Portfolio planner and old EVM source
definition still require the coupled production migration and deletion; their temporary presence in
the uncommitted worktree is not a second supported API or completed cutover.

```sh
nix develop -c cargo test -p mfm-evm --test balance_route --message-format short
```

Passed 1 regression: retained-descriptor cold decoding reconstructs the public endpoint name,
matching source facts produce the expected native binding, a different same-chain endpoint is
Execute rejection, and a wrong native descriptor schema is Decode rejection. This is descriptor
qualification evidence, not yet configuration-deleted product publication acceptance.

## Complexity and LOC

Removed duplicate policy-scope depth accounting and unbounded complete-document serialization.
Reused the existing bounded serializer and callback failure representation. Necessary additions are
boundary regressions and synchronous native codec panic scopes. Production/test/docs LOC and exact
commit references must be computed on the final coherent candidate; interim counts are not the
final cutover accounting.

## Material uncertainties

None remain for the agreed Phase B implementation boundary: configuration-deleted publication,
State-derived native failure dispatch and resource binding without Application caches have executable
cold acceptance. Final composed CI passed on the exact candidate identified above. The
accepted downstream source-publication limitation remains explicit in [known-gaps.md](known-gaps.md).
Managed Reth settlement and a surviving ephemeral signer do not establish production finality or
host-process key recovery; those remain outside this cutover.

## Production boundary checkpoint (uncommitted B2)

Portfolio's production library now has no EVM dependency. Native configuration DTOs moved to
`mfm-evm-live::client::portfolio`; Portfolio retains semantic demands, per-source execution
Objects, confirmed balances and checked projections. Enrichment filters source/descriptor pairs
together. Its native client reconstructs configuration and renders native views without live handles.
Chain's complete-confirmation check reuses its existing prefix validator.

`EvmResources<Sources>` now implements the existing selection/binding/environment contracts.
Its checked records own route/provider or binding/signer/authority/provider correspondence;
public views derive from those records. The native registration functions and assembly module are
removed. Application's BoundCapabilitySet, ComposedRuntime, duplicate target/view caches and
handwritten component table are removed. Application compiles the actual maintained sources and
loads retained Programs before Runtime read/resume; native configuration admission replaces its
wire/planning duplication. Cold publication no longer searches live bindings for endpoint names.

Compiler discovery supplies sorted, deduplicated owner metadata without Plan or resources, and
rejects conflicting metadata with both claims retained. Transaction reconciliation checks receipt,
then transaction presence, and submits retained bytes only when absent. Every Pending return awaits
one private second inside the adapter. Known-transaction provider failures remain unchanged.
Settlement qualification moved to the native settlement owner and is reused by adapter/projector.
The obsolete adapter-side reservation check duplicated the retained-reservation identity check and
was removed with its superseded capability call.

Additional pinned commands/results at this intermediate working revision:

```sh
nix develop -c cargo check -p mfm-portfolio --lib --message-format short
nix develop -c cargo check -p mfm-evm-live --lib --message-format short
nix develop -c cargo check -p mfm-app --lib --message-format short
nix develop -c cargo test -p mfm-program --test components --message-format short
nix develop -c cargo test -p mfm-chain --all-targets --message-format short
nix develop -c cargo test -p mfm-evm-live --lib --no-run --message-format short
nix develop -c cargo test -p mfm-evm-live --lib --message-format short
nix develop -c cargo test -p mfm-evm-live --lib both_continuations --message-format short
nix develop -c cargo clippy -p mfm-evm-live -p mfm-app --lib --message-format short -- -D warnings
nix develop -c cargo test -p mfm-evm --test provider_failure_contract --test scalar_read_evidence --test balance_route --message-format short
```

The three library checks passed. Component discovery passed 2 tests; Chain passed 24 tests.
Live library tests first exposed obsolete registration fixtures, which were migrated to real
ResolvedEffect/ResolvedRead plus compile/load/Runtime. The first executable run passed 39 and failed
12 on one shared fixture's retired Read intent shape. Replacing that fixture with typed current
intents preserved the transport assertions. The updated suite passed 54 tests in 17.63 seconds,
including retained-wire cold recovery, cancellation, custody acknowledgement loss, concurrent signer
winner, wrong signatures, provider ancestry and transaction-known pacing/error tests. This does not
replace the still-outstanding Journal-boundary and managed integration suites.

The separate native-client regression passed in 98.15 seconds. Both actual Portfolio continuations
compiled without IO, rejected a different same-ledger cold endpoint, cold-executed after dropping
source configuration, and made exactly nine expected native observations. Publication after dropping
Runtime/resources reconstructed the snapshot config field-for-field; snapshot rendering preserved
native amounts 42 and 84. These amounts are not evidence for the separate contract lifecycle 42/84
acceptance, which remains required.

Clippy passed after boxing the new route-error identities rather than inflating every Result.
The frozen diagnostic schema test initially failed on the intended RPC-method/operation changes;
reviewed current identities are EvmOperationalError v3
`40cb1920449b8270da4ef453fc654762a18e7165305cd402e716ef5bfd08fd95` (12756 descriptor bytes) and
EvmTransactionOperationalError v4
`cc11620d29b5503c3bdea1a162847b3522d71d64fd26228c0825395f14739985` (14359 bytes).
The focused EVM command then passed 6 tests, including scalar unsuccessful evidence and route checks.

The subsequent failure-projection work is not yet covered by these passing results. Native balance
State declarations now derive the closed stage vocabulary and exact decoder dispatch. Native client
functions accept retained declaration/input/original, without Runtime types. Shared closed failure
codes and Portfolio context validation replace arbitrary presentation strings. Application retains
exact reports and requests separate product projections. Full native-stage/shared/actual-Portfolio
failure regressions, forged context/ABI cases and transport verification remain outstanding.

Disk pressure was resolved using the pinned Cargo cache command below (8.8 GiB removed); no Phase A
source or commit changed. A direct force-remove command was automatically rejected before execution;
Cargo's targeted cache cleanup succeeded.

```sh
nix develop -c cargo clean --target-dir /home/willyrgf.linux/dev/mfm2-phase-a/target
```

### Consumer migration checkpoint (uncommitted B2)

- `nix develop -c cargo test -p mfm-portfolio --lib --message-format short`: 4 passed (1.10s). Native fixture construction/rendering moved out; semantic maximum fields, enrichment membership/coverage, and substituted collection handoff remain covered.
- `nix develop -c cargo test -p mfm-app --lib --message-format short`: 12 passed, 1 stale component inventory assertion failed (18.01s). The maximum candidate publication test now constructs semantic enrichment via maintained States and reconstructs the native document, including provenance, under the existing document ceiling. The inventory fixture has been updated to the discovered current source set; rerun outstanding.
- `nix develop -c cargo test -p mfm-app --test use_cases --message-format short`: initial migrated suite 5 passed, 6 stale assertions failed (194.00s). Interrupted Reads, configuration deletion, ambiguous start/progress, and incomplete/forged/lost-publication acknowledgement passed. Subsequent focused checks below replace the stale assertions; this is not a passing full-suite claim.
- `nix develop -c cargo test -p mfm-app --test use_cases snapshot_ --message-format short`: 3 passed, 1 provider fixture mismatch failed (43.49s). Exact native snapshot rendering, durable operational error with no fabricated product failure, and interrupted progression passed. The scripted provider now distinguishes native account/asset facts instead of removed shared source names; token rerun outstanding.
- `nix develop -c cargo test -p mfm-app --test use_cases config_rejects_ --message-format short`: 1 passed (0.73s), covering native admission rejection, missing construction bindings before append, and forged retained configuration.
- `nix develop -c cargo test -p mfm-app --test use_cases shipping_metadata_constructor --message-format short`: 1 passed (129.51s). Malformed stored shared metadata preserves the JSON/parser rejection through cold observation and execution; forged identity/digest/slot cases remain rejected. This replaces the obsolete EVM constructor-specific ancestry assertion with the RFC's stored-data parser contract.
- `nix develop -c cargo test -p mfm-evm-live --test portfolio_runtime --message-format short`: 4 passed (102.54s). The former Portfolio EVM planning/runtime suite now belongs to the native client crate. It retains route-sensitive identity/admission, both continuations, terminal cold inspection without IO, cancellation before final confirmation with no balance reread, and second-collection exact failure/context preservation.
- `nix develop -c cargo check -p mfm -p mfm-rest-api --bins --message-format short`: passed. Both production binaries consume derived inspection/resource results; construction diagnostics retain run identity and cause in transport rendering. Remaining binary tests and managed acceptance are still outstanding.

Earlier native product projection check: `nix develop -c cargo test -p mfm-evm-live --lib real_native_failures --message-format short`: 1 passed (61.99s), covering both continuations, a token-decimals rejection after a confirmed source, exact native original/report retention, wrong-continuation rejection, and cold product failure presentation without source configuration or additional provider IO. This does not yet establish the complete stage/forgery matrix.

- `nix develop -c cargo test -p mfm --bin mfm_cli reporting --message-format short`: 9 passed (0.01s). The retired synthetic Operation/assembly fixture is replaced by the existing typed `Identity<NoParams>` source. All stdout/stderr, serializer, source-chain, append-recording and acknowledged-head assertions remain.
- `nix develop -c cargo check -p mfm-rest-api --all-targets --message-format short`: passed; `nix develop -c cargo test -p mfm-rest-api --bin mfm_rest_api --message-format short`: 4 passed (60.52s), including router transport/recovery and original reporting failures.
- `nix develop -c cargo test -p mfm-evm --test balance_construction --message-format short`: 1 passed (9.13s). Migrated the source to shared `BalanceSourceDefinition` and deleted `EvmBalanceSourceDefinition` plus its duplicate direct-binding resolver. Retained designated-route rejection through cold executable adapter invocation before provider IO.
- `nix develop -c cargo test -p mfm-evm-live --test balance_recovery --message-format short`: 1 passed (20.39s), replacing `portfolio/tests/recovery_policy.rs` with an explicit maintained-defaults construction around the shipping native protocol. Both changed initial anchor and changed confirmation retain the acknowledged prefix/exact native original and restart to one coherent observation point. Scripted IO does not replace managed chain acceptance.
- `nix develop -c cargo test -p mfm-portfolio --test native_boundary --message-format short`: 1 passed (26.59s). An independent native test ABI executes both maintained Portfolio operations, shared preparation/confirmation, semantic candidate filtering, and cold terminal inspection. No EVM crate is used by Portfolio. Its fixed test point and scripted amounts are not production network support or provider authentication.

### Remaining kernel and production consumers

`nix develop -c cargo check -p mfm-runtime --all-targets --message-format short` currently fails in the retained `current_state`, `pending_failure`, and `runtime_contract` integration suites (old assembly, Operation expansion and capability signatures). Earlier Runtime verification in this ledger is **library-only** and does not establish those integration guarantees. Their fault cases must be migrated, with any removed overlap mapped to an executable replacement; none may be disabled to pass the cutover.

Other remaining consumers include the generic transaction integration suite, managed Effect lifecycle/support, and PostgreSQL Runtime integration fixtures. Managed acceptance, full affected checks, exact-candidate CI and coherent B2 commit remain outstanding. The implementation must not be reported as a passing production cutover at this checkpoint.

### Completed migrated boundary checks

- `nix develop -c cargo test -p mfm-app --all-targets --message-format short`: 13 library tests passed (17.61s), 11 use-case tests passed (195.93s). This includes both products, exact-revision/cold configuration deletion, native product rendering, operational/domain distinction, token-decimals context, discovery, substitution rejection, interrupted progression, ambiguous append and publication acknowledgement, provenance forgeries, and parser/identity/slot cause preservation. The maximum publication fixture's native point was subsequently changed from the old anchor DTO to the actual `EvmBlockPoint`; rerun that focused test before final acceptance.
- `nix develop -c cargo test -p mfm-evm-live --lib real_native_failures --message-format short`: expanded regression passed (85.16s). Both continuations now fail after a completed collection **and** source; projection rejects forged ordinal/correlation/route/request/source and wrong input/original/declaration contracts. Exact original, report, terminal head and product failure survive cold configuration-free presentation with no extra provider call. Full native/shared stage coverage remains outstanding.
- `nix develop -c cargo test -p mfm-evm --lib --test balance_stages --test balance_binding --message-format short`: 3 library, 1 stage, 2 binding tests passed (1.28s/1.71s/0.19s). Ported full-width/mixed native-token/large caller confirmation and consolidation. Removed redundant old balance-only unit fixtures: unsuccessful evidence is now checked by `native_and_token_preparation_keep_typed_context_and_confirmation_precedes_amount_admission`; source/ordinal/scale/route substitution by `supporting_translation_checks_retained_source_qualification_against_each_selected_binding`; descriptor-only assertions by their actual canonical roundtrips and native construction tests. Product handoff substitution remains independently covered in Portfolio.
- `nix develop -c cargo test -p mfm-evm --test transaction_contract --message-format short`: 7 passed (0.13s). Replaced retired cumulative native phase wrappers with `native_preparation_and_settlement_reject_hostile_combinations`, retaining exact command/reservation/preparation roundtrip and wrong Effect/nonce/hash/action rejection. Shared applied-result/original custody belongs to `native_preparation`, `lifecycle_runtime` and Live transaction recovery; their complete managed counterparts are still being migrated.

The new authoring guide links the actual compiler/capability/lifecycle/native client owners and consuming tests. Portfolio's unused native/legacy error variants and the corresponding forwarding branch have been deleted. Shared semantic failures now expose the closed public code projection at their own owner instead of duplicating that mapping in the native client.

- `nix develop -c cargo test -p mfm-evm --all-targets --message-format short`: 34 passed; 2 existing managed-solc cases ignored by their declared setup contract. The concurrently completed focused scalar check below is the explicit result for the final changed substitution fixture.
- `nix develop -c cargo test -p mfm-evm --test anchored_call_contract --test scalar_read_evidence --message-format short`: native wire/bounds tests passed (3); the new scalar cross-intent fixture initially reused the original route identity and correctly did not reject that identical intent. Fixed the fixture to use a genuinely different route. `nix develop -c cargo test -p mfm-evm --test scalar_read_evidence --message-format short`: 1 passed (0.31s).
- Retired anchored workflow/slot/plan fixtures are replaced by scalar native projection (`scalar_projection_preserves_all_outcomes_and_rejects_mismatched_or_malformed_evidence`), including route/block/target/calldata substitution and exact unsuccessful originals. The native anchored wire/closed-shape/calldata-return bounds remain in `anchored_call_contract`; shared lifecycle contexts and applied facts remain in `lifecycle_runtime` and shared Chain tests. Managed lifecycle caller migration remains outstanding.

Disk maintenance: `nix develop -c cargo clean -p mfm-app -p mfm-evm-live -p mfm -p mfm-rest-api` removes only selected Phase B package build artifacts. Source, histories and other worktrees remain unchanged. Subsequent verification recompiles these packages as needed.

### Pending-failure cutover and shared text presentation (uncommitted B2)

The eight `pending_failure` regressions now use explicit native implementation/adapter contracts,
compiled Programs, and cold `load`; the registration builder and expansion/failure-map setup are
removed from that suite. The identity native codec and scripted adapter are test-owned boundaries,
not a production registry. Existing assertions remain for retry exhaustion, explicit settlement
after Stop, acknowledged original before policy, protocol-qualified nonacceptance, phase denials,
ambiguous failure append, cancellation on both sides of append, hostile current records, and
competing exact-head candidates. Removed `RecoveryOutcome::Stop.root` construction matches the
current no-failure-map design; the hostile extra-root input still must be rejected.

- `nix develop -c cargo check -p mfm-runtime --test pending_failure --message-format short`: first exposed four obsolete `Stop.root` constructions; those were removed.
- `nix develop -c cargo test -p mfm-runtime --test pending_failure --message-format short`: 8 passed (0.25s).
- `nix develop -c cargo clippy -p mfm-runtime --test pending_failure --message-format short -- -D warnings`: passed.
- `nix develop -c cargo check -p mfm-storage-postgres --all-targets --message-format short`: passed (5.66s). The closed-Store diagnostic regression now uses the config-free `program_document` bootstrap, preserving the same exact Store cause and no-observation assertions. Database acceptance has not run. An initial command used the nonexistent package name `mfm-postgres` and was corrected.
- `nix develop -c cargo clippy -p mfm-portfolio -p mfm-evm --all-targets --message-format short -- -D warnings`: passed (5.65s).
- `nix develop -c cargo test -p mfm-app --lib maximum_candidate --message-format short`: 1 passed (2.79s), including the actual native `EvmBlockPoint` fixture correction.

The Runtime classification compile-fail witness now selects a native operational error without
`ClassifyError` through both fresh compilation and cold discovery. The standalone native protocol
remains reusable. The reviewed pinned diagnostics are E0277 at precisely `compile` and `load`, not
missing retired APIs. `TRYBUILD=overwrite` generated the expected snapshot; normal verification is
required after formatting. The private Runtime/RunView authority witness remained unchanged.

CLI text now consumes the same prepared Application model as JSON and borrows each raw JSON field.
This removes the second handwritten state renderer and makes `product`/`product_failure` visible
without native decoding in the binary. The exact terminal value uses the shared field name `value`;
pending/recovery fields retain their shared nested JSON shapes. Reporting failures retain their
causes and known head through the existing final-report path.

- `nix develop -c cargo test -p mfm --bin mfm_cli reporting --message-format short`: 9 passed (0.01s), including write/flush/serializer failures, source custody and exact terminal value.

The earlier package cache cleanup removed 6.0 GiB. No source or retained repository history was
removed. `current_state`, `runtime_contract`, generic/native managed lifecycle consumers, managed
acceptance and final CI remain outstanding. The new all-observation-stage product regression is
running; no result is claimed yet. Native/shared arithmetic and genuine Portfolio failure projection
still need dedicated coverage.

### Migrated original/codec/capacity evidence (uncommitted B2)

- `nix develop -c cargo test -p mfm-runtime --test terminal_report --test private_boundaries --message-format short`: terminal original/cold inspection/extra-root rejection passed (1, 0.03s); both normal compile-fail snapshots passed (2, 0.26s). `current_state/terminal.rs` and its root mapper were deleted. Its original preservation, no repeated evaluation and malformed-root assertions are retained in `terminal_report`; retrying an in-Runtime root mapper is intentionally removed with that API. Actual checked presentation and projection rejection are covered by native client/Portfolio tests, with report/head preservation outside Runtime.
- `nix develop -c cargo test -p mfm-runtime --test native_construction --message-format short`: 1 passed (0.03s). Moved `current_state/constructors.rs` to an independent public-API target and removed its wrapper Operation/registration setup. Below/above constructor causes, Decode stage, admission preservation, cold inspection and repeated rejection without evaluation remain.
- `nix develop -c cargo test -p mfm-runtime --test pending_failure capacity --message-format short`: 2 passed (0.04s). Both former `current_state/capacity.rs` cases now reuse the pending suite's native protocol. The initial direct migration passed in 8.48s; it was then changed to supply a boundary-valued physical snapshot instead of allocating 65,536 frames, as RFC §13 requires. It still checks the real Runtime pre-append 65,537/65,536 comparison, exact size disposition, no Store append/probe, unchanged head, durable original and retained EffectId with no interpretation. This fixture does not claim to exercise MemoryStore's physical count enforcement; that remains Store-owned coverage. The smaller physical-Store rejection cases in `engine/tests/capacity.rs` remain independent.
- `nix develop -c cargo test -p mfm-runtime --test callback_phases --message-format short`: 4 passed (1.39s). Moved the former `current_state/callback_phases` suite into a standalone target with typed native implementations and explicit binders. All existing Pure/Read/Effect preparation, codec, adapter construction/poll, binding, interpretation, first-original encoding, append-before-classification and classifier-failure assertions remain. Native command decoding now rejects at Effect preparation before retention, so those two expected operation/head assertions were deliberately updated from adapter/sequence 2 to prepare/sequence 1. Added nested native Decode/Encode rejection and panic in both translation and evidence projection, checking exact phase, supplied nested cause, no panic payload, no IO before successful preparation and no extra append. Cold load/read retains the acknowledged prefix.
- `nix develop -c cargo clippy -p mfm-runtime --test pending_failure --test terminal_report --test native_construction --test callback_phases --test private_boundaries --message-format short -- -D warnings`: passed (0.35s), before the final boundary-snapshot simplification; rerun the capacity target after formatting.

Authoritative design/architecture and REST/CLI docs are being cut over to Program-owned construction,
native resources and checked product projection. Remaining stale Phase A migration paragraphs and
retired consumer references require a final audit. No B2 commit or production acceptance verdict is
claimed while those consumers and managed checks remain outstanding.

- `nix develop -c cargo test -p mfm-evm-live --lib every_observation_stage --message-format short`: 1 matrix test passed (986.97s), covering eleven scenarios for **each** Portfolio continuation. A completed earlier collection and source precede rejection at chain identity, token anchor, token decimals, shared token observation, confirmation, native anchor and shared native observation; additional cases cover SafeFailure, IntegrityBlocked, wrong chain and changed confirmation anchor. The Program is compiled once per continuation, source configuration is dropped, and each case cold-loads with fresh explicit native resources before execution and again for terminal inspection. Exact native/shared original contract, public ordinal/code, full report, head and provider count are asserted. This is scripted evidence through shipping clients/operations/Runtime, not managed-provider authentication or settlement acceptance. Native/shared arithmetic and genuine Portfolio failure projection remain outstanding.
- `nix develop -c cargo test -p mfm --all-targets --message-format short`: 12 passed (17.71s), including the shared text-model cutover and current derived component inventory.
- `nix develop -c cargo clippy -p mfm-runtime --test pending_failure --test callback_phases -p mfm --bin mfm_cli --message-format short -- -D warnings`: passed (2.08s), including the final cheap capacity snapshot and nested native codec cases.

No preserved assertion was removed to obtain these passes. The mapper-specific terminal transition
was replaced with exact-original retention and external checked projection as specified by the RFC;
all other moved assertions remain at their current owners. Remaining migration work is concentrated
in the older `current_state`/`runtime_contract` fixtures and generic/managed EVM transaction consumers.

### Current-state migration and managed PostgreSQL preparation (uncommitted B2)

- `nix develop -c cargo test -p mfm-runtime --test current_state --test original_encoding --message-format short`: 11 current-state tests passed (0.24s), 4 original-encoding/size-projection tests passed (0.01s). The latter replaces `current_state/sizes.rs` without its wrapper Operation/registration setup; panic-on-first-original encoding remains single-shot, known position/contract and unavailable original facts remain, forbidden fixture text remains absent, and secondary reload size causes cannot replace the primary append disposition.
- `nix develop -c cargo clippy -p mfm-runtime --test current_state --test original_encoding --message-format short -- -D warnings`: passed.
- Current-state coverage retains exact original before failed policy, recording custody without speculative probes, settlement before interpretation without resubmission, checkpoint restoration/pruning/usage, collision presence/exclusion/reload failure, local-current validation, counter-sum bounds and code-free bootstrap/parser/identity failures. Native callbacks/binders are now explicit; cold paths load the retained Program. The first checkpoint fixture migration correctly rejected forward/out-of-scope targets. It now keeps markers and restart policy in the owning scope and explicitly clears recovery around the preceding Pure States, matching their former Stop behavior. No compiler rule was weakened.
- A different complete Program during repeated admission now asserts the existing structured `restore_frames` Program-reference rejection, including both expected and actual refs and no observation/provider re-entry, replacing the obsolete unit-only AdmissionConflict expectation.
- `nix develop -c cargo clippy -p mfm-evm-live --lib --message-format short -- -D warnings`: passed (4.27s).

All intended working files were staged so Nix can include added modules. This is not a B2 commit or
coherence claim: `runtime_contract` and generic/managed EVM transaction consumers remain to migrate.
`nix develop -c cargo clean -p mfm-runtime -p mfm-app -p mfm-evm-live -p mfm` removed 3.3 GiB of
rebuildable Phase B package artifacts before managed verification. Source/history remain unchanged.
`nix run .#run -- --task postgres-test` is running; its result is not yet claimed.

Managed PostgreSQL result: `nix run .#run -- --task postgres-test` passed (task 33.46s,
run 34.78s) on staged source tree `efa4ccef76698aeaa3e5f0729f4e65285891547a` based on
`660285d0` (tree object, not a B2 commit). The isolated `managed_postgres_rejects_pgoptions` test
passed, followed by all 12 PostgreSQL tests (7.85s), including the managed persistence/authority
contract and actual SQL diagnostic cases. Run evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-2542071-1789842065588572445/artifacts/run-summary.json`;
logs are in that run's `logs/` directory. This closes the focused managed DB lane for the current
source; managed client/Effect acceptance and the final exact-candidate CI still remain.

### Managed client acceptance and exact codec migration

- `nix run .#run -- --task client-e2e`: **passed**, one managed integration test
  (224.53s; task 226.36s, run 227.52s), staged tree
  `1c7bd3f99d5c71aab8074f41714d0aa85bf1a984`, based on `660285d0`.
  Evidence: `/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-2553445-1789842746298783910/artifacts/run-summary.json`.
  The first attempt timed out at the fixture's five-second wait before provider entry; this now
  matches the existing 130-second HTTP deadline. The second attempt passed success/cold recovery
  and exposed the obsolete `report.cause` assertion. The final run checks the current exact
  `report.failure.read.original`, including every supplied provider field, and absence of an
  operational product-domain projection. No production timeout or failure contract was changed.
- Snapshot acceptance decodes `PortfolioSnapshotOutput`, independently derives its complete
  contract/value references, and checks the exact native product against the established expected
  holdings/report. The obsolete v1 output constants were deleted. The new Portfolio dev dependency
  is an existing workspace owner used only for that typed boundary check.
- `nix develop -c cargo clippy -p mfm-rest-api --test client_execution_e2e --message-format short -- -D warnings`:
  passed (5.06s).
- `nix develop -c cargo test -p mfm-runtime --test exact_contracts --message-format short`:
  passed, three tests (0.07s). `nix develop -c cargo clippy -p mfm-runtime --test exact_contracts --message-format short -- -D warnings`:
  passed after applying the pinned integer API suggestion (0.19s).
  These replace the old Runtime registration collision, generic codec and Pure/zero-State tests.
  Fresh construction exercises conflicting actual source selections; cold discovery exercises
  installed source claims. Exact generic schemas, cold bytes/identities, mutable descriptor audit
  conflicts, failed-construction isolation and zero-State completion are preserved. Ordinary fresh
  compilation does not enumerate unrelated installed source types; that responsibility belongs to
  cold discovery. The fixture was corrected to exercise the appropriate owner rather than changing
  this contract.
- Runtime's remaining protocol fixtures are being migrated to native implementations and explicit
  resource binding. `runtime_contract`, its callback/injection consumers and the Live generic and
  managed transaction consumers remain unfinished; no full Runtime/Live all-targets or final CI
  pass is claimed. The arithmetic/product projection matrix is still running and is not yet evidence.

- `nix develop -c cargo test -p mfm-evm-live --lib arithmetic_and_product_failures --message-format short`:
  **passed**, one five-case matrix (404.78s). Both continuations preserve a native confirmation
  `Scale` rejection and a shared consolidation `Sum` rejection (exact 81/80 digit cause), after a
  completed earlier collection and source. Nine maximum-U256 token observations are the smallest
  sum fixture at the supported two-digit scale increase; no production-size allocation is used.
  A snapshot with fractional native value in an earlier collection and a large whole token value
  reaches an actual Portfolio `ConsolidationFailed` at decimal alignment. Enrichment does not perform
  that aggregate, so the Portfolio-owned case correctly applies only to snapshot. Every case drops
  configuration before execution, cold-loads twice, retains exact original/report/head, validates the
  product ordinal/code, and checks 13 or 53 provider calls with none during inspection. The initial
  fixture omitted enrichment's required source in one collection and was correctly rejected at
  admission; adding its declared native source preserved the existing admission rule.
- The retired recursive mutable-expansion fixture is replaced by
  `mfm-program --test operation_depth`: the one guard is tested at 16/17 active operation scopes and
  14/15 scopes enclosing native support injection, including no binding before rejection and
  independent sibling construction. There is no mutable parent expansion to continue after an
  error in the new API. Runtime injection coverage still executes a five-State native prefix,
  designated Effect and suffix, cold-resumes the retained command, and checks the exact suffix
  original; the removed root-mapper assertion belongs to the deliberately deleted API.

- Runtime protocol migration is complete. `nix develop -c cargo test -p mfm-runtime --test runtime_contract --message-format short`:
  15 passed (0.25s); matching focused Clippy passed (0.39s). Construction correctly rejected the
  first injection fixture's two different native Rust implementations claiming one identity; the
  explicit injected implementation now declares its own identity. No ownership check was relaxed.
- `nix develop -c cargo test -p mfm-runtime --all-targets --message-format short`: **all 74 tests passed**,
  including both normal compile-fail witnesses, callback phases/custody, cancellation, pending
  recovery, checkpoints, ambiguous append, exact retained facts, physical capacity and native
  injection. No old registration, expansion, mutable parent or Runtime root-map references remain
  in the Runtime crate. Native mode identity remains covered; duplicate adapter registration is
  removed with the registry and explicit native resource uniqueness is checked at its current owner.

### Managed lifecycle acceptance and final fixture deletion mapping

`nix run .#run -- --task effect-e2e` **passed** (task 300.37s, run 301.64s), source tree
`b81148d5ee01660d385b4fe5547814869c691d10` based on `660285d0`. The task runs the two EVM
lifecycle tests including the previously ignored proof (149.95s), both scalar recipe tests (0.43s),
and the managed Live test (113.97s). Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-2568915-1789844282726293926/artifacts/run-summary.json`.

The task now provisions a separate pinned Reth service with ten-second interval mining. It reuses
adapter probes/containment and declares all three listeners; the client task retains instant sealing.
The pinned Nixfied `docs/GUIDE.md` and `docs/ADAPTERS.md` were read at input
`6b3a70bcc4458c353c7e84b3e12e18e261547fdd`. `nix run .#model-check` passed with model hash
`b7ef70c32e3b71711892721669fb7db4c3c7d85e9a92185fb26e61bdfbb339cb`. An initial whole-record module
merge caused Nix recursion; explicit service fields referencing individual probe options fixed it.
An earlier path-based metadata query tried to include build artifacts and exhausted temporary disk
space; the Git-filtered query succeeded. No source or history was removed by that query.

Before deleting the old fixture and its registration/slot assertions, coverage is mapped as follows:

| Replaced assertion/API | Executable replacement and owner |
| --- | --- |
| Fixture deployment/configuration/ABI getter/report | Live `evm_contract_effect_recovers_cold_and_accepts_external_nonce_advance` uses maintained `ContractDeploymentLifecycle`, `ConfigureAndObserve` and composed addition. Native recipes/Read decode ABI; checked `ContractDeploymentReport` returns 42 and 84. Its native originals retain nonce/hash/block identity and its exact cold outputs match. |
| Lost reservation acknowledgement, no broadcast before recovery | Same managed test retains actual authority row after injected acknowledgement loss; asserts no signature, submission or pending nonce advance before recovery. |
| Cold prepared command, cancellation, delayed settlement | Same managed test cancels immediately after real broadcast on interval-mining Reth; cold reconstruction uses retained Program/input, with no new signature for that command. Five native transactions have five unique accepted submissions; real absent receipt and known-transaction observations occur. Private `transaction_tests.rs` retains exact-wire, rejecting-signer and ambiguous-append matrices. |
| Wallet follow-up and second supported native context | Same managed test executes standalone `ConfigureAndObserve` from the actual deployed predecessor, after external nonce advance `2 -> 3`; its existing-address call reserves nonce 3 and advances to 4. Composed84 then uses nonces 4/5 and advances to 6. This replaces fixture-owned generic context/slot families with maintained request specializations. |
| Target/recipe selection identity, wrong mode and custom recipe | `native_preparation_keeps_request_and_checks_command_binding_and_implementation`, `native_preparation_and_settlement_reject_hostile_combinations`, managed scalar recipes and lifecycle tests qualify exact request/native command, binding, implementation, action and real predecessor locator. Native implementation ownership conflicts are covered at compiler construction. The retired independently selectable slot/recipe type parameters have no current API. |
| SQL failure, retained epoch, closed signer | Same managed test checks actual SQLSTATE 42501 and complete authority cause through App, cold transport equality, local epoch mismatch without append, and exact closed-owner signer diagnostic/classification. |
| Terminal replay | Same managed test cold reads/resumes both 42 and 84 without changing exact head/output or pending nonce. The managed owner remains alive until the closed-signer scenario; this is not process-restart key recovery. |
| Unused checked plan wrappers and context-slot macro | Direct EIP-1559 command factories, `Eip1559Options`, native request recipes and typed shared predecessors replace all production/fixture uses. `fixed_eip1559_command_has_exact_wire_and_checked_factories`, scalar options tests, native preparation and managed existing-address execution own retained behavior. Wrapper-only roundtrips, slot replacement and layout diagnostics are removed with those APIs. |

`nix develop -c cargo clippy -p mfm-evm-live --test evm_contract_effect_e2e --message-format short -- -D warnings`
passed (4.64s). `nix develop -c cargo test -p mfm-app --lib construction_causes --message-format short`
passed (one matrix, 0.02s): actual unsupported/duplicate selections, missing cold State and forward
checkpoint errors retain exact operation, references, positions and complete causes through App,
with no fabricated invocation/candidate. App lib Clippy passed (1.04s). Runtime all-targets Clippy
also passed (1.08s).

## Final owner checks and documentation cutover

- `nix develop -c cargo test -p mfm-values -p mfm-program-derive -p mfm-evm --all-targets --message-format short`
  passed all selected ordinary tests after deleting unused plan wrappers and the context-slot macro.
  The two solc cases remain ignored in ordinary selection and passed in managed `effect-e2e` above.
- `nix develop -c cargo clippy -p mfm-evm-live -p mfm-app -p mfm-values -p mfm-program-derive -p mfm-evm --all-targets --message-format short -- -D warnings`
  passed (10.37s), including migrated native/resource consumers and Application construction causes.
- `nix develop -c cargo test -p mfm-portfolio --lib --message-format short` passed all four (1.11s),
  and `nix develop -c cargo clippy -p mfm-portfolio --all-targets --message-format short -- -D warnings`
  passed (1.98s) after inlining ordinary scenario setup and deleting `test_support.rs`.
- `nix develop -c cargo fmt --all` and `git diff --check` pass. Current changed-document links
  were checked against the filesystem; historical proposals identify superseded APIs as historical.

The unused native-collection/root-failure JSON fixtures are deleted: shared collection handoff
qualification and native/product failure matrices above replace them. The public snapshot JSON
fixture remains checked by Application. Current EVM routing, Portfolio snapshot/enrichment,
contract freeze, error audit, known limitations and verification docs describe the sole current
ownership and API. The RFC changes only repair current implementation links; no new design decision
is introduced. Managed funding/progression deadlines remain test-driver policy, not Runtime policy.

## Candidate complexity accounting

The pre-commit source tree `965df0b6d7af99af5c9ef1155135e65b70714b8f` was measured with
`python3 docs/dsl-phase-b-loc.py 660285d0 965df0b6d7af99af5c9ef1155135e65b70714b8f`:

| Category | Added | Removed | Net |
| --- | ---: | ---: | ---: |
| Production Rust | 11,615 | 7,715 | +3,900 |
| Test Rust | 17,815 | 8,134 | +9,681 |
| Documentation | 1,789 | 639 | +1,150 |
| Manifests/lock/UI/Nix/evidence scripts | 385 | 239 | +146 |

This includes promotion of the isolated proof. Measured against `c4d97c9d` with the same command,
production is +3,058/-2,209 (**+849**), tests +8,758/-8,184 (**+574**), documentation
+1,193/-582 (+611), and other +203/-238 (-35). Counts include blank/comment lines, explicitly
separate external test modules and inline cfg(test), and disable rename detection. The final
revision accounting will include this evidence text and the final CI result.

Necessary production additions are the shared semantic contracts and typed compiler promoted from
Phase A, native resource qualification, retained public route descriptors, native client admission
and projection, bounded complete Program serialization and phase-preserving native codec hooks.
The Phase B increment replaces EVM-dependent Portfolio and Application interpretation with native
clients and checked semantic handoffs, rather than adding a parallel API. It deletes Runtime
assembly/registration/root maps, Application composition and binding caches, handwritten component
inventory, old mutable authoring/depth scope, checked-plan wrappers, context slots and fixture-owned
transaction workflow. New tests cover actual cold product failure projection, both continuations,
independent routes, native delayed settlement and preserved diagnostic/authority boundaries.

## Combined-service startup correction

B2 implementation commit: `472f950a3c42e3a9f503bf10e92b5cd454dd21aa`.
The first `nix run .#ci` on that exact revision stopped before any Rust task (4.37s): pinned Reth
binds a peer listener even in dev mode, contrary to the imported adapter's three-listener assumption.
Starting both managed nodes exposed `0.0.0.0:30303` already in use. Smallest reproducer: simultaneous
instant and delayed node startup, as in CI's required service closure. Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-2588688-1789845420122954767/artifacts/run-summary.json`.

The adopter now models a fourth loopback endpoint for each node, disables discovery and peers,
and shares one explicit launch definition with the interval flag as its only timing difference.
Imported readiness/health/containment remain in use. No upstream patch or dependency was added.
The existing `reth-smoke` task requires both services so this co-start regression remains executable.
`nix run .#run -- --task reth-smoke` passed (11ms task, 3.35s run), both nodes ready with owned,
reserved endpoints. Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-2590728-1789845533987397213/artifacts/run-summary.json`.
This startup failure is retained as evidence; it is not a passing CI result. The corrected committed
candidate receives the final composed CI below.

## Final CI and assessment

From `/home/willyrgf.linux/dev/mfm2-phase-b`, clean candidate
`4defb1299a79cc12c5de7bf8f568072eeb03c74f` (tree
`42e04b70bfed7fbd8f919b3a97f453ff4f8801d4`):

```sh
nix run .#ci
```

**Passed: nine tasks, zero failures, 2360.688 seconds.** Exact machine-readable result:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-2591337-1789845588547974150/artifacts/run-summary.json`.
All task exits are zero; none timed out or was canceled. This is the final composed run after the
recorded startup-only failure, not an aggregation of earlier focused successes.

| Task selected by CI | Result | Seconds |
| --- | --- | ---: |
| `fmt` | Passed | 0.424 |
| `sqlx-check` | Passed; checked metadata unchanged | 18.699 |
| `clippy` | Passed workspace with warnings denied | 31.495 |
| `cargo-check` | Passed workspace | 7.649 |
| `cargo-test` | 332 passed; seven managed cases ignored by ordinary selection | 1477.665 |
| `doc-tests` | Five passed | 16.958 |
| `postgres-test` | 13 passed across isolated/full selections | 40.912 |
| `client-e2e` | One managed acceptance passed (456.99s test) | 472.077 |
| `effect-e2e` | Two lifecycle, two recipe and one managed test passed | 293.125 |

All seven ordinary ignored cases ran through their explicit managed owners. Live's 58 unit tests
passed together (786.73s), including the full cold failure matrix; native Portfolio integration,
collection restart, independent native implementation, all Runtime custody/cancellation and Store
contracts also passed. Managed lifecycle composition took 154.71s, recipes 0.47s and the actual
PostgreSQL/Reth/keystore scenario 115.00s. Native Pending pacing survives cancellation and cold load,
checks receipt and transaction-known before exact rebroadcast, and preserves supplied errors.

Newly verified production guarantees are: the complete immutable compiler/Runtime path; independent
expected routes through cold native binding; both actual Portfolio continuations without EVM
interpretation in Portfolio/Application; configuration-deleted execution/publication without live
handles for publication; exact native/product failure projection; complete-document serialization
bounds; native codec phases and construction causes; maintained defaults and one depth guard; and
real 42/84 execution with delayed settlement. The Phase A proof is now promoted into exercised
production consumers. No Phase B migration or managed acceptance remains deferred.

Accepted limits remain explicit: downstream authors publish new source semantics once in the
installed environment; arbitrary Rust code cannot be reconstructed from stored identities. Managed
Reth settlement does not define production finality, and a surviving ephemeral signer does not prove
host-process key recovery. No mutation entry point was added to Portfolio or its transports.
Existing unrelated first-loss gaps remain scoped in the adapter audit. These are not newly weakened
contracts or hidden blockers to the amended Phase B scope.

The final evidence-only commit corrects stale routing prose (Journal, not custody, retains
settlement) and removes historical integration-pending wording in design. It is verified with
`git diff --check`, changed-document local-link checks and the LOC script; CI is not redundantly
rerun for these Markdown-only edits. The exact tested code and executable task graph remain those
of `4defb129`.

## Final revision LOC

Reproduce from this evidence revision:

```sh
python3 docs/dsl-phase-b-loc.py 660285d0 HEAD
python3 docs/dsl-phase-b-loc.py c4d97c9d HEAD
```

| Category | Added vs RFC baseline | Removed | Net | Net vs amended proof |
| --- | ---: | ---: | ---: | ---: |
| Production Rust | 11,615 | 7,715 | +3,900 | +849 |
| Test Rust | 17,815 | 8,134 | +9,681 | +574 |
| Documentation | 1,935 | 653 | +1,282 | +743 |
| Manifests/lock/UI/Nix/evidence scripts | 396 | 239 | +157 | -24 |

Production and test counts are unchanged after final CI. The added native semantic boundary and
its qualification/projection machinery account for the Phase B production increase; the promoted
proof accounts for the rest relative to the RFC branch. Deletions and executable replacements are
mapped above. Documentation counts include this complete evidence ledger and historical Phase A
proof, rather than counting them as production code.
