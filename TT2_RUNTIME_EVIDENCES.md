# TT2 runtime remediation evidence

This append-only ledger records the implementation candidate, focused evidence,
and the independent review for `IMPL_PLAN_PROBLEMS_TT2_IMPLRFC_RUNTIME.md`.
It is deliberately not a blanket pass: the runtime cutover is substantially
implemented and the focused candidate checks are green, but the plan's strict
§12/§13 acceptance bar still has identified proof and deployment-boundary gaps.

## Material uncertainties

- `mfm-authority-seal` is unpublished and is treated as a workspace deployment
  boundary. If the marker package is distributed independently, the sealed
  authority surfaces require a new review.
- SQLx/offline metadata, SQL inventory completeness, and model-check results in
  the composed gate are task evidence, not an independently reproduced audit
  of every query/model generation step.
- The concrete production implementations of retained physical-release and
  store/checkpoint trust are outside this workspace; the portable tests use
  explicit test implementations. Their deployment integration is therefore
  still unverified.
- No injected process-kill matrix was run for every EVM broadcast/receipt,
  finality, promotion, and completion boundary. The release qualification
  covers restart/reload paths, not all kill points.
- The application production path still has no live multi-hop export
  integration fixture. The implementation now performs kind-aware fixed-point
  discovery and retains an authenticated principal, fixed export grant, and
  content-addressed decision reference for every selected prefix; the live
  production deployment proof remains absent.
- Decision references are intentionally opaque content-addressed policy
  evidence. Offline replay does not resolve them live; the explicit trust
  snapshot binds the exact closure digest. If deployment requires independent
  decision-record resolution, that is a new authority contract.

## Ordered implementation revisions

The original plan cutover is retained here, followed by the post-step-12
revisions that closed review findings and added portable/recovery evidence.

| Step | Revision | Subject |
| ---: | --- | --- |
| 0 | `f55300ab5` | plan tt2 runtime history remediation |
| 1 | `07b9d7311` | seal runtime physical access behind committed history |
| 2 | `b03520fce` | bind postgres sessions to external deployment authority |
| 3 | `8a450cad4` | make postgres loads and races exact |
| 4 | `186c008f7` | share canonical storage ingress and inventory every sql query |
| 5 | `3e4ba6729` | bound prior fact verification by unique producer prefix |
| 6 | `3051a50cf` | restrict run evidence to its exact purpose |
| 7 | `0deb0ffd9` | move decrypted keys through one protected allocation |
| 8 | `fd9fd2017` | qualify evm signer and pending nonce evidence |
| 9 | `d0b6995f3` | restore stable evm intent and bounded wallet currentness |
| 10 | `afd31b559` | make evm recovery observation first and producer authorized |
| 11 | `e535b6b02` | delete replay reproduction and generate exact projection contracts |
| 12 | `d9e53a5e9` | finish tt2 runtime remediation cutover |

Post-step-12 implementation and proof revisions:

| Revision | Subject |
| --- | --- |
| `4104ddff1` | exercise fresh production keystore submission |
| `a6d9f1cb8` | harden portable frame tamper bounds |
| `7c8488b7a` | prove portable offline replay parity |
| `a4a484540` | exercise recursive portable replay parity |
| `c070c674a` | add portable replay corpus leaf |
| `1de47c5c0` | seal recursive portable source fixtures |
| `b0b28382b` | classify concurrent configuration conflicts |
| `bcfc6f0cc` | retry raced configuration append identities |
| `786ebe12f` | fix portable frame bound lint |
| `2b870b01f` | accept flattened portable source closures |
| `7ceed7683` | hide store fixture authority from surface tests |
| `2d5b17139` | hide fixture authority behind replay accessor |
| `076a50c1d` | complete portable replay bound and trust proofs |
| `3c7525a61` | bound export source discovery |
| `a7ef6b636` | bound portable source and fact route discovery |
| `d67a3bc3a` | add portable source bound regressions |
| `b3dfd8377` | retain kind-aware export authorization closure |
| `e68aca910` | classify configuration idempotent checkpoint predecessors |
| `fcd56ab09` | retry transient checkpoint read races |
| `f99e3a84c` | inventory checkpoint retry query ownership |
| `6d4f48933` | complete configuration ambiguity recovery |
| `4e11e3550` | strengthen configuration ambiguity regression |
| `5711097b` | expand generated portable replay corpus |
| `74bfa335` | add generated offline replay acceptance vector |
| `6284e8d9` | expand generated portable graph corpus |

