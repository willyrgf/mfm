# Recovery replacement validation

This records evidence for [the recovery RFC](../RFC_REFACT_RUNTIME_TO_RECOV_SM.md).
The first milestone is complete. Program v4 and Journal frame v3 are the current contracts;
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

The standalone signature proof is supporting evidence only. The results above establish the
production milestone before the complete consumer replacement and current wire declaration.

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

Final composed CI has not run.

## Material uncertainties

none for the completed focused scenarios. Final CI remains required; focused results do not
substitute for that gate.
