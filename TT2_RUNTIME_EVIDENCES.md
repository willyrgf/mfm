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
- The explicit offline fold seam (`RawRunHistory` plus concrete trust) now
  returns an opaque `OfflineVerifiedRun` containing only recorded status and
  bounded export metadata. The full verified cursor and object graph remain
  store-private; the complete runtime/audit/replay/export isolation matrix is
  still unverified.
- The store-shaped observed-read audit stream is now retained as a generated
  corpus artifact with an exact generator-owned hex source. The corpus test
  regenerates that artifact from the real fixture, verifies offline parity,
  and rejects the same later suffix when relabeled Semantic. Live
  Postgres/application replay remains separate evidence.
- The keystore boundary now has direct witnesses for both truncated and
  oversized ciphertext, authentication/tag/AAD failures, authenticated
  invalid-key material, and a valid key with the wrong public/account identity.
  External termination, OOM, and other resource-failure behavior remains
  outside package-level witness coverage.
- The shared configuration append boundary now rejects an oversized serialized
  revision before backend dispatch. A managed 18-test PostgreSQL qualification
  compares memory and PostgreSQL on positive, exact-limit, one-byte-over,
  stale-predecessor, and idempotent-replay vectors; the broader generated and
  scale corpus remains unverified.

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
| `82e474ca` | bound wallet status to current projection |
| `61bf2091` | witness decrypt failure cleanup |
| `4fd1e757` | cover decrypt aad cleanup |
| `50c8cac1` | rehydrate completion from closure |
| `afec8457` | prove closure-alone multi-candidate reload |
| `d2a39d7a` | witness post-decrypt key cleanup |
| `8b2e64ba` | witness failed decrypt handoff address |
| `2fc4afa8` | prove purpose information isolation |
| `1ef7d694` | retry transient structured history loads |
| `e7624406` | bound postgres snapshot retries |
| `bd98e8ca` | hide actionable frontier from purpose evidence |
| `a1a78b84` | seal offline replay fold boundary |
| `eb38ea44` | prove valid later audit suffix export |
| `1ca2f7cd` | retain observed audit artifact in replay corpus |
| `db1b7503` | witness malformed ciphertext and wrong key identity |
| `c832e5d8` | cover matching public key identity witness |
| `624af5b2` | align configuration parity with sealed run status |
| `7e467952` | expand sql inventory syntax fixtures |

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
production replay/runtime behavior. `82e474ca` is the bounded EVM projection
cutover: normal status no longer performs a lifetime reservation `COUNT/MAX`,
and its managed qualification adds a primary-key `EXPLAIN` regression plus a
current-frontier omission check. The corrected source gate below runs on
`82e474ca`; this evidence and contract-wording refresh is documentation-only
after that gate. `61bf2091` adds the shared production decrypt allocation
guard and failure cleanup witnesses, `4fd1e757` adds the AAD identity
substitution cleanup witness, `50c8cac1` adds closure-only completion
rehydration and canonical public projection, and `afec8457` proves those
closure APIs against a persisted two-candidate PostgreSQL completion row.
`d2a39d7a` adds the authenticated post-decrypt invalid-key cleanup witness;
`8b2e64ba` adds direct source-to-`SecureKey` handoff-pointer coverage on that
failure path and moves the test transfer witness after the ownership move.
`2fc4afa8` adds two application trybuild cases that reject public raw-record
enumeration and trace authorization-request access; the complete purpose
data-isolation proof remains conditional.

`624af5b2` moves the serialized configuration-revision byte bound into the
shared canonical append constructor, so memory and PostgreSQL cannot diverge at
backend dispatch. Its managed qualification adds exact and one-byte-over
serialized-revision vectors alongside positive, stale-predecessor, and
idempotent replay cases, and refreshes integration assertions to the sealed
`RunEvidenceStatus` API after the frontier cutover.

`7e467952` extends the AST inventory fixture corpus with generic scalar and
`query_as` calls, the checked `query_as!` macro, wrapped generic calls, and a
`QueryBuilder::push` fragment. The runtime inventory and fixture tests pass;
the broader query ownership and scale audit remains a separate residual.

