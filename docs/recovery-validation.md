# Recovery replacement validation

This records evidence for [the recovery RFC](../RFC_REFACT_RUNTIME_TO_RECOV_SM.md).
The first milestone is complete. Program v5 and Journal frame v3 are the current contracts;
there is one linear compiler and one Runtime semantic fold. Old graph bytes are rejected.
The inseparable replacement and subsequent enrichment are validated by the focused gates below.

## Architectural decisions

The dedicated architect reviewed the RFC, sketch, contracts, and assembly. Initial decisions are
committed in `5558dc6f` (`record recovery validation decisions`). Program owns scoped authoring and
finite lifecycle arithmetic; Runtime owns association and the sole semantic fold; Journal owns exact
wire qualification and numeric ceilings; Store remains mechanical. No compatibility reader or
parallel Runtime is retained.

Checkpoint allocation identity prevents capture from a parent, sibling, or previous expansion.
Descendants can inherit installed policy bindings without acquiring ownership of their tokens.
Injection has explicit before/designated/after scopes and a designated failure map. Effects establish
recovery barriers when their command authority is retained. Typed operational errors and State
adapter contexts are distinct from original domain failures; root maps run only for Stop.

Shipping Portfolio uses NoRecovery/Stop with zero global and local allowances. EVM recovery is an
explicit policy selection, not an implicit registration side effect. AnchorChanged is checked,
authenticated domain evidence; a local adapter mismatch cannot become a durable integrity block.

Recursive injection exposed stack exhaustion before the old 64-level limit. The architect selected
one synchronous authoring contract of 16 callback levels, checked before entry, with a boxed suspended
occurrence descriptor. Failed child drafts do not merge. No larger test stack or alternate compiler
was introduced.

## Production milestone evidence

All direct Rust commands ran in the pinned default Nix shell. Tests use production MfmValue types,
Operations, adapters, Runtime assembly, and the current Journal format.

| Boundary | Result |
| --- | --- |
| Portfolio actual-domain policy selection and restart (`recovery_policy`) | Both tests passed in 16.38 seconds. Four policy selections cover parent mapping, inherited child policy, original EVM classification, and occurrence overrides. Both changed-confirmation and changed-initial-anchor restarts discard stale balances and retain coherent replacement anchors, evidence, decisions, positions, and output. Cold reads make no provider calls. |
| 64-source Portfolio admission (`runtime_contract`) | Passed in 227.31 seconds. Actual genesis is 6,683,411 bytes; declared history is 515 frames / 138,309,207 bytes. The first typed Read failure is retained and cold-readable without provider IO. |
| Portfolio planning matrix (`planning_contract`) | Both tests passed in 1,102.23 seconds. Six native/mixed/token layouts with one or 64 collections fit. Frame bounds are 262/451, 294/483, and 326/515; corresponding byte bounds are 69,618,867/120,100,667, 79,656,987/129,155,163, and 90,012,347/138,373,499. A 65th source is rejected. |
| Maximum single creation (`recovery_transaction`) | Success, revert, and pending-authority recovery passed; rerun after the authoring-depth fix took 37.47 seconds. Actual histories use eight frames and 1,256,385–1,256,405 bytes; declared budget is 15 frames / 3,722,530 bytes. Excessive allowances reject before Store admission or provider calls. |
| Accumulating transactions (`evm_contract_effect_e2e`, deterministic tests) | Success/failure/cold facts passed in 635.11 seconds. All 12 capacity cases passed in 1,452.92 seconds, including maximum creation/call and returned evidence. Budget is 47 frames / 133,736,464 bytes; maximum actual run is 9,315,661 bytes, frame 1,327,931 bytes, terminal value 1,675,520 bytes. |
| Live EVM library | All 36 tests passed in 262.32 seconds, including real loopback timeout/429 classification, cancellation, and ambiguous acknowledgements at each transaction boundary. Additional signer and authority cause regressions passed. |

