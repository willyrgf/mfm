# DSL construction and boundary review implementation

Baseline: `05f74977` on `refactor/dsl-phase-b`. The user-approved review is the attachment
`c325b6d4-64fe-4c26-bf58-dbd8109276c7/pasted-text-1.txt`. This record covers its complete five-step
implementation sequence, not a narrowed follow-up to the earlier passing Phase B assessment.

## Required sequence

1. Installed Inventory is the sole owner for fresh/cold executable association. Draft emits only
   declarations, public bindings and policy, checking selected owners against installed contracts.
   Bound handlers decode available parameters during construction; validate every selected parameter
   before native resource binding. Retain generic ABI distinctions and diagnostic causes.
2. One Portfolio collection-set validator owns count/correlation/source aggregate invariants for
   demand and output decoding, retaining local child and output/report checks.
3. Seven infallible Portfolio States declare Never; consolidation has its precise original.
   Product presentation keeps only collection/consolidation summaries, without executable identity,
   classifier or impossible dispatch branches.
4. Read has one completion callback: bind native evidence once, interpret the typed result and
   encode its outcome. Preserve Bind versus Interpret failures and exact originals. Effect retains
   separate callbacks across acknowledgement.
5. Complete the existing failed standalone deployment after restoring resources, on the same RunId,
   retained command and failure prefix. Remove duplicated Pure consumer coverage, name callback fault
   modes, and test manual yield versus automatic continuation after exact-candidate reconciliation.

Each step is one coherent commit with its regression and current documentation. Focused pinned Nix
verification precedes affected managed acceptance and final exact-candidate CI. Final review and
production/test/docs LOC accounting follow all five steps.

## Material uncertainties

None for the approved five-step implementation after the verification recorded below. Existing
operational limits are stated in the final assessment.

## Initial evidence

The old claim-conflict test selected both owners in its source and therefore missed source versus
installed disagreement. Selecting only ShadowAdd against installed Add reproduces the defect:

```sh
nix develop -c cargo test -p mfm-program --test claim_conflicts fresh_and_cold_construction_reject_conflicting_state_and_handler_owners -- --exact
```

Failed at the expected `compile(...).is_err()` assertion: fresh compilation accepted the shadow.
The corrected scenario remains the regression. No State evaluation is needed to reproduce it.

## Step 1: installed ownership and checked handlers

Fresh and cold construction now share `Inventory::associate`. Draft retains declarations and
checks selected owners against immutable installed contracts; it no longer owns callbacks or binds
resources. Full execution ABI participates in State ownership. Public native bindings and handler
parameters are decoded before any resource binding; private temporary binders retain those typed
values without decoding twice. Bound handlers capture non-Clone parameters and never decode during
recovery. No persistent schema or additional public registry is introduced.

The shadow-owner regression now asserts `select_state`/`select_handler` and `conflicting_owner`.
New construction coverage rejects schema-valid, typed-invalid handler parameters and later native
bindings before *any* resource binder, on fresh and cold paths. It checks one decode per construction,
non-Clone parameters, and handler decoder panic containment. Existing generic ABI, native cold
construction and recovery-scope coverage remains. Application diagnostics now install the actual
Pure State when testing checkpoint rejection; duplicate installed owners fail during discovery,
before an occurrence has a position.