`ef3e412d5` (`wording in code-quality`) was committed while the earlier gate was
running. It changes only prose and whitespace in `docs/code-quality.md`. The
later `42c80525` whitespace cleanup, `a027640c` review refresh, `d0459d4d`
bounded-source review refresh, `8803a585` evidence pin, `ebc4f8a81`,
`7530a447`, and `f8568ff26` are documentation-only commits. `5711097b` is a
post-step-12 corpus/test-only revision: it adds deterministic generator
resealing helpers, generated portable vectors, the replay corpus assertion, and
evidence prose, without changing production replay/runtime behavior. `74bfa335`
adds a generated offline-fold acceptance vector derived from the production
zero-state export, and `6284e8d9` adds production-shaped semantic/audit cutoff
vectors plus shared-DAG and cycle graph vectors that exercise the real source
closure expander. These revisions remain corpus/test-only and do not change
production replay/runtime behavior. The corrected source gate below starts
after `6284e8d9`; this evidence refresh is documentation-only after that gate.

## Independent review checkpoints

| Checkpoint | Candidate | Review evidence | Result |
| --- | --- | --- | --- |
| Initial TT2 cutover | `d9e53a5e9` | `258059180` | FAIL; wallet/replay guards followed |
| Wallet projection guards | `774f0b742` | `40612039f` | FAIL; budget/signer/authority fixes followed |
| Retry/authority candidate | `4fc1baea0` | `16fe501a` | FAIL; replay trust/incarnation fixes followed |
| Registered-incarnation candidate | `55f1cafac` | `9bd6f7056` | PASS for that scope |
| Lint-clean candidate | `7f2a792af` | `76ce09085` | PASS for that scope |
| Current implementation candidate | `6284e8d926e2ff866eae94c714420050cfe29643` | independent implementation and post-gate evidence review completed on this exact revision; exact composed gate and focused evidence below | CONDITIONAL; residuals below |

## Focused and composed verification

All direct Rust tooling was run in the default Nix development shell. The
composed source gate was run once on the current implementation candidate,
without preceding it with separate composed check/test/test-db gates. The
focused checks and exact run are recorded below; the later evidence-only
commit does not alter the gated source tree.

| Check | Evidence |
| --- | --- |
| formatting | pass (`nix develop -c cargo fmt --all -- --check`) |
| workspace Clippy | pass (`-D warnings`) |
| metadata and SQLx offline | pass |
| portable replay leaf | pass; `mfm-replay` unit corpus 9/9, including generated artifact and nested source-graph vectors |
| workspace nextest and doctests | pass |
| PostgreSQL SQLx check | pass |
| recoverability PostgreSQL v1 | pass |
| wallet-nonce PostgreSQL storage qualification | pass |
| structured EVM submission qualification | pass |
| Bitcoin parity | pass |
| closing source revision | pass |

Current exact composed gate:

```text
nix run .#ci
source: 6284e8d926e2ff866eae94c714420050cfe29643
run id: run-1862293-1785966011921687322
result: ok — 13 passed, 0 failed in 1938.53s
structured EVM submission: ok in 1259.57s
```

The run also observed the PostgreSQL recoverability leaf and wallet-nonce
qualification as passing, plus Bitcoin parity and closing-source-revision.
The closing-source-revision leaf observed the pinned `6284e8d9` source; the
evidence refresh after the run is documentation-only.
The portable leaf consumed the tracked generated corpus and passed 9/9 replay
tests; deterministic regeneration was checked separately with
`python3 contracts/recoverability/generate.py`, retaining corpus metadata of
1,115,887 bytes, SHA-256
`aaed5cbea0feb3650d8e3d912f6b8f3f688230427619185defe77adb12ac9b80`, 15
artifact vectors, and two source-graph vectors.
The final independent checkpoint reviewed the exact run summary and closing
source leaf: all 13 nodes succeeded, workspace nextest reported 590 passed and
1 skipped, and the closing leaf printed the full candidate hash above. It also
confirmed that generator determinism is separate evidence rather than a
hidden step inside the portable corpus leaf.