Its managed inventory leaf also passed on the exact source tip:

```text
nix run .#run -- --task postgres-sql-inventory-check
run id: run-2113805-1785987174287207054
result: ok — 1 task passed in 2.42s
```

`1ef7d694` bounds configuration append ambiguity and transient configuration
reads at eight attempts. `e7624406` removes the generic store-level snapshot
retry introduced by that revision and keeps the bounded retry at the
PostgreSQL checkpoint-read boundary, avoiding nested retry multiplication while
preserving fail-closed persistent-history behavior. The focused configuration
library suite passes 7/7 and the exact managed recoverability task passes after
the cutover; cross-process acknowledgement-loss and injected-fault proof remain
conditional.

`bd98e8ca` replaces the public and recorded-replay purpose projections' full
`StructuredFrontier` with the shared, data-free `RunEvidenceStatus`. The app and
replay adapters consume only its redaction-safe tag, and the compile-fail
privacy matrix rejects frontier access for both products. `a1a78b84` then
removes the public `VerifiedStructuredRun` conversion seam: explicit offline
fold callers receive only an opaque `OfflineVerifiedRun` with recorded status
and bounded export metadata. Store (33/33), replay (9/9), and app (31/31)
package tests remain green; the expanded 16-case application privacy matrix
also passes. The broader runtime/audit/replay/export isolation matrix is still
a verification residual.

`eb38ea44` adds a real store-shaped one-state Read history with committed
authorization and observed return batches. Its audit export folds offline and
matches the online recorded projection byte-for-byte; carrying the same later
physical suffix as a semantic export is rejected before fold. The full replay
library now passes 10/10, while the broader live multi-hop and retained corpus
proofs remain separate residuals.

`1ca2f7cd` retains the exact observed-read audit bytes in
`contracts/recoverability/v1/portable_store_observed_read_audit.hex`, makes
the generator emit acceptance and semantic-rejection vectors from those bytes,
and extends the corpus test to rebuild the artifact through the real store
fixture and compare online/offline projection bytes. The generated corpus now
contains 17 portable artifact vectors; live application and multi-process
replay remain unverified.

## Independent review checkpoints

| Checkpoint | Candidate | Review evidence | Result |
| --- | --- | --- | --- |
| Initial TT2 cutover | `d9e53a5e9` | `258059180` | FAIL; wallet/replay guards followed |
| Wallet projection guards | `774f0b742` | `40612039f` | FAIL; budget/signer/authority fixes followed |
| Retry/authority candidate | `4fc1baea0` | `16fe501a` | FAIL; replay trust/incarnation fixes followed |
| Registered-incarnation candidate | `55f1cafac` | `9bd6f7056` | PASS for that scope |
| Lint-clean candidate | `7f2a792af` | `76ce09085` | PASS for that scope |
| Generated portable graph candidate | `6284e8d926e2ff866eae94c714420050cfe29643` | independent implementation and post-gate evidence review completed on this exact revision; superseded by the bounded projection cutover below | CONDITIONAL; residuals below |
| Bounded wallet projection candidate | `82e474caf83bea3338da116e5735f06367f80742` | independent implementation review and exact composed gate below cover this revision | CONDITIONAL; residuals below |
| Closure/decrypt focused candidate | `afec8457` | focused decrypt cleanup, closure-only rehydration, and persisted two-candidate managed qualification reviewed on exact revision | CONDITIONAL; offline/public-result and broader fault residuals below |
| Retry-boundary correction | `e7624406` | exact configuration/recoverability checks and independent review confirm one bounded PostgreSQL snapshot-retry owner | PASS for retry scope; global residuals below |
| Purpose-status isolation cutover | `bd98e8ca` | full frontier removed from public/recorded products; 14-case compile-fail matrix and package regressions pass | PASS for API scope; broader isolation residuals below |
| Offline-fold boundary cutover | `a1a78b84` | opaque `OfflineVerifiedRun` replaces the public full-fold conversion seam; 16-case compile-fail matrix, store/replay package tests, and portable corpus pass | PASS for API scope (independent review); broader isolation residuals below |
| Later-audit suffix proof | `eb38ea44` | real store-shaped authorization/observation suffix: audit offline parity passes and semantic carry-forward is rejected; replay 10/10 | PASS for implementation scope; retained/live residuals below |

