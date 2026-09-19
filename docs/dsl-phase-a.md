# DSL Phase A assessment

Status: **the amended Phase A design passes within the isolated proof scope.** The scalar-route
amendment closes the previous counterexample using the existing compiler, native capability and
Runtime paths. Actual construction/execution covers the five callers, both balance continuations,
two selected native ABIs and the newly enforced caller-owned scalar route. This verdict is the
bounded proof verdict in [RFC §12.1](../RFC_REFACTOR_DSL.md#121-phase-a-bounded-integrated-proof),
not a workspace/production or managed-acceptance pass. Phase B migration has not started.

## Source and reproduction boundary

- Baseline: `b49c90b82b3cb862695582e9a68ee9c2e3bf270b`.
- Isolated branch: `proof/dsl-phase-a`.
- Checkout: `/home/willyrgf.linux/dev/mfm2-phase-a`.
- Pinned Rust: `rustc 1.96.0 (ac68faa20 2026-05-25)`; all Cargo/Rust commands use `nix develop -c`.
- Initial source commits: `5705778b` (typed source/UI), `e0cfb3b2` (shared unsigned arithmetic),
  `4b6e92d2` (Chain addition), `461d1146` (Program callback custody), `d6c7eff3` (invocation versus
  transitions), `656cbfb7` (retained Program document bootstrap).
- Integrated source checkpoint: `fc3bd54e0db3bc1ea6025c72772c58dac0ddfda8`.
- Acknowledgement/classification checkpoint: `58856fefbce14fe7c05aa2088119b9f1837da4af`.
- Previous assessment: `d6a4314ea90970d25d1bd65a6c0ff8a8fc9a9b5c`, with the scalar-route gap recorded.
- Amended source checkpoint: `d7a42c78ec2ea8786dcac05dc67883d2500f8395`, implementing and verifying
  caller-owned scalar-route enforcement and updating constructors/codecs/schema identities together.
- This assessment and its LOC script are a documentation-only successor. The delivery response
  identifies that commit; it changes no production or test Rust.

Reproduce with `git switch --detach d7a42c78ec2ea8786dcac05dc67883d2500f8395` in a disposable checkout,
then run the commands below. The earlier six checkpoints alone do not reproduce the integrated proof.
The original proof commands below establish the inherited evidence at fc3bd54e/58856fef. The scalar
contract amendment has its own focused verification section, run on d7a42c78 sources. Unchanged
Program/Runtime/balance tests were not repeated merely to create another commit.

The selected closure is Values/Capabilities/Program/Runtime/Chain/EVM/Portfolio plus explicit proof
consumers. It contains one replacement compiler and Runtime path. Older integration targets and
Application/live/binary consumers remain visible migration inputs. The proof is not a coherent
production merge candidate and does not claim a workspace build. No targets were disabled to make
these commands pass. Portfolio's pre-existing explicit test-target manifest is retained.

## Approved decisions and scalar-route enforcement

The user approved borrowed `Runtime::execute(..., input: &I)` with synchronous admission encoding,
outcome-bearing `ContractValueEvidence`, and `ReadBalanceAt.route_ref`. These are implemented with
corresponding tests and RFC amendments. Admission encodes once before IO; the blocking-work rule has
that explicit exception. Scalar evidence retains Observed, Rejected, SafeFailure or IntegrityBlocked,
and shared Observe preserves their Permanent failure meanings and native originals.

The scalar-route amendment is now implemented. ContractExecutionConfig v2 retains the caller's
`observation_route_ref`. Shared Observe copies that field into ReadContractValue v2 without decoding
native configuration. ObservedConfiguration and report consistency reconstruct the same route-bearing
intent. Missing fields in prior wires are rejected, and nested descriptors change dependent schema
identities; there is no compatibility decoder.

One EVM qualification function compares the caller's expected reference with the actual native
route's content identity. Configuration resolution calls it during construction; native Read encoding
calls it before invoking an adapter, including when the Program was loaded without source planning
configuration. The expected reference is never replaced by or derived from the selected binding.
Mismatch retains the operation, reason and expected/actual public refs in an invocation-only binding
diagnostic. It cannot become authenticated IntegrityBlocked evidence.

The prior same-ledger alternate-endpoint counterexample in `scalar_read_evidence.rs` is replaced by
rejection assertions. The lifecycle test now also constructs a valid cold Program whose designated
Read has an alternate binding while the actual configured predecessor retains the original route.
Execution and cold resume reject it without provider calls or observation appends. The initial
Runtime admission is explicitly separate: fresh execute retains only frame 1; cold read/resume retain
that head and the counting Store confirms zero additional append attempts. Invalid caller/native
configuration agreement is rejected by compile before any Runtime admission or adapter invocation.

## Scalar-route amendment verification

These commands ran on the amended source checkpoint's Rust/manifests. The existing pinned solc
fixture is the same 497-byte artifact reproduced by the command below; this amendment changes no
Solidity source or compiler settings.

```sh
nix develop -c cargo test -p mfm-chain --all-targets
nix develop -c cargo test -p mfm-chain --test report
nix develop -c cargo test -p mfm-evm --test scalar_read_evidence --test scalar_recipes --test lifecycle_runtime
MFM_EFFECT_E2E_INITCODE_PATH=/tmp/mfm-phase-a-reproduction-8b9up4qh/MfmEffectFixture.bin nix develop -c cargo test -p mfm-evm --test lifecycle_runtime --test scalar_recipes -- --include-ignored
nix develop -c cargo clippy -p mfm-evm --lib --test scalar_read_evidence --test scalar_recipes --test lifecycle_runtime -- -D warnings
nix develop -c cargo clippy -p mfm-chain --all-targets -- -D warnings
nix develop -c cargo fmt --all -- --check
git diff --check
```

All passed. Chain's complete suite passed after constructor migration, and the focused report test
passed again after adding v2/missing-field and changed-route codec assertions. The ordinary EVM run
passed three tests and skipped the two artifact-dependent tests. The explicit artifact run then
passed both lifecycle tests in 185.99 seconds and both recipe tests in 0.56 seconds, with zero ignored.
The temporary artifact path is a recorded invocation, not a required reproduction input: regenerate
it with the solc instructions below and substitute the new path.

| Requested guarantee | Regression evidence |
| --- | --- |
| Intended route succeeds | Native scalar projection; all five lifecycle cases and standalone Observe using the actual configured predecessor. |
| Different same-ledger endpoint rejected | `scalar_read_evidence` asserts qualify_scalar_route and exact expected/actual refs; lifecycle replaces only the designated Read binding with another endpoint. |
| No provider or rejection append | Construction failure leaves script calls at zero; cold mismatch preserves provider count. CountingStore remains at one initial admission call through execute, cold read and resume; no observation, integrity block or failed append attempt is produced. |
| Cold reconstruction without source configuration | Resources<true> has no Resolve implementations. Cold load uses retained Program bytes; fresh execution and subsequent stored-document load/resume both retain the caller's expected reference. |
| 42/84 and unsuccessful evidence unchanged | Full lifecycle and recipe runs pass. Scalar projection retains all four outcomes/native originals; Chain report/Observe tests retain Permanent unsuccessful outcomes and reject success/report construction from them. |

A combined Clippy command accidentally selected EVM's old library test targets via global
`--all-targets` and failed on removed EvmBalanceContext/CheckChainIdentity and related APIs:
`nix develop -c cargo clippy -p mfm-chain --all-targets -p mfm-evm --lib --test scalar_read_evidence --test scalar_recipes --test lifecycle_runtime -- -D warnings`.
That broad result is not green. The subsequent separate focused commands above passed; the obsolete
fixtures remain visible Phase B migration inputs. An initial compile error from the new ContentRef
import was corrected before passing verification. No new architecture blocker appeared.

## Requirement-to-evidence mapping

Paths in this table are repository-relative. Commands are listed below.

| Requirement | Current authoritative evidence | Scope and limitation |
| --- | --- | --- |
| A1: five source shapes and new State | `evm/tests/lifecycle_runtime.rs`, `program/tests/phase_a_source.rs` | Actual Pure, individual Deploy, maintained lifecycle, composed addition and extended lifecycle callers. The extension adds RequireNonZero and retains its typed failure. |
| A1: fixed/nested Operations, local configuration, homogeneous vectors | `phase_a_source.rs`; Portfolio `planning_contract.rs` | Source order/local demands, empty structural identity, tuple arities, product-selected routes and ordered collection declarations. |
| A1: maintained child alone and nested | `lifecycle_runtime.rs` | ConfigureAndObserve executes alone from an actual Deploy output and inside lifecycle compositions. |
| A1: compile-time endpoint exclusions | `program/tests/ui/source_{adjacent,expanded,nested,vector}.rs` via `phase_a_source` | Four checked compile-fail diagnostics; no runtime weakening of endpoint types. |
| A2: inferred complete Program and exact cold identity | `native_construction.rs`, `phase_a_source.rs`, `claim_conflicts.rs` | Executable entries and native public bindings; cold resources lack Resolve/planning implementations; conflicting owners and unsupported/duplicate selection rejected. |
| A2: two ABIs, recursive support, selected resources | `program/tests/native_construction/support.rs`; Runtime `engine/tests/native_abis.rs` | Distinct request/evidence/operational-error types, recursive support Read, four success/failure Runtime cases. Cold terminal inspection uses adapters that panic on invocation. Second ABI is proof infrastructure. |
| A2: Pure requires no live resources; recomposition | Chain `construction.rs`; EVM Pure case in `lifecycle_runtime.rs`; source tests | Fresh/cold Pure construction and actual execution; source tuples compose already installed roots. New State publication appears in resource Sources once. |
| A3: native/token specializations and both continuations | EVM `balance_construction`, `balance_stages`; Portfolio `planning_contract` | Actual native/token protocol expansion; snapshot and enrichment produce distinct nominal outputs with 42/84 source data. Scripted provider only. |
| A3: retained candidate before confirmation | Chain `balance_context`; Portfolio `both_continuations_restore_the_last_candidate_before_confirmation_without_rereading_balances` | Cancel at final confirmation; cold load/read preserves candidates; resume with an adapter accepting confirmation only. No early confirmed result or repeated balance read. |
| A3: order, binding, local scopes | Portfolio planning test; Program `recovery_scopes`; EVM `balance_binding`, `balance_construction` | Ordered source/config data, inherited/replaced checkpoint scopes, foreign retained source rejection and cold designated-route mismatch before IO. Scalar-route agreement is additionally covered by the amendment regressions. |
| A3: arithmetic and bounds | Chain `balance_arithmetic`, `balance_completion`, `balance_request`; Values `unsigned256` | Canonical zero, dust/remainder, increasing-width totals, first overflowing partial total, grammar/range rejection, full-width scalar arithmetic. |
| A4: actual 42/84 predecessor data | EVM `lifecycle_callers_execute_predecessor_data_and_new_state_failure_survives_cold_inspection` | Uses pinned compiled fixture, actual native preparation and calldata, retained deployment output and configured getter word. Scripted settlement/getter, not a running EVM or signer. |
| A4: original custody and checked inspection | Runtime `native_abis`, `recovery`, `original_encoding`; Chain `report`; EVM `scalar_read_evidence` | Exact nominal originals, distinct operational ABIs, nested causes, wrong-type rejection, matching hot/cold bytes; unsuccessful evidence cannot become a successful report. |
| A4: cancellation, pending and ambiguity | Runtime `pending`, `recovery`, `barrier`, projection tests | Actual MemoryStore/Journal transitions, same prepared EffectId across cancellation/cold resume, reconciled ambiguous appends, competing writers, nested allowances and injected Effect checkpoint barrier. Hostile Store/provider scripts supply faults. |
| A4: input/Program admission and acknowledgement | Runtime `admission` and `admission_checks_input_and_cold_reports_retain_the_complete_original_call` | Borrowed non-Clone input, encode once, pre-append rejection, exact cold Program association and previous qualified observation on failed projection. |
| A4: ordinal product failure | Portfolio `failed_second_collection_retains_ordinal_source_and_native_original_hot_and_cold` | Failure in collection ordinal 1 retains completed prefix, active token source, semantic intent and exact native Rejected evidence hot/cold. |
| A4: small-bound and encoding rejection | Runtime `capacity`, `original_encoding`; Canonical `bounded` | Physical two-frame fixture ceiling, actual report encoder at 128 bytes, original/command serializer error and panic. No production-maximum allocations. |
| A5: reviewable assessment | This document, exact source refs, command results and LOC script | Complete passing bounded assessment, including the scalar-route amendment; Phase B boundaries and production verification remain explicitly separate. |

The audit added `original_acknowledgement_loss_defers_classification_until_cold_recovery` to
Runtime's existing recovery fixture. Both absent and physically retained ambiguous original appends
return the prior acknowledged head without classification. Cold read also does not classify; cold
resume classifies only the retained original. This directly checks the acknowledgement boundary,
without introducing a separate engine or production API.

## Encoding and capacity assertions

`original_or_command_encoding_fault_preserves_admission_without_retry_or_classification` runs six
cases: Pure original, Read operational original and prepared Effect command, each with serializer
rejection and panic. It asserts one encoding, no classification or serializer retry, retained
operation/phase/cause, excluded diagnostic panic payload, and unchanged admission head on cold read.
For failed originals, known contract/position survive with identity/detail explicitly unavailable.
Prepared command failure occurs before native materialization, adapter invocation or prepared append.

`report_encoding_bound_preserves_the_durable_original_before_terminal_append` invokes the actual
FailureReport encoder with a test-only run-scoped 128-byte ceiling. It checks the serialization lower
bound and nested BeforeAppend cause, cold AwaitingRecovery at head 2, and subsequent cold completion
under ordinary policy. Production retains its fixed limit; no configurable policy/API is added.

The LimitedStore fixture rejects frame 3 with a two-frame ceiling. It proves Runtime retains either
the acknowledged original or prepared authority when the next append is rejected, not MemoryStore's
production capacity setting. Canonical owner tests use small explicit bounds to prove exact
acceptance/rejection, retained sources/lower bounds and early traversal stop. Prepared-command
serializer errors are not misrepresented as structured size violations.

## Inherited proof verification commands and results

Run from the isolated checkout. These are focused proof checks, not workspace/managed acceptance.
The inherited complete Runtime suite has **23 passing tests**; Runtime implementation and tests are unchanged by the route amendment.

```sh
nix develop -c cargo test -p mfm-ids -p mfm-values -p mfm-program-derive -p mfm-capabilities -p mfm-chain --all-targets
nix develop -c cargo test -p mfm-program --test phase_a_source --test native_construction --test native_callbacks --test claim_conflicts --test recovery_scopes --test identity_error
nix develop -c cargo clippy -p mfm-program --test phase_a_source --test native_construction --test native_callbacks --test claim_conflicts --test recovery_scopes --test identity_error -- -D warnings
nix develop -c cargo test -p mfm-runtime --lib
nix develop -c cargo clippy -p mfm-runtime --lib -- -D warnings
nix develop -c cargo test -p mfm-canonical bounded
nix develop -c cargo test -p mfm-evm --test balance_binding --test balance_construction --test balance_projection --test balance_stages --test native_preparation --test read_contract --test read_evidence --test scalar_read_evidence --test scalar_recipes
```

All commands above passed. Program: 17 tests including four compile-fail fixtures. Canonical: two
tests. EVM: 13 passed and one recipe test ignored because it requires the Solidity artifact; that
ignored case is not counted as evidence until the artifact command below succeeds. The Program run
initially warned about an unused import in recovery_scopes; it was removed and focused Clippy passed.

The combined Portfolio command passed all four tests in 79.56 seconds:

```sh
nix develop -c cargo test -p mfm-portfolio --test planning_contract
```

Fresh artifact reproduction uses the pinned flake's solc 0.8.33. Use a Git flake reference so Nix does
not copy ignored build output into the store. The first local-path-flake attempt failed with no disk
space; removing only this checkout's disposable `target/debug/incremental` cache and using the Git
reference resolved compiler reproduction. No source, test evidence or persistent application data
was removed.

```sh
proof_solc=$(nix build --no-link --print-out-paths --impure --expr 'let f = builtins.getFlake "git+file:///home/willyrgf.linux/dev/mfm2-phase-a"; p = import f.inputs.nixpkgs { system = builtins.currentSystem; }; in assert p.solc.version == "0.8.33"; p.solc')
proof_artifact_dir=$(mktemp -d /tmp/mfm-phase-a-reproduction-XXXXXX)
"$proof_solc/bin/solc" --bin --overwrite --evm-version cancun --output-dir "$proof_artifact_dir" crates/live/evm/tests/fixtures/MfmEffectFixture.sol
MFM_EFFECT_E2E_INITCODE_PATH="$proof_artifact_dir/MfmEffectFixture.bin" nix develop -c cargo test -p mfm-evm --test lifecycle_runtime --test scalar_recipes -- --include-ignored
```

Compiler acquisition and fresh compilation passed. The file contains 497 decoded bytes; the hex
file SHA-256 is `81c908fa2f2978cb41b8ce890bcc9560a7c2183fe11f58ec9392b1617c7e406a`.
The fresh combined execution passed: both lifecycle tests in 168.26 seconds (all five callers and
standalone child), and both scalar recipe tests in 0.55 seconds; zero ignored tests.
The Solidity SPDX warning is unchanged. Generated bytecode is temporary, never committed.

`nix develop -c cargo fmt --all -- --check` initially found only test-module ordering. That ordering
was fixed by pinned cargo fmt; the final format check passed. `git diff --check` also passed
after the assessment rewrite.

The broader command `nix develop -c cargo clippy -p mfm-runtime --lib --tests -- -D warnings`
**failed** on unmigrated integration targets runtime_contract, pending_failure and current_state,
including removed RuntimeAssemblyBuilder/OperationExpansion APIs. It is not recorded as green.
No final CI, PostgreSQL managed task, live signer/provider or CLI/REST acceptance was run for this
isolated proof. RFC §12.1 reserves those production gates for Phase B.

## Surrogates and remaining consumers

Program's nominal source fixtures prove construction/typing, not product semantics. Its second
native family demonstrates a distinct ABI and recursive support, not a shipping network. Runtime
uses minimal kernel-owned synthetic States because it cannot depend on domain crates. Its hostile
Stores prove handling of dispositions; only actual Store/managed tests can establish physical
PostgreSQL atomicity, ambiguous COMMIT acknowledgement and process-restart behavior.

Lifecycle proof uses actual checked chain/EVM contracts and compiled fixture bytes, but scripts
nonce reservation, transaction preparation custody, settlement and getter IO. It does not execute
bytecode, sign, broadcast, validate real provider endpoints or exercise authority SQL. Portfolio
scripts identity/anchor/balance/confirmation evidence and uses MemoryStore; it is not live-chain
acceptance. Cancellation is task cancellation, not a process crash. Cold proof drops and reconstructs
Programs/resources from canonical stored documents; it is not a binary restart.

The source audit found these direct obsolete API consumers. This inventory is not a claim that
unexecuted targets compile or fail for only one reason; the observed Runtime compiler failure above
is separate evidence. Phase B must review the complete dependent target, not just replace a symbol.

| Consumer boundary | Retained migration inputs and required replacement |
| --- | --- |
| Program tests | `src/tests.rs`, `src/tests/depth.rs`, `tests/authoring_boundaries.rs` and scoped-authoring UI fixtures reference removed expansion/scoped authoring. Retain private-construction and depth protections under the new compiler. |
| Runtime integration | `tests/runtime_contract.rs`, `tests/current_state.rs`, `tests/pending_failure.rs`, their submodules and support/UI fixtures use old assembly/expansion contracts. Replace their remaining consumer-specific assertions before deleting them; the new engine tests are not blanket coverage for every old assertion. |
| EVM | `tests/anchored_call_contract.rs` uses removed assembly/expansion. Review other retained domain targets and frozen schema fixtures during the complete cutover. |
| Portfolio | `tests/recovery_policy.rs` uses old injection/expansion; retained enrichment unit fixtures also need the new typed continuation contract. |
| Live EVM | `src/{lib,assembly,transaction}.rs`, `src/{json_rpc_tests,transaction_tests}.rs`, generic transaction and contract Effect integration tests retain old runtime construction. Replace binding/composition once, retaining signer, authority and provider guarantees. |
| PostgreSQL | `src/tests.rs` uses old Runtime construction for persistence scenarios. Preserve repeatable-read admission and exact-head append/ambiguity acceptance when migrating the fixture. |
| Application | `src/{lib,inspection}.rs` and `tests/use_cases.rs` use old composed runtime/inspection. They depend directly on Program, Runtime, Portfolio and Live EVM. |
| CLI/REST | Application consumers must adopt the resulting execution/inspection outputs and preserve their redacted transport contracts and managed e2e behavior. No transport migration was attempted here. |

Inventory command: `rg -l 'RuntimeAssemblyBuilder|OperationExpansion|expand_program|CapabilityInjection|ComposedRuntime' crates --glob '*.rs'`, supplemented by manifests and the relevant test entry files.
The two Live source edits only move provider field access from balance source to balance target;
they do not prove that crate builds against the replacement construction API.

## Simplification and necessary complexity

Removed production machinery includes Runtime assembly/registration and its recovery root maps,
Program's former authoring/expansion module and separate recovery scope module, EVM's old cumulative
balance implementation/decoder, old anchored observation context, and superseded transaction facts.
Program owns complete immutable executable association; Runtime owns transitions and continuation.
Native implementations own translation/projection; shared Chain contracts own semantic balance and
lifecycle facts. Evidence retains exact native Objects instead of reconstructing originals from
rendered diagnostics. Shared admission avoids a second encoding path.

Necessary additions are typed planning/construction, selected native ABI callbacks, shared checked
identity/arithmetic/balance/lifecycle values, two typed product continuations, and boundary proof
fixtures. No new external runtime dependency or parallel engine is introduced; EVM tests add the
existing Runtime/Store crates as dev dependencies. The reproducible delta from the original baseline through amended source checkpoint d7a42c78 is:

| Category | Added | Removed | Net |
| --- | ---: | ---: | ---: |
| Production Rust | 9,239 | 6,188 | +3,051 |
| Test Rust | 10,535 | 1,428 | +9,107 |
| Documentation | 859 | 231 | +628 |
| Manifests, lockfile, UI expectations and other files | 179 | 1 | +178 |

Run `python3 docs/dsl-phase-a-loc.py` from the repository root at the assessment revision. The script
reads the two fixed Git revisions, disables rename compression and classifies added/removed lines.
Counts include blank/comment lines; test directories, test-named Rust files and reviewed cfg(test)
items/blocks are excluded from production Rust. The assessment-only successor and its accounting
script are outside this source-checkpoint delta. This is physical line accounting, not a complexity
score or a claim that every added production line is a new concept.

The +3,051 production lines add shared checked identity, balance and lifecycle contracts plus
complete executable construction, which the former expansion model did not supply. They replace
6,188 lines including Runtime assembly/root maps and native cumulative context machinery. The
increase is necessary proof functionality, not claimed as a LOC reduction. Shared arithmetic,
semantic contexts and one Program/Runtime ownership boundary remove duplicated responsibilities;
there is no second registry or compatibility path underneath them.

The scalar-route amendment alone, from assessment d6a4314e to source d7a42c78, is:

| Category | Added | Removed | Net |
| --- | ---: | ---: | ---: |
| Production Rust | 70 | 11 | +59 |
| Test Rust | 199 | 5 | +194 |
| Documentation | 26 | 13 | +13 |
| Manifest/lockfile | 2 | 0 | +2 |

Reproduce with `python3 docs/dsl-phase-a-loc.py d6a4314e d7a42c78`. The +59 production lines are the
two retained references, checked codec/accessor propagation, semantic intent reconstruction and one
shared native qualification function. That function reuses and replaces the existing route encoding
step; no compiler/Runtime branch, registry, configuration layer or duplicate validation owner was
added. The obsolete acceptance assertion was removed. The only dependency addition is the existing
workspace Journal crate as an EVM dev dependency, needed to implement Store's exact frame interface
in the append-counting fixture; no external or production dependency was added.

## Dependency-supported Phase B boundaries

1. **B1, optional shared scalar groundwork.** The e0cfb3b2 extraction demonstrates an independently
   usable Values-owned Unsigned256 consumed by EvmU256. Keep parser/arithmetic ownership singular;
   include changed original-error schema identities and affected native/provider tests. Do not
   include unused replacement capability or State APIs to manufacture an independent commit.
2. **B2, inseparable construction/execution/domain/consumer cutover.** The new native associated
   types, executable Program document, Runtime signatures, Failure/report wire and shared domain
   continuations are coupled. Migrate Application, live resource composition, CLI/REST and all
   affected tests in the same coherent commit. Remove old registration/authoring/context fixtures
   and transport/root-map projections with retained coverage. Retain the verified scalar-route
   amendment and current schemas; complete the authoring guide and authoritative/transport docs together.
   Merge B1 here if an independently compiling extraction cannot retain complete error custody.
3. **Verification boundary, not a deferred implementation commit.** Re-run the A1–A4 proof on
   production paths, migrate and run managed PostgreSQL/client/Effect acceptance, and run one final
   `nix run .#ci` on the exact candidate. Signing/SQL/transport correctness cannot be inferred from
   this scripted proof. Required deletions and behavior must already be present in B2.

The isolated proof source checkpoint keeps inseparable framework/domain changes together,
followed by the completed A5 evidence commit. It is not permission to merge broken consumers into
production or retain compatibility wrappers.

## Material uncertainties

No material uncertainty remains for the newly tested scalar-route guarantees or the bounded Phase A
verdict. The following uncertainties belong to the explicitly separate production work:

- **Production commit split:** the shared scalar extraction suggests B1 can stand alone, but the
  complete production dependent graph has not been migrated. If that assumption fails, combine B1
  with B2. Validate compiling affected consumers and exact-candidate CI in Phase B.
- **External behavior:** scripted provider/Store faults are assumed to represent the intended
  boundary dispositions, not actual network, signer or PostgreSQL behavior. If production adapters
  differ, the proof cannot establish those guarantees. Managed acceptance in Phase B must validate
  them. No external-behavior pass is claimed by this assessment.