Earlier exact composed gate (historical; d67 implementation):

```text
nix run .#ci
source: d67a3bc3aac44d6820ee47a9235a8bfdbb3a6ed2
run id: run-1587720-1785947457045693112
result: ok — 13 passed, 0 failed in 1738.58s
structured EVM submission: ok in 1229.82s
```

The immediately preceding 6d implementation gate is retained as historical
evidence:

```text
nix run .#ci
source: 6d4f4893371a2e48dbacb0ed5f5959547306c029
run id: run-1760799-1785957370345872343
result: ok — 13 passed, 0 failed in 1802.02s
structured EVM submission: ok in 1261.66s
```

The earlier candidate diagnostics included a same-stream configuration race
and a transient commit-before-checkpoint-ack read race. `e68aca910` aligns the
idempotent predecessor digest with the checkpoint's canonical-revision head,
and `fcd56ab09` adds bounded retries only for the resulting transient
`InvalidHistory` reads. `f99e3a84c` updates the reviewed SQL inventory
ownership predicate for those wrappers; it does not change SQL text or query
semantics. `6d4f48933` extends the bounded read classification to application
configuration resolution and reconnects ambiguous appends through an identical
canonical retry. `4e11e3550` strengthens the durable-row/unknown-acknowledgement
regression and asserts one retained row. `5711097b`, `74bfa335`, and
`6284e8d9` add the generated portable corpus, an offline-fold acceptance
vector, production-shaped cutoff vectors, and nested source-graph vectors; the
final run above exercises that reviewed source.

Additional focused evidence on the current implementation sequence includes
six bounded recursive source-closure tests, nine portable replay tests, two
portable source/fact-route bound regressions, exact frame and total-byte
limit/one-over checks, root target/tenant trust tamper denials, serialized
recursive-prefix tamper cases, the generated 15-vector portable artifact
corpus and two nested source-graph vectors, application root/dependency
zero-byte denials, and online/offline
projection byte parity. The managed PostgreSQL recursive parity fixture passed
17/17 on the corresponding current storage sequence, and the composed gate
passed the configured-value same-stream append race after the checkpoint
predecessor and retry fixes. The focused store suite now passes 33/33,
including bounded configuration-reader retry and identical-append ambiguity
recovery regressions. The historical composed gate includes the source-bound
fix in `d67a3bc3`; the current run's closing-source-revision leaf observed
`6284e8d9`.

## TT2 disposition at the current candidate

“Conditional” means implementation exists but the plan-required production
proof or deployment evidence is still missing; it is not a waiver.