## Focused and composed verification

All direct Rust tooling was run in the default Nix development shell. The
historical composed source gate was run once on `82e474ca`, without preceding
it with separate composed check/test/test-db gates. The focused checks and
later exact runs are recorded below; documentation-only refreshes do not alter
that gated source tree, while later focused code revisions are evidenced on
their own exact revisions.

| Check | Evidence |
| --- | --- |
| formatting | pass (`nix develop -c cargo fmt --all -- --check`) |
| workspace Clippy | pass (`-D warnings`) |
| metadata and SQLx offline | pass |
| portable replay leaf | pass; `mfm-replay` unit corpus 10/10, including the retained observed-read artifact and nested source-graph vectors |
| workspace nextest and doctests | pass |
| PostgreSQL SQLx check | pass |
| recoverability PostgreSQL v1 | pass |
| configuration memory/PostgreSQL parity | pass; managed recoverability task `run-2114883-1785987374603393785`, 18/18 structured-history tests |
| PostgreSQL SQL inventory syntax fixtures | pass; `mfm-storage-postgres` inventory tests 2/2 |
| wallet-nonce PostgreSQL storage qualification | pass |
| structured EVM submission qualification | pass |
| Bitcoin parity | pass |
| closing source revision | pass |

Current exact composed gate:

```text
nix run .#ci
source: 82e474caf83bea3338da116e5735f06367f80742
run id: run-1900515-1785969180314881458
result: ok — 13 passed, 0 failed in 1811.86s
structured EVM submission: ok in 1322.75s
```

The run also observed the PostgreSQL recoverability leaf and wallet-nonce
qualification as passing, plus Bitcoin parity and closing-source-revision.
The closing-source-revision leaf observed the pinned `82e474ca` source; the
evidence refresh after the run is documentation-only.
The historical portable leaf consumed the then-tracked generated corpus and
passed 9/9 replay tests; deterministic regeneration was checked separately with
`python3 contracts/recoverability/generate.py`, retaining corpus metadata of
1,115,887 bytes, SHA-256
`aaed5cbea0feb3650d8e3d912f6b8f3f688230427619185defe77adb12ac9b80`, 15
artifact vectors, and two source-graph vectors.
The final independent checkpoint reviewed the exact run summary and closing
source leaf for `82e474ca`: all 13 nodes succeeded, workspace nextest reported 590 passed and
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
six bounded recursive source-closure tests, ten portable replay tests, two
portable source/fact-route bound regressions, exact frame and total-byte
limit/one-over checks, root target/tenant trust tamper denials, serialized
recursive-prefix tamper cases, the generated 17-vector portable artifact
corpus (including the retained observed-read audit stream) and two nested
source-graph vectors, application root/dependency
zero-byte denials, bounded wallet projection `EXPLAIN` coverage, and online/offline
projection byte parity. The managed PostgreSQL recursive parity fixture passed
17/17 on the corresponding current storage sequence, and the composed gate
passed the configured-value same-stream append race after the checkpoint
predecessor and retry fixes. The focused store suite now passes 33/33,
including bounded configuration-reader retry and identical-append ambiguity
recovery regressions. The historical composed gate includes the source-bound
fix in `d67a3bc3`; the current run's closing-source-revision leaf observed
`82e474ca`.

The current managed PostgreSQL qualification ran from clean source tip
`a2664734`:

```text
nix run .#run -- --task recoverability-postgres-v1
run id: run-2114883-1785987374603393785
result: ok — 18 structured-history tests passed, 0 failed in 31.79s
```

The new parity test exercises one positive append, an exact serialized
`MAX_CONFIGURATION_REVISION_BYTES` revision, a one-byte-over revision, stale
predecessor rejection, idempotent replay, and final reader parity against the
memory backend. The same managed run also rechecked the pre-existing race and
fresh-process/role/schema cases.

