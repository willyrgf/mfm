# DSL production verification record

This record separates the verified production cutover from subsequent cleanup. Current API and
invariants are owned by [design](design.md), [architecture](architecture.md) and the
[authoring guide](capability-authoring.md). Current commands and independent acceptance oracles are
owned by [build and verification](build-and-verification.md).

## Verified production revision

- Full branch baseline: `6d47027c9a6d2f0a07cae4cd78a0fbd3e0346c10` (`origin/dev` at review).
- RFC implementation baseline: `660285d0`; proof handoff revision: `b49c90b8`.
- Integrated Phase A assessment: `d6a4314e`; route amendment: `d7a42c78`; amended assessment:
  `c4d97c9d`. The isolated proof exercised real kernel contracts with scripted external services;
  it did not establish live provider, signer, transport or PostgreSQL acceptance.
- Production cutover: `472f950a3c42e3a9f503bf10e92b5cd454dd21aa`.
- Combined Reth listener isolation and exact final tested candidate:
  `4defb1299a79cc12c5de7bf8f568072eeb03c74f`, tree
  `42e04b70bfed7fbd8f919b3a97f453ff4f8801d4`.
- Documentation-only evidence successor: `f89faca9115dc7a4d3cee3b20befdf3ab49f90f8`.

B1 was omitted: the native interfaces, complete Program, Runtime, domain contracts and consuming
applications required one coherent B2 cutover. No legacy reader, registration wrapper, alternate
compiler or disabled consumer was retained to manufacture an intermediate release.

From the clean Phase B checkout at the tested revision:

```sh
nix run .#ci
```

Passed: nine tasks, zero failures, 2360.688 seconds. Machine-readable result:
`/home/willyrgf.linux/.local/state/nixfied/mfm/dev/0/runs/run-2591337-1789845588547974150/artifacts/run-summary.json`.
This local path identifies retained evidence; reproduction runs the task above and obtains a new
run artifact. It is not a portable prerequisite or a claim that subsequent commits passed.

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


All seven ordinary ignored cases ran through their managed owners. Live's 58 unit tests passed
(786.73s). Managed lifecycle composition took 154.71s, recipes 0.47s and the PostgreSQL/Reth/keystore
scenario 115.00s. The first combined service startup exposed a listener collision; `4defb129`
assigned independent RPC/peer listeners and the complete composed run above then passed. The
`reth-smoke` task retains co-start coverage. No upstream dependency patch was introduced.

## Retained replacement coverage

This mapping records observable guarantees retained during the cutover. Test names and paths refer
to the tested revision; later consolidation must provide an equally explicit replacement map.

| Removed path or changed boundary | Retained evidence/owner |
| --- | --- |
| Mutable OperationExpansion, root input validation and old Program descriptors | Program `current_program_commits_input_and_rejects_retired_or_forged_contracts`, typed planning/source tests and `authoring_boundaries` compile-fail cases protect exact initial input, private construction and adjacent contracts. |
| Mutable nested policy/checkpoint fixtures | Program `recovery_scopes`: inherited/replaced targets, foreign/forward markers, explicit non-Default parameters, cold identity; `operation_depth`: 16/17 limit before planning/injection, sibling unwinding and shared mixed-injection guard. |
| Handwritten executable registration and native assembly | Program `native_construction`/claim-conflict tests exercise selected-only resources, distinct native ABIs, recursive support, no cold planning, forged ABI rejection and uninstalled code. Derived inventory is shared with inspection. |
| Callback ownership moved to Program/Capabilities | Runtime `current_state` callback-phase/custody tests, native callbacks and original-encoding tests preserve decode/execute/encode provenance, one original encoding, excluded panic payload, and classification only after acknowledgement. |
| Root error maps and report conversion | Runtime reports retain exact domain/Read/pending-Effect originals and calls; report-capacity rejection occurs before terminal append. Live Portfolio client tests decode both continuations and reject foreign context/ABI before public ordinal/code projection. |
| Old balance phase/slot and native product contexts | Native/token injection retains four/five Reads; Portfolio snapshot/enrichment cold restore a candidate before confirmation without repeated balance Reads. Shared arithmetic retains zero/dust/remainder/overflow rules; independent native boundary test shows Portfolio does not interpret EVM. |
| Old Runtime drivers/assembly | Runtime integration/engine tests retain checkpoints, retries, cancellation, competing/ambiguous appends, stopped Pending authority, cold settlement interpretation and terminal replay. `postgres-test` checks actual physical persistence contracts. |
| Fixture deployment/configuration/report | Managed lifecycle uses ContractDeploymentLifecycle, ConfigureAndObserve and production addition; reports 42/84, real predecessor address, exact calldata, native receipt/point and unchanged terminal cold output. |
| Lost reservation acknowledgement and delayed settlement | Managed actual authority row survives acknowledgement loss with no early signing/broadcast; cancellation after real submission cold-resumes retained commands without re-signing. Five transactions produce five unique accepted submissions; receipt/known-status and rebroadcast ordering have focused paused-clock coverage. |
| Generic slot-based existing-address recipe | Managed existing-address call follows external nonce advance 2→3, reserves 3 and advances to 4; composed 84 reserves 4/5 and advances to 6. Native preparation tests reject wrong request/binding/action/implementation. |
| Native custody and operational failures | Managed SQLSTATE 42501 preserves authority causes through App/cold transport; wrong epoch is local with no append; closed signer retains exact cause/classification. Focused native tests retain rejecting signer, exact-wire and ambiguous append cases. |
| Application native interpretation and manual inventory | Native clients own config/output/failure conversion; Application forwards exact reports and product projections. Native source declarations derive decoder dispatch; publication after configuration deletion uses retained public route data without live handles. |
| Client selection/publication | Managed CLI/REST cases preserve generated/explicit RunId, selected revision, independent output/anchor checks, deletion recovery, provider causes, idempotent enrichment publication and dependent start recovery. |
| Complete Program bound and route agreement | Bounded whole-document encoding, aggregate binding overflow, cold pre-parse length, independent expected route and no provider/outcome append on local mismatch have focused regressions. |