| Item | Disposition | Current evidence or residual |
| --- | --- | --- |
| AUTH-01 | Closed | Runtime/physical access is marker-sealed and API-surface tests reject ordinary implementations. |
| AUTH-02 | Closed | PostgreSQL session issuance is bound to the external deployment authority and private credentials. |
| STORE-01 | Conditional | External target/checkpoint trust is explicit and exercised by fakes; concrete production trust integration is absent. |
| STORE-02 | Conditional | Snapshot/head checks and race tests pass; deterministic cross-process acknowledgement/fault matrix is absent. |
| STORE-03 | Closed | Fresh loads compare folded prefixes with indexed heads and reject rewind/divergence. |
| STORE-04 | Closed | Aborted-transaction classification was removed; raced append identities are retried and reconciled. |
| STORE-05 | Conditional | Shared canonical ingress exists; a complete exact memory/PostgreSQL acceptance corpus is not independently reproduced. |
| STORE-06 | Conditional | SQL inventory and bounded producer-prefix path exist; generic/builder query inventory and long-history bound proof remain. |
| STORE-07 | Closed | Prior fact routes use a unique producer-prefix verification and bounded discovery. |
| EVM-01 | Closed | Fresh production keystore signing/broadcast qualification passed in the current composed gate. |
| EVM-02 | Closed | Recovery reobserves chain state before any retained-candidate broadcast. |
| EVM-03 | Closed | Replacement evidence is producer-authorized and bound to the retained prefix/permit. |
| EVM-04 | Closed | Definite failure reconciles final authoritative status before closure. |
| EVM-05 | Closed | Stable intent and separate semantic digest/conflict behavior are covered by domain tests. |
| EVM-06 | Closed | Release currentness resolves registered historical incarnations and promotion paths. |
| EVM-07 | Conditional | Status/projection paths are bounded in the current design; lifetime/long-history cost proof is not complete. |
| EVM-08 | Closed | Retained signer integrity failures remain integrity faults; no availability downgrade path is accepted. |
| EVM-09 | Conditional | Completion retains terminal witnesses and activated prefixes, but a full independent public-result closure audit is incomplete. |
| EVM-10 | Closed | `u64::MAX` is rejected before pending observation and persisted wallet mutation. |
| EVM-11 | Closed | Pending-floor route/policy and configured semantics are authority-qualified before mutation. |
| EVM-12 | Conditional | Current release/restart qualification passes; injected crash/ambiguity/replacement/scale matrices are incomplete. |
| REPLAY-01 | Conditional | Recursive source proof, fixation, trust, limits, and serialized source-prefix tamper coverage are implemented; live multi-hop app proof remains. |
| REPLAY-02 | Closed | Reproduction/current-history comparison and the old capability surface are deleted. |
| REPLAY-03 | Conditional | Exact semantic cutoff and kind-aware authorization cutoff are enforced; synthesized semantic/audit suffix vectors pass strict reject/accept behavior, but no genuinely valid production later-audit artifact fixture exists yet. |
| REPLAY-04 | Closed | Exact frame/total limits, one-over failures, large-frame and many-small-frame paths pass. |
| REPLAY-05 | Conditional | Generated schema vectors, a 15-vector portable artifact corpus, two nested source-graph vectors, and the generated offline-fold acceptance leaf pass; a genuinely valid later-audit artifact remains. |
| APP-01 | Conditional | Purpose-specific evidence types and redaction exist; complete data-isolation proof is not independent. |
| APP-02 | Conditional | Flattened recursive closure is kind-aware, fixed-point, graph-checked, and retains principal/grant/decision references; app unit tests prove root and dependency zero-byte denial, while live production multi-hop proof remains. |
| SEC-01 | Conditional | Protected key movement and zeroization checks exist; same-allocation lifetime is not established by Rust move semantics. |
| QUALITY-01 | Conditional | Duplicate runtime/replay paths were removed; core ownership/hand-written LOC remains concentrated. |
| VERIFY-01 | Conditional | Broad and focused gates pass, but the required complete proof matrix is not present. |
| PROCESS-01 | Conditional | This review now preserves residuals and exact provenance; strict PASS is withheld until the residual proof/deployment work closes. |

## Baseline and deletion audits

The named regression matrix and baseline disposition are recorded in the
independent review. The new portable, trust, recursive, fresh-keystore, and
configuration-race regressions were added after `a4dada89`; the baseline has
no corresponding tests to execute, so their baseline result is *not
applicable*, not an inferred failure. Existing API rejection and replay tests
retain their historical baseline behavior.

The source deletion audit was run with `rg` over `crates`, `bin`, and `tests`.
There are no production matches for the removed authority names, raw session
openers, self-fence permits, repeated-prefix implementation, or monolithic
portable encoder. The only `compare_current` matches are deliberate negative
parse/transport tests. Old plan and historical review documents naturally
retain the names as audit vocabulary.

Secret review found no new secret logging or persistence in manifests, events,
artifacts, portable frames, CLI/API output, or error details; the redaction and
keystore lanes remain part of the composed gate.

## Strict-plan residuals

The following are the concrete blockers to an unconditional §13 PASS:

1. add a genuinely valid production semantic export with a later audit suffix
   that can be folded offline, and a live application multi-hop export
   integration. The current generated suffix vectors prove strict semantic/audit
   cutoff behavior on synthesized envelopes; the generated shared-DAG and cycle
   vectors prove the source-closure algorithm, but neither substitutes for that
   real later-history artifact or live application fixture;
2. provide concrete production retained-release/checkpoint trust implementations;
3. complete the EVM injected-kill and cross-process PostgreSQL fault/acknowledgement
   matrices;
4. substantiate same-allocation key lifetime and long-history cost/LOC ownership
   evidence; and
5. close the remaining STORE-05/06, EVM-07/09, APP-01, QUALITY-01, and VERIFY-01
   production proof matrices.

Until those items are resolved or the normative plan is deliberately amended,
the honest disposition is **CONDITIONAL / INCOMPLETE**, not PASS.