## TT2 disposition at the current candidate

“Conditional” means implementation exists but the plan-required production
proof or deployment evidence is still missing; it is not a waiver.

| Item | Disposition | Current evidence or residual |
| --- | --- | --- |
| AUTH-01 | Closed | Runtime/physical access is marker-sealed and API-surface tests reject ordinary implementations. |
| AUTH-02 | Closed | PostgreSQL session issuance is bound to the external deployment authority and private credentials. |
| STORE-01 | Conditional | External target/checkpoint trust is explicit and exercised by fakes; concrete production trust integration is absent. |
| STORE-02 | Conditional | Snapshot/head checks and recoverability races pass with one bounded eight-attempt PostgreSQL checkpoint-read owner; deterministic cross-process acknowledgement/fault matrix is absent. |
| STORE-03 | Closed | Fresh loads compare folded prefixes with indexed heads and reject rewind/divergence. |
| STORE-04 | Closed | Aborted-transaction classification was removed; raced append identities are retried and reconciled. |
| STORE-05 | Conditional | Shared canonical ingress now bounds serialized configuration revisions before backend dispatch. Managed memory/PostgreSQL vectors cover positive, exact-limit, one-byte-over, stale-predecessor, and idempotent replay cases; the complete generated/scale acceptance corpus remains unverified. |
| STORE-06 | Conditional | The AST inventory covers runtime calls, aliases, generic scalar/query-as forms, checked macros, wrapped helpers, and QueryBuilder fragments with 2/2 focused tests; a full independent query ownership/scale audit remains. |
| STORE-07 | Closed | Prior fact routes use a unique producer-prefix verification and bounded discovery. |
| EVM-01 | Closed | Fresh production keystore signing/broadcast qualification passed in the current composed gate. |
| EVM-02 | Closed | Recovery reobserves chain state before any retained-candidate broadcast. |
| EVM-03 | Closed | Replacement evidence is producer-authorized and bound to the retained prefix/permit. |
| EVM-04 | Closed | Definite failure reconciles final authoritative status before closure. |
| EVM-05 | Closed | Stable intent and separate semantic digest/conflict behavior are covered by domain tests. |
| EVM-06 | Closed | Release currentness resolves registered historical incarnations and promotion paths. |
| EVM-07 | Conditional | Normal status now reads the maintained domain projection, exact frontier, and bounded candidate prefix without lifetime reservation `COUNT/MAX`; the primary-key `EXPLAIN` regression passes, while the long-history query-count/latency matrix remains. |
| EVM-08 | Closed | Retained signer integrity failures remain integrity faults; no availability downgrade path is accepted. |
| EVM-09 | Conditional | Completion retains and validates typed preimages, including per-candidate permits bound to the retained prefix/ordinal and reservation observation rounds; serialized outer-value roundtrip, hostile tamper tests, and persisted two-candidate closure-only reload/public projection pass. An independent offline/public-result audit remains, including provider-attestation/signature verification without the storage verifier. |
| EVM-10 | Closed | `u64::MAX` is rejected before pending observation and persisted wallet mutation. |
| EVM-11 | Closed | Pending-floor route/policy and configured semantics are authority-qualified before mutation. |
| EVM-12 | Conditional | Current release/restart qualification passes; injected crash/ambiguity/replacement/scale matrices are incomplete. |
| REPLAY-01 | Conditional | Recursive source proof, fixation, trust, limits, and serialized source-prefix tamper coverage are implemented; live multi-hop app proof remains. |
| REPLAY-02 | Closed | Reproduction/current-history comparison and the old capability surface are deleted. |
| REPLAY-03 | Conditional | Exact semantic cutoff and kind-aware authorization cutoff are enforced; the retained store-shaped authorization/observation artifact folds as an audit export with byte-identical offline parity, while carrying it as a semantic export is rejected. Live production evidence remains. |
| REPLAY-04 | Closed | Exact frame/total limits, one-over failures, large-frame and many-small-frame paths pass. |
| REPLAY-05 | Conditional | Generated schema vectors, a 17-vector portable artifact corpus including retained observed-read audit bytes, two nested source-graph vectors, the generated offline-fold acceptance leaf, and online/offline parity pass; the complete live schema/corpus matrix remains. |
| APP-01 | Conditional | Public, recorded-replay, and offline replay products now expose only fold-derived status plus bounded export metadata; the 16-case application trybuild matrix rejects raw-record, frontier, trace authorization-request, and full verified-run access, while complete runtime/audit/replay/export isolation proof remains outstanding. |
| APP-02 | Conditional | Flattened recursive closure is kind-aware, fixed-point, graph-checked, and retains principal/grant/decision references; app unit tests prove root and dependency zero-byte denial, while live production multi-hop proof remains. |
| SEC-01 | Conditional | The shared production decrypt guard and witnesses cover one protected heap allocation, source-to-`SecureKey` handoff, cleanup on success, truncated and oversized ciphertext, ciphertext/tag/AAD authentication failure, injected unwind, authenticated post-decrypt invalid-key rejection, and explicit rejection of a valid key with the wrong public/account identity; external termination/OOM/resource-failure classes remain. |
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

