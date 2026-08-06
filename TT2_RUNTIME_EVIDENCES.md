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
  revision before backend dispatch. The clean managed 24-test PostgreSQL
  qualification compares memory and PostgreSQL on positive, exact-limit,
  one-byte-over, stale-predecessor, idempotent-replay, escaped JSON, an exact
  UTF-8 byte-boundary value and its one-byte-over rejection, 128 deterministic
  JSON shape values, a 128-revision sequential stream, and an exact canonical
  depth-limit value. The latest default-concurrency managed run also passes
  all 24 tests, including the same-stream race; two earlier attempts hit that
  race intermittently. The shared run-envelope validator now also has exact
  and one-byte-over `MAX_STORED_FRAME_BYTES` unit witnesses. One-level-over
  depth, float, duplicate-key, and malformed values are rejected before
  backend dispatch; the complete generated,
  hostile, and large-scale acceptance matrix remains unverified.

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
| `40051076` | cover unchecked sql query macros |
| `b0e1de8f` | expand configuration parity corpus |
| `0a4b02e1` | fix numeric ordering for configuration history rows |
| `fce1edca` | fix checkpoint preparation durability |
| `27edd8c6` | harden checkpoint sidecar arbitration |
| `cf74053f` | expand configuration boundary corpus |
| `77a06884` | exercise configuration acknowledgement recovery |
| `df61d1b9` | prove postgres snapshot interleavings |
| `dcb72608` | exercise postgres contention rollback |
| `f4fd9a9b` | exercise run acknowledgement recovery |
| `5f4be506` | measure fact producer fold bounds |
| `bb002b19` | scope fact scan counters |
| `2ac67ec9` | tighten bounded wallet query proof |
| `a3ad0e9a` | expand configuration acceptance corpus |
| `1edcf7bb` | close unchecked sql inventory aliases |
| `018a946a` | collect sql inventory imports before traversal |
| `836e9c31` | harden sql inventory scope collection |
| `2fea7802` | restore sql inventory builder scope |
| `8a902d03` | fail closed sql file macros |
| `6525fad6` | expand configuration corpus breadth |
| `9f61c39a` | prove canonical frame boundaries |

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
`QueryBuilder::push` fragment. Revision `40051076` adds the pinned SQLx
`query_unchecked!`, `query_as_unchecked!`, and `query_scalar_unchecked!`
macros. Revision `1edcf7bb` corrects their SQL argument positions and covers
direct, renamed-alias, and glob imports with dynamic rejection fixtures;
`018a946a` pre-collects file/module imports, and `836e9c31` adds block-scope
pre-collection and restores import state on scope exit. Revision `2fea7802`
restores `QueryBuilder` binding state across those same scopes. Revision
`8a902d03` recognizes every pinned SQLx `query_file*` form and rejects external
file SQL before literal ownership parsing. The clean Nixfied inventory leaf
`run-2236542-1786001395306859242` passes the source and fixture tests (2/2);
the broader query ownership and scale audit remains a separate residual.

Its managed inventory leaf also passed on the exact source tip:

```text
nix run .#run -- --task postgres-sql-inventory-check
run id: run-2113805-1785987174287207054
result: ok — 1 task passed in 2.42s
```

The final scoped inventory leaf ran on clean source tip `2fea7802`:

```text
nix run .#run -- --task postgres-sql-inventory-check
source: 2fea7802
run id: run-2232004-1786000886918900083
result: ok — 2 inventory tests passed, 0 failed in 1.95s
```

The query-file fail-closed inventory leaf ran on clean source tip `8a902d03`:

```text
nix run .#run -- --task postgres-sql-inventory-check
source: 8a902d03
run id: run-2236542-1786001395306859242
result: ok — 2 inventory tests passed, 0 failed in 2.01s
```

The current replay package re-audit also passes the complete portable boundary
suite on the clean evidence tip:

```text
nix develop -c cargo test -p mfm-replay --lib
source: 886fee09
result: ok — 10 passed, 0 failed
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
| Bounded wallet query-shape proof | `2ac67ec9` | managed 64-reservation history keeps Q/E counts constant, every captured wallet-history `SELECT` has an exact key/prefix, lifetime aggregates are rejected, and the domain primary-key path is available under `enable_seqscan = off`; run `run-2218000-1785998748489917662` | CONDITIONAL; production latency envelope remains |
| SQL inventory macro/import scope | `8a902d03` | independent review passes SQLx checked/unchecked and all six `query_file*` macro semantics, direct/renamed/glob imports, after-use file/module/block declarations, and import/`QueryBuilder` scope restoration; run `run-2236542-1786001395306859242` passes 2/2 | PASS for current inventory scope; broader ownership/scale audit remains |
| Configuration corpus breadth | `6525fad6` | serialized managed run expands memory/PostgreSQL parity to 128 generated shape values and 128 sequential revisions; all 24 tests pass with test threads serialized, while default-concurrency race attempts remain separate | CONDITIONAL; generated/hostile/production-scale corpus remains incomplete |
| Canonical frame boundary | `9f61c39a` | shared `mfm-store` ingress unit tests accept an exact `MAX_STORED_FRAME_BYTES` canonical frame and reject one byte over; the bound equals the canonical package and PostgreSQL schema limits | PASS for this exact/one-over boundary; broader run/object corpus remains conditional |
| Default-concurrency configuration qualification | `897ec4b8` | clean managed run `run-2246603-1786002530958490012` passes all 24 structured-history tests, including `configured_value_history_linearizes_same_stream_append_races`; two earlier attempts recorded intermittent 23/24 race witnesses | PASS for this run; repeatability and broader corpus remain conditional |

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
| configuration memory/PostgreSQL parity | pass; managed recoverability task `run-2138056-1785988778488975922`, 18/18 structured-history tests |
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
`0a4b02e1`:

```text
nix run .#run -- --task recoverability-postgres-v1
run id: run-2138056-1785988778488975922
result: ok — 18 structured-history tests passed, 0 failed in 31.91s
```

The new parity test exercises one positive append, an exact serialized
`MAX_CONFIGURATION_REVISION_BYTES` revision, a one-byte-over revision, stale
predecessor rejection, idempotent replay, 32 deterministic JSON shape vectors,
32 sequential successors, and final reader parity against the memory backend.
The numeric ordering fix keeps PostgreSQL's loaded prefix in sequence order
after the tenth successor. The same managed run also rechecked the pre-existing
race and fresh-process/role/schema cases.

The latest configuration corpus expansion ran from clean source tip
`a3ad0e9a`:

```text
nix run .#run -- --task recoverability-postgres-v1
source: a3ad0e9a
run id: run-2223304-1785999851634604306
result: ok — 24 structured-history tests passed, 0 failed in 38.75s
```

The parity fixture now covers 64 deterministic shape values and 64 sequential
successors across memory/PostgreSQL, plus an exact canonical JSON depth-limit
append. One-level-over depth, float, duplicate-key, and malformed inputs are
rejected by the shared canonical constructor before either backend receives an
append.

Revision `6525fad6` expands the same fixture to 128 deterministic values across
16 canonical shape families (including null, empty collections, negative/zero,
escaped, nested, variable payload, and unsorted-key forms) and a 128-revision
sequential stream. The serialized managed run uses
`RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1` and
passes all 24/24 tests; two default-concurrency attempts independently hit the
pre-existing same-stream race witness (23/24), so this is targeted corpus
evidence rather than a default-concurrency gate pass.

Revision `9f61c39a` adds shared ingress witnesses for an exact canonical frame
at `MAX_STORED_FRAME_BYTES` and a one-byte-over frame. The focused Nix
development test `mfm-store::structured::canonical_append` passes all 3/3
unit tests; the bound matches both `mfm_canonical::limits` and the PostgreSQL
`octet_length(canonical_json)` constraint.

The latest default-concurrency managed qualification ran from clean source
tip `897ec4b8` (documentation-only evidence refresh after implementation
revision `9f61c39a`):

```text
nix run .#run -- --task recoverability-postgres-v1
run id: run-2246603-1786002530958490012
result: ok — 24 structured-history tests passed, 0 failed in 43.42s
```

The separate release no-run integration compile was also rerun against the
same implementation source:

```text
nix develop -c cargo test --release -p mfm-integration-tests \
  --features parity-tests --test evm_postgres_submission --no-run