The results above establish the production milestone before the complete consumer replacement
and current wire declaration. Production consuming tests now cover the original signature examples.

## Replacement verification

| Command or boundary | Result |
| --- | --- |
| `nix develop -c cargo test -p mfm-program -p mfm-journal --all-targets` | Passed: seven Program unit tests, authoring privacy/borrow compile-fail cases, two Journal unit tests and 12 wire contract tests. Includes canonical vectors, old-format rejection, scoped checkpoints, finite arithmetic, and ordinary-stack child/injection/mixed depth boundaries. |
| `nix develop -c cargo test -p mfm-runtime --all-targets` | Passed: 11 unit tests, private-authority compile-fail fixture, and 19 external contract tests. Covers hot/cold equivalence, cancellation, typed callback errors, lost append races, pending authority, and source-preserving Store errors. |
| Production Portfolio after depth fix | Native/token success, original/mapped failure, operational context, and cold reconstruction passed in 40.12 seconds. |
| Store and PostgreSQL focused tests | All six shared Store tests and five local PostgreSQL tests passed. The managed `postgres-test` task passed in 18.80 seconds, run `run-3891134-1788902678842453945`. |
| Generic transaction Runtime | All five tests passed in 30.56 seconds. |
| Application | Nine unit and seven integration tests passed; integration took 38.84 seconds. Interrupted Read, durable operational failure, cold reconstruction, ambiguous acknowledgement, and shared public rendering are covered. |
| CLI / REST | Three CLI tests and the REST router contract passed. |
| `nix develop -c cargo check --workspace --all-targets` | Passed after the consumer ports. |
| `nix run .#run -- --task client-e2e` | Passed in 82.36 seconds, run `run-3880222-1788902058714876169`. REST is killed during the first actual Read; the retained three-frame prefix survives config deletion, cold-resumes against Reth, and renders identically through CLI. A fresh CLI-generated run produces the same semantic output. |
| `nix run .#run -- --task effect-e2e` | Passed in 169.32 seconds with a 300-second cold-progress deadline, run `run-3886671-1788902430582699486`. The old 60-second deadline expired after frame 13; the completed rerun proves cold settlement, retained facts, and terminal nonce/head/output stability. |

Application and both transports now distinguish Runnable, EffectPending, Succeeded, and Failed.
Failed embeds the canonical FailureReport; stopped invocations preserve their observed view and
reviewed detail. Ambiguous acknowledgement preserves recovery identity and last_observed, including
null before any qualified observation. Obsolete Match, label, join, selector, and failure-handler
machinery and fixtures have been deleted or replaced by current authority tests.

Scoped Clippy passed with warnings denied across all changed runtime/domain/application/transport
targets. IDs, values, derive, and capabilities all-targets tests passed, as did the scoped
Program/Runtime/EVM/Portfolio/Application rustdoc tests. Runtime all-targets passed again after boxing large incident/report payloads.

## Enrichment verification

The separate candidate implementation follows [the selected design](recovery-enrichment-design.md).
Domain execution passed native retention, ordered nonzero-token selection, provider failure, and cold
reconstruction in 38.76 seconds. The digest regression checks exact lowercase hex and canonical
MfmValue qualification; it preserves the existing ContentDigest grammar. Maximum 64-source output,
configuration and root-closure tests passed for one and 64 collections. Candidate and published
config documents with maximum public fields remain below 128 KiB, within the unchanged 256 KiB cap.

Application all-targets passed nine unit and eight integration tests before the additional hostile
publication scenarios. Those scenarios passed loss of a committed publication acknowledgement,
repeat without provider IO, exact one-digit revision mismatch, wrong output schema, forged head,
resolved values and route rejection, and a new immutable revision without changing the old one.
Pending enrichment cannot publish; start can resume its cancelled Read. The complete regression,
including a resumed-start ambiguous acknowledgement retaining its Start envelope and observed
three-frame prefix, passed in 49.61 seconds. Runtime all-targets passed after the genesis/entry-point
view changes.