1. add a live application multi-hop export integration. The store-shaped
   observed-read fixture and the retained generated artifact now prove the real
   audit suffix and semantic cutoff behavior; the generated shared-DAG and
   cycle vectors prove the source-closure algorithm, but live application
   replay remains;
2. provide concrete production retained-release/checkpoint trust implementations;
3. complete the EVM injected-kill and cross-process PostgreSQL fault/acknowledgement
   matrices;
4. cover external termination, OOM, and resource-failure classes in the
   keystore boundary while retaining the long-history cost/LOC ownership
   evidence; and
5. close the remaining STORE-05/06, EVM-07/09, APP-01, QUALITY-01, and VERIFY-01
   production proof matrices.

Until those items are resolved or the normative plan is deliberately amended,
the honest disposition is **CONDITIONAL / INCOMPLETE**, not PASS.

## Focused follow-up after the bounded projection candidate

This append-only entry records the next implementation/proof revisions. The
historical composed gate above remains pinned to `82e474caf`; these focused
checks do not retroactively turn that gate into a gate for the later tree.

| Revision | Subject | Evidence |
| --- | --- | --- |
| `5bebfb72` | `prove bounded wallet reads across long history` | managed 64-reservation history and proxy cardinality baseline |
| `4ab49f7b` | `harden long-history wallet query proof` | Q/P SQL capture, 64 reserve/activate/complete operations, mutation count/text checks |
| `d7454e37` | `cover final wallet status query` | final-status Q/P slice rejects lifetime reservation aggregates |
| `9390503c` | `prove production key allocation continuity` | decrypt-to-sign same protected heap allocation and cleanup witness |
| `56c533ae` | `extend wallet query plan proof` | post-history `EXPLAIN (ANALYZE)` checks for projection, reservation, candidate, completion, and frontier paths |
| `dec2f0c4` | `prove wallet completion closure reload` | serialized completion closure revalidates every preimage and preserves public-result bytes |
| `266d6889` | `accept bounded wallet projection scan` | one-row domain projection plan is accepted explicitly; multi-row exact-key index checks remain strict |
| `ed7b341e` | `bind completion closure permits` | exact per-candidate permit/ordinal and reservation observation-round bindings; hostile tamper tests; independent review found no implementation gap |
| `61bf2091` | `witness decrypt failure cleanup` | shared production decrypt guard witnesses cleanup on auth failure, bounded rejection, and injected unwind; keystore package and Clippy pass |
| `4fd1e757` | `cover decrypt aad cleanup` | substituted entry identity reaches the shared decrypt helper and zeroizes without ownership transfer; decrypt-focused suite 7/7 |
| `50c8cac1` | `rehydrate completion from closure` | closure-only rehydration reconstructs outer fields, validates every retained preimage, and projects canonical public-result bytes |
| `afec8457` | `prove closure-alone multi-candidate reload` | persisted two-candidate completion reload, closure-only rehydration, and canonical public projection; exact managed run and independent review pass |
| `d2a39d7a` | `witness post-decrypt key cleanup` | authenticated invalid-key payload reaches post-decrypt identity rejection and zeroized cleanup; focused decrypt suite 8/8 |
| `8b2e64ba` | `witness failed decrypt handoff address` | source-to-`SecureKey` handoff pointer equality and cleanup are directly asserted on post-decrypt rejection; full keystore and Clippy pass |
| `2fc4afa8` | `prove purpose information isolation` | application trybuild privacy suite passes all 12 cases, including public raw-record enumeration and trace authorization-request accessor denial |