Pinned verification (all passed):

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-program --all-targets --message-format short
nix develop -c cargo test -p mfm-runtime --all-targets --message-format short
nix develop -c cargo test -p mfm-app --lib construction_causes --message-format short
nix develop -c cargo check --workspace --all-targets --message-format short
nix develop -c cargo clippy -p mfm-program -p mfm-runtime --all-targets --message-format short -- -D warnings
```

The Runtime classification compile-fail diagnostic was reviewed and updated for the added installed
`Discover` bound, then passed in the normal Runtime suite. Architect review found no ownership
blocker; its request for precise shadow-owner assertions was incorporated. Native binding decoder
panic containment is not a newly claimed guarantee. Managed acceptance and final CI remain pending
until the complete five-step candidate.

## Step 2: aggregate validation

Step 1 revision: `8c83ce25`. Its accounting against `05f74977` is production +254/-197 (net +57),
test Rust +396/-17 (net +379), documentation +102/-6 (net +96), UI diagnostics +24/-0.
The necessary production increase implements selected-owner checks and full preflight before binding;
it deletes the independent hot association authority and repeated recovery parameter decoding.

One private collection-set validator now owns count, correlation uniqueness, total-source bound and
source uniqueness for admission and output. Deleted the separate summation and output source-ID
validator, including owned source-ID copies. Local child checks and output/report agreement remain.
The existing maximum-field scenario now mutates output wire independently: each child still decodes,
but duplicate correlations, cross-child duplicate source IDs and two valid 64-source children fail
aggregate decoding. Re-running the new test against the preceding production implementation failed
at the expected output-rejection assertion; the corrected implementation passes.

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-portfolio --all-targets --message-format short
```

Passed all four unit tests and the independent native ABI consumer integration test (both maintained
Portfolio continuations). Production change is +41/-44 (net -3); regression change +51/-0.

## Step 3: exact originals and product summaries

Step 2 revision: `07f1d6a2`. Seven Portfolio States now declare `Never` and consolidation alone
owns `PortfolioConsolidationFailure::AggregateCapacityExceeded` (Permanent). The product summary
loses unused InvalidInput, executable schema identity and classification. Deleted both native-client
dispatch macros, seven impossible branches and the generic product-original decoder. The one
consolidation decoder derives input/failure contracts from its owning State. No compatibility reader
is retained: changed failure ABIs select the current exact contracts.

The existing actual arithmetic failure scenario now decodes the precise consolidation original,
checks unchanged product rendering, and compares exact retained originals and publication hot/cold.
Infallible State consumers use irrefutable success patterns, deleting impossible test panic branches.

```sh
nix develop -c cargo fmt --all
nix develop -c cargo check --workspace --all-targets --message-format short
nix develop -c cargo test -p mfm-portfolio --all-targets --message-format short
nix develop -c cargo test -p mfm-evm-live --lib client::portfolio --message-format short
nix develop -c cargo test -p mfm-app --lib --message-format short
nix develop -c cargo clippy -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets --message-format short -- -D warnings
```

All passed: Portfolio four unit/one integration, native client six, Application fourteen. Initial
workspace checking identified the now-irrefutable test branches; these were removed and Clippy
passed without warnings. Production +48/-67 (net -19), tests +15/-54 (net -39), before documentation.

## Step 4: one Read completion

Step 3 revision: `1ebae644`. Read now has one completion callback which binds/projects native
evidence once, directly interprets its typed result and encodes the outcome. Removed the separate
binding callback and duplicate captures/projection. The internal kernel-only `ReadCompletionFailure`
distinguishes Bind and Interpret while carrying existing `CallbackFailure` unchanged; Runtime keeps
its existing operation diagnostics. Effect callbacks and acknowledgement boundaries are unchanged.