source: 0a4b02e1
result: pass
```

The checkpoint sidecar authority was then qualified on `fce1edca`:

```text
nix run .#run -- --task recoverability-postgres-v1
source: fce1edca
run id: run-2150431-1785990104705897798
result: ok — 18 structured-history tests passed, 0 failed in 32.08s

nix develop -c cargo test -p mfm-storage-postgres \
  --features test-support checkpoint --lib
source: fce1edca
result: ok — 9 checkpoint tests passed, 0 failed
```

The sidecar now persists every successful `Prepared` single- or multi-key
successor before returning. Fresh ledger instances reload that state, reject a
different successor, and acknowledge the exact prepared bytes after restart;
`acknowledge_many` also rejects unprepared successors. The sidecar remains a
test authority rather than a production deployment implementation.

The follow-up sidecar hardening was then qualified on `27edd8c6`:

```text
nix run .#run -- --task recoverability-postgres-v1
source: 27edd8c6
run id: run-2155505-1785990746657729092
result: ok — 18 structured-history tests passed, 0 failed in 32.05s

nix develop -c cargo test -p mfm-storage-postgres \
  --features test-support checkpoint --lib
source: 27edd8c6
result: ok — 10 checkpoint tests passed, 0 failed
```

The sidecar now uses an OS file lock and reloads persisted state before every
mutating validation; independently loaded workers reject a stale conflicting
successor instead of overwriting the durable slot. A complete injected
serialization/deadlock, acknowledgement-loss, promotion, and production
authority matrix remains unverified.

The configuration boundary corpus was then expanded and qualified on the clean
source tip `cf74053f`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: cf74053f
run id: run-2162922-1785991603848871458
result: ok — 18 structured-history tests passed, 0 failed in 31.72s
```

The new vectors compare memory and PostgreSQL on escaped JSON, an exact
UTF-8-sized serialized revision, and a one-byte-over UTF-8 revision. The same
qualification retains the existing positive/exact/one-over, stale,
idempotent, 32-shape, 32-successor, race, and fresh-process cases; broader
generated, hostile, and large-scale coverage remains conditional.

The acknowledgement-recovery and bounded-race follow-up was then qualified on
the clean source tip `77a06884`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: 77a06884
run id: run-2167617-1785992166392324915
result: ok — 19 structured-history tests passed, 0 failed in 31.76s
```

This run injects one committed-but-unknown configuration acknowledgement and
asserts exact retry, one durable row, and the recovered reader head. The
configuration retry loop also yields between its bounded attempts so a
committed SQL prefix and its external acknowledgement cannot be misclassified
as a permanent stale result. The broader acknowledgement, promotion, and
production-authority matrix remains unverified.

The snapshot interleaving follow-up was then qualified from clean source tip
`df61d1b9`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: df61d1b9
run id: run-2177250-1785993122646524325
result: ok — 21 structured-history tests passed, 0 failed in 32.49s
```

The new run and configuration barriers release the external fixation, append a
successor while the reader transaction remains repeatable-read, and assert that
the reader returns the old complete prefix. The subsequent read observes the
committed successor. This closes the deterministic snapshot-interleaving proof;
the broader acknowledgement, promotion, and production-authority matrix remains
unverified.

The contention rollback follow-up was then qualified from clean source tip
`dcb72608`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: dcb72608
run id: run-2190510-1785994272280747833
result: ok — 22 structured-history tests passed, 0 failed in 32.29s
```

The added test injects one PostgreSQL `40001` serialization failure and one
`40P01` deadlock failure at batch insertion. Each failed transaction leaves no
rows, is classified after rollback through a fresh transaction, and accepts the
same append request after the injected fault is removed. Cross-process
acknowledgement, promotion, and production-authority matrices remain
conditional.

The run acknowledgement-recovery follow-up was then qualified from clean
source tip `f4fd9a9b`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: f4fd9a9b
run id: run-2194313-1785994645494655426
result: ok — 23 structured-history tests passed, 0 failed in 32.33s
```

The run append path commits one batch while returning
`AcknowledgementUnknown`, retries the exact candidate, and resolves it as
`ExistingSame` with one durable row. Cross-process acknowledgement and
promotion matrices remain conditional.