The corrected exact managed leaf ran after `266d6889` with no intervening
commits:

```text
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
source: 266d6889
run id: run-1977166-1785976039045273113
result: ok — 1 task, 4 tests passed, 0 failed in 660.48s
```

Its real-history probe keeps normal-status and reserve/activate/complete
statement counts constant after 64 completed reservations, captures frontend
Q/P SQL text for baseline/mutation/final-status aggregate rejection, and
checks `EXPLAIN (ANALYZE)` indexed one-row plans for the frontier, exact
reservation, candidate-prefix, and completion keys. The one-row domain
projection is allowed to use a bounded sequential plan. The superseded
diagnostic `run-1971258-1785975345382628345`
failed only because the first version incorrectly required an index for the
one-row domain projection; PostgreSQL selected a bounded sequential plan, and
`266d6889` records that valid case.

The latest exact managed leaf ran on `afec8457` with no intervening commits:

```text
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
source: afec8457
run id: run-2003174-1785978540996350273
result: ok — 1 task, 4 tests passed, 0 failed in 671.76s
```

Its long SQL test reloads the persisted completion through both authorities,
confirms two retained candidates, then rehydrates and projects from the
serialized recovery closure alone. Independent review of that exact revision
confirmed the persisted multi-candidate evidence and found no implementation
gap. The remaining EVM-09 scope is an independent offline/public-result audit,
including provider-attestation/signature verification without the storage
verifier.

Additional current-tree leaves:

```text
nix run .#run -- --task postgres-sql-inventory-check
run id: run-1981332-1785976730139548702 — ok, 1/1 task in 2.67s
nix run .#run -- --task postgres-sqlx-offline-check
run id: run-1981529-1785976737932619794 — ok, 1/1 task in 5.07s
nix develop -c cargo test -p mfm-keystore — 91 unit tests + 9 doctests passed on `8b2e64ba`
nix develop -c cargo test -p mfm-keystore decrypt_ -- --nocapture — 8 passed on `8b2e64ba`
nix develop -c cargo test -p mfm-evm completed_wallet_nonce_retains_rehashable_public_recovery_closure -- --nocapture — 1 passed
nix develop -c cargo test -p mfm-evm — 47 unit tests + signing UI trybuild + 2 doctests passed on `50c8cac1`
nix develop -c cargo clippy -p mfm-keystore --all-targets -- -D warnings — pass on `8b2e64ba`
nix develop -c cargo fmt --all -- --check — pass on `e7624406`
nix develop -c cargo test -p mfm-store --lib configuration -- --nocapture — 7 passed on `e7624406`
nix develop -c cargo test -p mfm-store --lib — 33 passed on `e7624406`
nix run .#run -- --task recoverability-postgres-v1 — run id `run-2046822-1785981827251476177`, 1/1 passed on `e7624406`
nix develop -c cargo test -p mfm-app --test application-privacy-ui — 14/14 passed on `bd98e8ca`
nix develop -c cargo test -p mfm-app --lib — 31 passed on `bd98e8ca`
nix develop -c cargo test -p mfm-replay --lib — 9 passed on `bd98e8ca`
nix run .#run -- --task portable-replay-corpus — run id `run-2057202-1785982481856354937`, 1/1 passed on `bd98e8ca`
nix run .#run -- --task postgres-sqlx-check — run id `run-2049878-1785982143970760651`, 1/1 passed on `bd98e8ca`
nix develop -c cargo test -p mfm-app --test application-privacy-ui — 16/16 passed on `a1a78b84`
nix develop -c cargo test -p mfm-store --lib — 33 passed on `a1a78b84`
nix develop -c cargo test -p mfm-replay --lib — 9 passed on `a1a78b84`
nix develop -c cargo fmt --all -- --check — pass on `a1a78b84`
nix run .#run -- --task portable-replay-corpus — run id `run-2069857-1785983372700769978`, 1/1 passed on `a1a78b84`
nix develop -c cargo test -p mfm-store --lib — 33 passed on `eb38ea44`
nix develop -c cargo test -p mfm-replay --lib — 10 passed on `eb38ea44`
nix develop -c cargo clippy -p mfm-replay --all-targets -- -D warnings — pass on `eb38ea44`
nix develop -c cargo fmt --all -- --check — pass on `eb38ea44`
nix develop -c cargo test -p mfm-replay generated_portable_artifact_corpus_round_trips -- --nocapture — pass on `1ca2f7cd`
nix develop -c cargo test -p mfm-replay --lib — 10 passed on `1ca2f7cd`
python3 contracts/recoverability/generate.py — deterministic regeneration pass on `1ca2f7cd`
nix develop -c cargo test -p mfm-keystore decrypt_oversized_ciphertext_rejects_before_allocating_plaintext -- --nocapture — 1 passed on `c832e5d8`
nix develop -c cargo test -p mfm-keystore qualification_rejects_a_valid_key_with_wrong_public_and_account_identity -- --nocapture — 1 passed on `c832e5d8`
nix develop -c cargo test -p mfm-keystore — 93 unit tests + 9 doctests passed on `c832e5d8`
nix develop -c cargo clippy -p mfm-keystore --all-targets -- -D warnings — pass on `c832e5d8`
nix develop -c cargo fmt --all -- --check — pass on `c832e5d8`
```