CLI/REST focused tests and scoped Clippy passed. Managed `client-e2e` passed in 113.58 seconds,
run `run-3909240-1788904054041268951`: actual Reth enrichment starts through REST, survives candidate
config deletion, publishes through REST, repeats through CLI, executes a dependent snapshot and
recovers its exact start after published-config deletion. The initial run's 204 JSON fixture error
was fixed; it was not a product failure.

The first composed CI run passed format, SQLx, Clippy and workspace compilation. It was cancelled
during workspace tests after the acceptance audit identified missing nested/global-budget coverage.
The new Runtime regression alternates inner and outer checkpoint restoration, proves preserved
inputs and cold positions, and exhausts global and local restart allowances independently. It passed
in 0.32 seconds without implementation changes.

The final composed `nix run .#ci` passed on `ae3b5b53`, run
`run-3931623-1788905182655598248`, in 4,587.728 seconds. Its `artifacts/run-summary.json` records
all 12 tasks successful with exit code zero: format, SQLx, Clippy, workspace check, workspace tests,
rustdoc, application/Runtime/Store capacity, PostgreSQL, client E2E, and Effect E2E. Workspace tests
took 2,798.802 seconds; application capacity took 1,461.01 seconds. This closes the replacement and
enrichment CI gate.

## Independent-review follow-up

The object and inline-report ceiling is now 32 MiB; complete frames allow four maximum objects plus
a 64 KiB envelope. History remains limited to 65,536 frames and 512 MiB. PostgreSQL baseline v2
matches the frame ceiling and rejects the older baseline. Original and mapped failures remain inline.
A combined report can still exceed its ceiling: `size_limit_exceeded` exposes the resource, actual
bytes and limit, and stops before the terminal append. The run stays Runnable or EffectPending.
This is an accepted contract, not a guarantee that every admitted failure can terminate.

Focused verification passed in the pinned shell:

| Boundary | Evidence |
| --- | --- |
| Values and reports | Exact 32 MiB accepted; one byte over rejected with precise metrics. Pure and Effect runs with 5 MiB failures terminate and reconstruct cold; 17 MiB failures exceed the combined report limit, and 33 MiB failures exceed the object limit, preserving the prior head and Effect identity. |
| Journal and Store | All-targets tests passed, including four distinct maximum-size Read objects, precise frame/history limits, malformed wire and predecessor checks, and rejection of a valid frame extending the wrong history without changing its head. |
| Runtime simplification | Canonical and Runtime all-targets tests passed after shared immutable value storage and precomputed checkpoint membership, including 19 external Runtime contracts, nested restart/barrier tests, and report boundaries. |
| Public errors | Application serialization test passed for the safe resource/actual/limit fields; workspace compilation and warnings-denied Clippy passed. |
| PostgreSQL and task graph | `sqlx-prepare` passed (run `run-86637-1788948818927666024`); managed PostgreSQL tests passed (run `run-93707-1788948951259946209`), including rejection of the old baseline; `model-check` passed. |

Raw Store transfer bytes remain untrusted. Journal qualifies canonical bytes, hashes, exact chain
linkage, object closure, and lifecycle adjacency before Runtime folds them. The encoded-frame wrapper
has been merged into the sole checked frame representation. Effect prepare and conclusion records
remain intact. Production consuming examples replace the deleted signature prototype.

Follow-up candidates use one final `nix run .#ci` after their focused checks. The candidate-specific
result is recorded by Nixfied in `runs/<run-id>/artifacts/run-summary.json` under its state directory
and reported with the candidate commit; the earlier composed result above applies to `ae3b5b53`.

## Material uncertainties

none. The oversized-report behavior is explicitly accepted and covered by regressions; focused
follow-up results and the earlier composed result are identified separately above.