The fact-scan work-bound follow-up was qualified from clean source tip
`bb002b19`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: bb002b19
run id: run-2206767-1785995910502340324
result: ok — 24 structured-history tests passed, 0 failed in 37.57s
```

The added same-producer fixture publishes two successive facts, then asserts
one dense publication page, one producer-prefix load, and three producer
batches folded for each scan invocation. Retries are counted separately and
still perform one prefix load per invocation; the counters are keyed by store
identity so parallel isolated schemas cannot contaminate the measurement. The
route and fold work therefore remain linear in unique retained producer history
rather than publication count squared.

The clean pre-fix source sequence also refreshed the source-local inventory and
portable corpus leaves (the checkpoint change does not touch either surface):

```text
nix run .#run -- --task postgres-sql-inventory-check
source: b38ae5c3
run id: run-2141603-1785989203587902728
result: ok — 2 inventory tests passed, 0 failed in 1.96s

nix run .#run -- --task portable-replay-corpus
source: b38ae5c3
run id: run-2144669-1785989584718916467
result: ok — 10 replay corpus tests passed, 0 failed in 21.14s
```

## TT2 disposition at the current candidate

“Conditional” means implementation exists but the plan-required production
proof or deployment evidence is still missing; it is not a waiver.

| Item | Disposition | Current evidence or residual |
| --- | --- | --- |
| AUTH-01 | Closed | Runtime/physical access is marker-sealed and API-surface tests reject ordinary implementations. |
| AUTH-02 | Closed | PostgreSQL session issuance is bound to the external deployment authority and private credentials. |
| STORE-01 | Conditional | External target/checkpoint trust is explicit and exercised by fakes; concrete production trust integration is absent. |
| STORE-02 | Conditional | Snapshot/head checks, deterministic run/configuration interleavings, Prepared restart, strict acknowledgement, stale-worker sidecar arbitration, run/configuration committed-but-unknown acknowledgement recovery, and injected `40001`/`40P01` rollback classification pass; cross-process acknowledgement, promotion, and production-authority matrices remain absent. |
| STORE-03 | Closed | Fresh loads compare folded prefixes with indexed heads and reject rewind/divergence. |
| STORE-04 | Closed | Aborted-transaction classification was removed; raced append identities are retried and reconciled. |
| STORE-05 | Conditional | Shared canonical ingress bounds serialized configuration revisions and run frames before backend dispatch. Revision `6525fad6` expands the managed memory/PostgreSQL parity to 128 generated values across 16 canonical shape families and a 128-revision stream, retaining positive, exact-limit, one-byte-over, stale-predecessor, idempotent replay, escaped JSON, exact UTF-8 boundary, exact depth-limit, one-level-over depth, float, duplicate-key, and malformed vectors; serialized run `run-2240964-1786001827263556370` passes 24/24. Revision `9f61c39a` adds exact/one-byte-over shared frame witnesses, and the latest default-concurrency run `run-2246603-1786002530958490012` passes 24/24. Two earlier attempts hit the same-stream race intermittently; the complete generated, hostile, and production-scale acceptance matrix remains unverified. |
| STORE-06 | Conditional | The AST inventory covers runtime calls, aliases, generic scalar/query-as forms, checked and `_unchecked` macros, all six `query_file*` forms (fail closed), wrapped helpers, QueryBuilder fragments, direct/renamed/glob imports, after-use declarations, and nested file/module/block scopes; independent review and managed run `run-2236542-1786001395306859242` pass 2/2. A full independent query ownership/scale audit remains. |
| STORE-07 | Closed | A managed two-publication same-producer witness asserts one dense page, one producer-prefix load, and three folded producer batches per scan invocation; store-scoped counters prevent isolated parallel schemas from contaminating the proof, and bounded discovery/session limits remain enforced. |
| EVM-01 | Closed | Fresh production keystore signing/broadcast qualification passed in the current composed gate. |
| EVM-02 | Closed | Recovery reobserves chain state before any retained-candidate broadcast. |
| EVM-03 | Closed | Replacement evidence is producer-authorized and bound to the retained prefix/permit. |
| EVM-04 | Closed | Definite failure reconciles final authoritative status before closure. |
| EVM-05 | Closed | Stable intent and separate semantic digest/conflict behavior are covered by domain tests. |
| EVM-06 | Closed | Release currentness resolves registered historical incarnations and promotion paths. |
| EVM-07 | Conditional | Managed run `run-2218000-1785998748489917662` keeps status and reserve/activate/complete Q/E counts constant after 64 completed reservations, rejects captured Q/P lifetime `COUNT/MAX`, requires exact key/prefix predicates for every wallet-history `SELECT`, and verifies the domain primary-key path under `enable_seqscan = off`; a production latency envelope remains. |
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
5. close the remaining STORE-05/06, the EVM-07 latency envelope, EVM-09, APP-01,
   QUALITY-01, and VERIFY-01
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
| `2ac67ec9` | `tighten bounded wallet query proof` | managed 64-reservation history keeps Q/E counts constant; exact wallet-history query predicates, lifetime-aggregate rejection, and strict projection-index availability pass in `run-2218000-1785998748489917662` |
| `a3ad0e9a` | `expand configuration acceptance corpus` | managed memory/PostgreSQL parity grows to 64 shapes and 64 sequential revisions; exact canonical depth boundary and hostile pre-dispatch rejections pass in `run-2223304-1785999851634604306` |
| `6525fad6` | `expand configuration corpus breadth` | serialized managed memory/PostgreSQL parity grows to 128 values across 16 shape families and 128 sequential revisions; `run-2240964-1786001827263556370` passes 24/24, while default-thread race attempts remain separately recorded |
| `9f61c39a` | `prove canonical frame boundaries` | shared canonical envelope tests accept exact `MAX_STORED_FRAME_BYTES` and reject one-byte-over input; the value matches the canonical limits and PostgreSQL schema |
| `897ec4b8` | `record default configuration qualification` | latest default-concurrency managed run `run-2246603-1786002530958490012` passes all 24 structured-history tests; earlier intermittent race witnesses remain provenance |
| `40051076` | `cover unchecked sql query macros` | AST inventory adds pinned SQLx checked/unchecked macro names |
| `1edcf7bb` | `close unchecked sql inventory aliases` | SQL argument positions match SQLx 0.9; direct, renamed-alias, and glob macro paths plus dynamic rejection fixtures pass |
| `018a946a` | `collect sql inventory imports before traversal` | after-use file/module alias and glob declarations are pre-collected; managed inventory remains 2/2 |
| `836e9c31` | `harden sql inventory scope collection` | block-local imports are pre-collected and file/module/block import state is restored; managed inventory run `run-2230514-1786000751269593986` passes 2/2 |
| `2fea7802` | `restore sql inventory builder scope` | `QueryBuilder` binding state is restored across file/module/block scopes; managed inventory run `run-2232004-1786000886918900083` passes 2/2 |
| `8a902d03` | `fail closed sql file macros` | all six pinned SQLx `query_file*` forms are recognized and rejected as external-file SQL; managed inventory run `run-2236542-1786001395306859242` passes 2/2 |

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

The bounded wallet query-shape follow-up ran from clean source tip
`2ac67ec9`:

```text
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
source: 2ac67ec9
run id: run-2218000-1785998748489917662
result: ok — 4 wallet-authority tests passed, 0 failed in 673.40s
```

The managed 64-reservation witness keeps status and reserve/activate/complete
statement counts constant, rejects lifetime reservation aggregates, checks
exact key/prefix predicates on every captured wallet-history `SELECT`, and
verifies that the domain projection's primary-key plan remains available when
sequential scans are disabled. A production latency envelope remains
unmeasured.

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

Independent review of exact `2ac67ec9` passes the current EVM-07 SQL-path
scope: every current wallet-history `SELECT` is covered by an exact domain,
reservation, candidate, or completion predicate, the managed artifact is clean
and 4/4, and the strict projection-index check is meaningful as an index-path
availability witness. The review notes that the checks are intentionally
substring-based and do not prove a default planner choice, multi-domain scale,
or production latency; those remain conditional.

The follow-up narrows, but does not eliminate, the residuals above. EVM-07 now
has exact query-shape, non-aggregate, projection-index, and 64-row scale
evidence, but still lacks an independent production latency envelope; EVM-09
still lacks the independent offline/public-result/provider
attestation audit; SEC-01 now has direct malformed-ciphertext and wrong-identity
witnesses, but still lacks external termination/OOM/resource-failure coverage;
and the
external trust, live multi-hop export,
cross-process/fault, quality, and complete verification residuals remain
Conditional.