Independent review of exact `ed7b341e` confirmed the permit/ordinal and
reservation observation-round bindings, and found no implementation gap in
the closure cutover. The later exact `afec8457` review closes the
persisted-closure-alone/later-run and multi-candidate reload evidence. Its
remaining proof scope is the independent offline/public-result audit.

Independent review of exact `e7624406` confirms that snapshot retry ownership
is singular and finite: the generic store wrapper is deleted and PostgreSQL's
checkpoint-read helper is the sole eight-attempt owner. The configuration
bound remains finite and fail-closed, although its eight attempts are per
nested loop and PostgreSQL retries persistent `InvalidHistory` until
exhaustion; these remain bounded resource/diagnostic residuals.

Independent review of exact `bd98e8ca` confirms no direct leak or contract
mismatch: public and recorded-replay products retain only `RunEvidenceStatus`,
the status mapping is exhaustive and annex-exact, and the 14-case compile-fail
and package matrix is green. The follow-up offline seam cutover at `a1a78b84`
is covered by the exact package tests above and keeps the full verified cursor
store-private. Independent review of exact `a1a78b84` found no implementation
gap: the full verified cursor, records, objects, and live bindings remain
store-private, while replay retains its bounded route/dependency checks. This
is PASS for the API scope; APP-01 remains Conditional until the complete
runtime, audit, replay, and export isolation matrix is independently
reproduced.

Independent review of exact `1ca2f7cd` confirms that the observed-read audit
artifact is generator-owned and retained, and that the corpus test rebuilds
the same bytes through `observed_read_export(203)` before running offline
verification and online/offline projection parity. REPLAY-03/05 now retain
the later-audit artifact provenance; live Postgres/application replay remains
outside this focused corpus proof.

The follow-up narrows, but does not eliminate, the residuals above. EVM-07
still lacks projection-index/scale, latency, and adversarial non-aggregate scan
evidence; EVM-09 still lacks the independent offline/public-result/provider
attestation audit; SEC-01 now has direct malformed-ciphertext and wrong-identity
witnesses, but still lacks external termination/OOM/resource-failure coverage;
and the
external trust, live multi-hop export,
cross-process/fault, quality, and complete verification residuals remain
Conditional.