The existing native callback scenario now counts exactly one projection while asserting retained
native evidence and semantic output. Existing fault cases retain binding/interpretation provenance,
codec phases, original acknowledgement timing and cold inspection. No callback authority moved.

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-program --test native_callbacks --message-format short
nix develop -c cargo test -p mfm-runtime --test callback_phases --message-format short
nix develop -c cargo test -p mfm-program -p mfm-runtime --all-targets --message-format short
nix develop -c cargo clippy -p mfm-program -p mfm-runtime --all-targets --message-format short -- -D warnings
```

All passed, including the normal compile-fail checks and Runtime cancellation, append/reconciliation,
recovery, classification and original-custody suites. Production +38/-39 (net -1), tests +27/-9
(net +18), before documentation. The narrow result distinction adds provenance without a second
error framework or evidence cache.

## Step 5: consumer evidence and overlap removal

Step 4 revision: `2cf07e8a`. The existing managed standalone deployment now resumes after restoring
its authority on the original RunId. It compares every prior frame byte-for-byte, finds settlement of
the exact retained Effect call/command, independently observes the creation receipt, checks one extra
signature/submission and nonce, then verifies unchanged terminal replay. It reuses the existing
fixture and resource reconstruction; no additional scenario framework or Runtime policy is added.

The existing NotInserted fixture now covers both manual progression and automatic `execute` at all
three Effect append boundaries. Manual calls yield the checked winner; automatic execution may
continue. Both paths make one adapter call using the retained command ref/value and EffectId, and
cold resume completes without repeating it. Runtime execution code is unchanged by this step.
Current design/Runtime documentation now states that distinction.

Callback fault controls are named private variants, including original codec/classifier faults.
All previous fault phases, reviewed nested causes, panic-payload exclusion, acknowledgement timing,
one original encoding and cold original custody remain exercised. Test-only wire labels now describe
the simulated fault rather than encoding its behavior as numeric ranges.

### Replaced coverage

| Removed/replaced coverage | Retained observable owner |
| --- | --- |
| Source selecting both conflicting States/handlers | `claim_conflicts`: selected shadow versus installed owner, exact operation/reason; installed conflicts and generic ABI distinctions remain separately covered. |
| Invocation-time handler parameter decoding | `handler_construction`: checked non-Clone parameters, one decode hot/cold, repeat recovery requests without decoding, invalid later values prevent every native binder. |
| Constructor-only maximum Portfolio output | `snapshot_extremes`: valid boundary values plus independently malformed wire; every child decodes before aggregate rejection. |
| Seven impossible Portfolio domain failures and generic projection | Owning States declare Never; actual native-client arithmetic scenario retains exact consolidation original, unchanged product summary and cold report bytes. |
| Separate Read bind/interpret projection | `native_callbacks` counts one native projection; `callback_phases` retains all codec causes and representative Runtime operation/head checks. |
| `existing_pure_caller_executes_without_binding_any_native_adapter` | Existing Live `contract_authoring::pure_selection_and_new_semantics_use_the_same_cold_execution_path`: 42+42=84, cold Program, empty bindings and actual resources with no native handles; also new State success/rejection. |
| Manual-only append-race assertion | Existing `runtime_contract` fixture now checks manual yield and automatic completion against the same exact retained command/EffectId. |
| Managed standalone rejection/inspection only | Same deployment completes after resource restoration, preserving all failure-prefix bytes and command authority; independent receipt and terminal replay checks added. |

Focused verification passed:

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-program --test native_callbacks --message-format short
nix develop -c cargo test -p mfm-runtime --test callback_phases --test runtime_contract --message-format short
nix develop -c cargo test -p mfm-runtime --test callback_phases --message-format short
nix develop -c cargo check -p mfm-evm-live --test evm_contract_effect_e2e --message-format short
nix develop -c cargo test -p mfm-evm-live --test contract_authoring --message-format short
nix develop -c cargo clippy -p mfm-program -p mfm-runtime -p mfm-evm -p mfm-evm-live --all-targets --message-format short -- -D warnings
```

Program native callbacks: four passed; Runtime phases: five passed; Runtime contracts: fourteen
passed; public authoring: one passed. The direct `cargo test -p mfm-evm --test lifecycle_runtime
--message-format short` compiled successfully but ignored its one fixture-dependent test; it is not
execution evidence. The managed Effect task supplies its pinned Solidity artifact.

Final architect source review accepted all five steps, finding no implementation blocker, new
registry, compatibility path or duplicate executable authority. That review did not independently
run managed tests or CI. Native binding decoder panic containment is an existing limitation and is
not claimed by this cutover; the new handler decoder panic boundary is covered separately.