The detailed independent oracle requirements live in the
[acceptance table](build-and-verification.md#acceptance-scenarios-and-independent-oracles), rather
than a second set of proposed APIs. The fixed scalar artifact recipe lives in
[build and verification](build-and-verification.md#scalar-contract-artifact).

## Scope and limits

Verified cutover behavior includes the complete Program/Runtime path, exact hot/cold association,
both Portfolio continuations, native codec phases, original custody, local configuration/defaults,
whole-document bounds, real lifecycle 42/84, delayed settlement and managed client publication.

These results do not establish production transaction finality or recovery of keys after the host
process dies. Cold managed reconstruction retained the ephemeral keystore owner. Shipping
Application/CLI/REST entry points remain observational Portfolio products. New downstream State
semantics require one-time source publication; arbitrary Rust code is not discovered from stored
identities. Local diagnostic context and ordinary lifecycle consumer ergonomics were subsequently
identified as cleanup work; the passing cutover does not waive those requirements. See
[known gaps](known-gaps.md) for deliberately deferred capabilities and the older unimplemented
managed extension business oracle.

## Full-branch and incremental accounting

Use the recorded revisions rather than a moving remote-tracking branch:

```sh
python3 docs/dsl-phase-b-loc.py 6d47027c9a6d2f0a07cae4cd78a0fbd3e0346c10 f89faca9
python3 docs/dsl-phase-b-loc.py 660285d0 f89faca9
python3 docs/dsl-phase-b-loc.py c4d97c9d f89faca9
```

| Category | Net full branch | Net since RFC baseline | Net since amended proof |
| --- | ---: | ---: | ---: |
| Production Rust | +3900 | +3900 | +849 |
| Test Rust | +10002 | +9681 | +574 |
| Documentation | +5163 | +1282 | +743 |
| Other | +157 | +157 | -24 |
| Total | +19222 | +15020 | +2142 |

Physical blank/comment lines are included. The script disables rename detection; gross additions
and removals can differ from default diff statistics, while net counts agree. Production ownership
moved from Runtime/EVM to Program/shared Chain; narrower proof-relative growth does not describe
full test/documentation cost. Source comments and test fixtures are not excluded to improve totals.

## Cleanup candidate

The follow-up implements the reviewed consumer, diagnostic, construction, validation and testing
corrections. Flat Report semantics and native Read canonicalization/materialization remain intact.
The latter still invokes the native owner's decoder before IO; typed handoff cannot bypass it.

| Commit | Coherent change |
| --- | --- |
| `f8ac83b9` | Construction diagnostics retain reviewed references and exclude canonical handler parameters; strengthen the existing forged-ABI regression. |
| `215df084` | Generic schema derivation reuses one descriptor while retaining semantic-owner checks. |
| `62fa9916` | State/native/handler/checkpoint/root/callback construction reuses already checked contract references; conflicting owners still reject. |
| `d14a420e` | Balance completeness adds the missing length condition to an already checked prefix. |
| `7ae6f585` | Portfolio preserves checked prefixes incrementally, trusts immutable children, and retains cross-demand and cold relationships. |
| `60b108b3` | Native contract client owns configuration admission and maintained sources; real Pure/new-State consumers and the managed lifecycle use that surface. |
| `2ba685ee` | Consolidate original encoding/cancellation fixtures; direct callback fault coverage complements representative Runtime authority histories. |
| `84ed00c4` | Replace synthetic Portfolio resolution/resources and fault cross-products with maintained consumers and direct typed projection coverage. |

Current code has no additional registry, cache, configuration framework or execution engine.
`ContractResources<Additional = ()>` publishes maintained lifecycle/addition components; only new
semantics need explicit publication. `EvmContractConfig` composes existing checked field contracts
and preserves explicit/absent allowances for Operation defaults. Its admission performs no IO.

### Coverage replacement

| Removed or reduced scenario | Current owner and retained observable guarantee |
| --- | --- |
| Entire synthetic `live/evm/tests/portfolio_runtime.rs` and its resource/adapter model | Maintained client success scenario covers both continuations, config-free load, wrong endpoint rejection, candidate pause, identical head after cancellation, confirmation-only resume, nominal outputs, total 126, enrichment publication and forbidden-provider terminal replay. |
| Portfolio two-continuation/eleven-fault cross-product | Direct actual projector test uses compiler declarations and checked stage inputs for each decoder/original owner; representative native failures for both continuations and shared observation failure retain cold original/intent/ordinal checks. |
| Arithmetic sum failure reached through 53 provider calls | Actual Pure consolidation of a checked completed context supplies the original to the real projector. Domain arithmetic owns numeric limits; native-confirmation and product-consolidation cold histories retain integration custody. |
| Duplicate Runtime original-encoding Read fixture | Engine original-encoding test covers Pure/Read serializer rejection, panic and admitted-shape rejection with one encoding, unavailable detail/identity, no classification/append, excluded payload and cold inspection. The two distinct size-projection tests remain. |
| Weaker Read cancellation test | Recovery test now enters through public Runtime::start and preserves visit identity and allowance as well as the runnable prefix. Effect cancellation remains separately tested. |
| Fifty full callback Runtime histories | All 50 faults run through actual Program callbacks with existing fixture; twelve representative Runtime histories retain operation/phase forwarding, no early IO/prepared authority, pending command, acknowledged settlement and no callback during cold inspection. |
| Fixture-owned lifecycle source list and request recipe | Managed standalone Effect rejection, lifecycle 42 and mixed 84 use native client admission/resources. The small contract_authoring consumer proves Pure 84 and newly published semantic State success 84/typed failure 0 through cold execution/read. |

Full Portfolio Runtime histories decreased from 36 to 7; callback Runtime histories from 50 to 12.
Those counts describe replaced execution cost, not requirements or frozen test assertions.
Independent-native Portfolio coverage, refused versus indeterminate original append, native signer/
authority/wire tests and managed transport acceptance remain separate because they own different
observable boundaries.

### Focused verification

All commands run from this checkout with pinned tooling:

```sh
nix develop -c cargo test -p mfm-program --test native_construction --test claim_conflicts
nix develop -c cargo test -p mfm-values --test value_contract
nix develop -c cargo test -p mfm-program-derive --test derive_contract --test nested_object
nix develop -c cargo test -p mfm-chain --test balance_context --test balance_completion
nix develop -c cargo test -p mfm-portfolio --all-targets
nix develop -c cargo test -p mfm-evm-live --test contract_authoring
nix develop -c cargo test -p mfm-evm-live --test evm_contract_effect_e2e --no-run
nix develop -c cargo test -p mfm-evm-live --lib client::portfolio::tests
nix develop -c cargo test -p mfm-runtime --lib original_or_command_encoding_fault
nix develop -c cargo test -p mfm-runtime --lib cancelled_read_preserves_visit
nix develop -c cargo test -p mfm-runtime --test callback_phases --test original_encoding --test runtime_contract
nix develop -c cargo test -p mfm-app --lib construction_causes_retain_selection_association_and_checkpoint_facts_in_transport
nix develop -c cargo fmt --all -- --check
```

All passed. Program's final marker regression was rerun after removing unnecessary test types.
The five redesigned Live Portfolio tests passed in 82.09 seconds; actual consumer test passed in
5.78 seconds. These are local test-profile measurements, not release performance claims. No
managed services were invoked by the focused checks; the managed lifecycle target was compiled.

A disposable public-API timing probe was removed after execution. Under the unoptimized test
profile, simple Pure compile/load took 5/2 ms; two 17-component Portfolio inventory calls took
7,918/7,979 ms, versus the earlier review's 18,298/17,523 ms. Both construction-reference reuse and
generic derivation changed; these samples do not isolate their contributions or promise production
latency. No timing assertion or benchmark framework was added to the ordinary tests.

The first cleanup CI attempt exposed an Application test that still required whole-State diagnostic
serialization. Its assertion now checks the reviewed identities and preserves exact transport
forwarding; the focused Application regression passed. The correction is included in the diagnostic
commit above. The final CI result and full-branch accounting are pending below. Historical passing CI must not
be attributed to this candidate until its exact combined tree passes.

## Material uncertainties

Focused tests and static ownership review support the cleanup, but the final combined CI has not
yet run. Managed native admission and 42/84 execution remain the final integration check. Performance
samples do not establish isolated causes or release latency. Finality, key-custody, source-publication
and the deferred extension oracle retain their documented limits.