Additional focused scalar check passed:
`nix develop -c cargo test -p mfm-evm --test scalar_read_evidence --message-format short`
(one test retaining all observation outcomes and mismatch/malformed rejection).

The first managed `nix run .#run -- --task effect-e2e` attempt passed the one domain lifecycle and two
scalar-recipe tests, then the enlarged live test future overflowed its test-thread stack before the
scenario ran (task exit 101). Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-245229-1790156017398676327/artifacts/run-summary.json`.
The added recovery block now uses one immediately awaited boxed future; no Runtime change, stack-limit
increase or assertion removal was made. Focused fixture Clippy passed with
`nix develop -c cargo clippy -p mfm-evm-live --test evm_contract_effect_e2e --message-format short -- -D warnings`.
The complete managed task was rerun after that correction; results are recorded below.

Managed Effect rerun passed: one lifecycle test, two scalar-recipe tests and one live recovery test
(65.60s live test; 184.512s task, 186.245s total). Exact command:

```sh
nix run .#run -- --task effect-e2e
```

Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-262855-1790156328347726397/artifacts/run-summary.json`.
Architect follow-up accepted the allocation correction and retained empty-binding assertion.
Step 5 adds no production Rust: test Rust +597/-321 (net +276). The increase replaces numeric fault
ranges with explicit variants, extends the existing reconciliation/managed fixtures, and deletes
43 net lines of duplicate Pure consumer coverage. Managed client acceptance and final CI remain
pending at this implementation commit; the final assessment below records their results.

## Final CI fixture correction

The first full CI on `18eb0b62` passed fmt, SQL metadata, workspace Clippy and compilation, then
stopped at two Application readiness watchdogs. Nine other `use_cases` tests passed; the failures
were exactly `Read was not entered` at the old ten-second fixture deadline, before the intended
cancellation point. Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-268981-1790156752138467609/artifacts/run-summary.json`.
The smallest preserved reproduction is the committed `18eb0b62` fixture with:

```sh
nix develop -c cargo test -p mfm-app --test use_cases snapshot_progresses_after_an_interrupted_read -- --exact
```

It also failed in isolation at 10.01s, so parallel contention alone is not claimed as the explanation.
Temporary instrumentation with the reviewed 120-second fixture bound measured provider entry at
10.1967s and 10.1914s. Both isolated scenarios then passed (27.95s and 36.38s total), including their
retained-history and ambiguous-recovery assertions. Measurement commands added `--nocapture`; the
instrumentation was removed after diagnosis. These are local debug-build timings, not latency
benchmarks or an API guarantee.

Architect inspection found no notification race: `notify_one` retains a permit, providers are local
to each test, and the ambiguous admission fails before provider IO. The correction uses one shared
`READ_ENTRY_WATCHDOG` of 120 seconds to bound a stalled fixture while allowing complete construction
and admission. Both tests still cancel only after actual provider entry, and all subsequent
assertions remain. No production deadline, scheduling change, retry, discovery cache or test
serialization was added. Test Rust +6/-2 (net +4); production Rust unchanged.

With temporary timing removed, focused verification passed on the correction:

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-app --test use_cases --message-format short
nix develop -c cargo clippy -p mfm-app --test use_cases --message-format short -- -D warnings
```

All eleven Application use cases passed under the default parallel test runner (110.85s), including
both readiness/cancellation cases. The complete final CI is rerun on the committed correction.

## Final verification and verdict

Implementation revisions, in dependency order:

| Revision | Coherent cutover |
| --- | --- |
| `8c83ce25` | Sole installed association authority and checked bound handlers |
| `07f1d6a2` | One Portfolio aggregate validator |
| `1ebae644` | Exact consolidation original and presentation-only product summaries |
| `2cf07e8a` | Single Read completion with preserved phase/operation causes |
| `18eb0b62` | Retained consumer recovery, named faults and coverage replacement |
| `cf18ab5e` | Bounded Application readiness watchdog; cancellation assertions unchanged |

Managed client acceptance passed with `nix run .#run -- --task client-e2e`: one test, 136.36s;
task 151.545s, total 152.671s. Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-266141-1790156545804391178/artifacts/run-summary.json`.

Final CI candidate: `cf18ab5e6539fb3cda0e6df050c8a4e589b40a68`, clean tree
`ec4d660016ee14df478f5cb4fed9ca31fb3f862e`. Command: `nix run .#ci`.
Passed all nine tasks, zero failures, 676.274s total. Evidence:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-278866-1790157402070025852/artifacts/run-summary.json`.

| Task | Result | Seconds |
| --- | --- | ---: |
| `fmt` | Passed | 0.407 |
| `sqlx-check` | Passed; checked SQL metadata unchanged | 1.025 |
| `clippy` | Passed workspace with warnings denied | 3.358 |
| `cargo-check` | Passed workspace | 2.538 |
| `cargo-test` | 330 passed; seven ignored by ordinary selection | 299.515 |
| `doc-tests` | Five passed | 7.862 |
| `postgres-test` | 13 passed across managed selections | 13.878 |
| `client-e2e` | One passed; configuration-deleted recovery/publication | 145.875 |
| `effect-e2e` | Four passed; lifecycle, recipes and live retained recovery | 200.173 |

The managed client test took 135.29s; live Effect recovery took 80.61s. Timings are local
observations, not a controlled performance comparison.

The documentation-only successor records evidence and corrects one Rustdoc description to say
that ConsolidationFailed projects aggregate capacity rejection, not local binding failure. No
executable tokens, test, manifest, schema, Nix task or lockfile changes after the tested candidate.

Verdict: the complete five-step review is implemented and passes its focused, managed and final CI
verification. Newly verified relationships are installed-versus-selected ownership, global
construction preflight before resource binding, demand/output aggregate invariants, exact originals
versus product summaries, one Read projection, same-run deployment recovery with immutable failure
prefix, and manual/automatic continuation using exact retained command authority. The production
Phase B cutover and managed acceptance are complete within these contracts.

This is not a claim of universal constructor-panic containment: native binding decoder panic
containment remains outside the new guarantee. Managed key custody remains alive across Runtime
reconstruction; it is not a host-process restart test. Finality, publication of new semantics and the
separately deferred extension business oracle retain their documented [known gaps](known-gaps.md).
No unresolved blocker remains for this approved implementation sequence.

## Final LOC accounting

Physical lines include comments/blank lines and all replacement tests. The existing script separates
inline/external test-only Rust, documentation and UI diagnostics. Reproduce from the final evidence
commit:

```sh
python3 docs/dsl-phase-b-loc.py 05f74977 HEAD
python3 docs/dsl-phase-b-loc.py 6d47027c9a6d2f0a07cae4cd78a0fbd3e0346c10 HEAD
```

| Category | This review: added / removed / net | Full branch: added / removed / net |
| --- | ---: | ---: |
| Production Rust | +382 / -348 / +34 | +11,798 / -7,773 / +4,025 |
| Test Rust | +1,075 / -386 / +689 | +18,444 / -8,121 / +10,323 |
| Documentation | +419 / -15 / +404 | +2,065 / -975 / +1,090 |
| UI/other | +24 / -0 / +24 | +362 / -239 / +123 |

The review's production increase is 34 lines: selected-owner checks and complete preflight require
57 net lines in step 1, offset by 23 lines removed from aggregate validation, product failure
projection and Read completion. Deleted independent hot executable construction, repeated recovery
parameter decoding, owned source-ID copies, broad impossible originals, duplicate dispatch macros,
and the second Read projection. No registry, binding cache, compatibility wrapper or parallel API
was added. Test growth adds missing cross-path assertions and readable fault variants to existing
fixtures, plus non-Clone checked-handler/native-binding preflight coverage. The full branch delta is
reported separately so the review baseline does not hide the earlier production cutover's growth.
