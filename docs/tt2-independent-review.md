# TT2 independent review evidence

Status: **CONDITIONAL / INCOMPLETE**

This is a skeptical review of the implementation candidate whose exact
composed source gate is pinned to `82e474caf83bea3338da116e5735f06367f80742`.
The historical gate remains separate from the focused follow-up evidence below.
The latest implementation tip is `923fce06`; the focused follow-up sequence
includes `88af683d`, which adds the detached hostile wire and cryptographic
corpus below, and its preceding binding and projection proofs. The earlier
focused candidate before the public-result cutover was `f3a15978`, which includes the bounded
wallet plan checks, completion-closure reload proof, protected-key allocation
continuity and failure cleanup proofs, completion-closure permit/observation
binding proof, and persisted multi-candidate closure-only rehydration proof
below. It includes `9f61c39a` (`prove canonical frame boundaries`),
`6525fad6` (`expand configuration corpus breadth`), and
`78dbc354`/`e5f8c3b0` (exact retained-object frame proof and predicate
isolation), `a1ad9814` (provider-proof byte bounds), `c36e233e`
(finish-authorization byte bound), `75ac74f5` (deployment route-count and
proof bounds), `219c588b` (shared canonical batch-count bounds), and
`2512ebf8` (completion-recovery byte bound), `49d2e77c` (nested collection
count bounds), `70e6900e` (prior-run source-manifest byte bound),
`7741a07e` (prior-run source count bounds), and `17129b21` (native canonical
recoverability bounds), `26bbb44d` (base64url parser ingress bound), and
`6f135845` (unsigned-native primitive cutover), and `da2b41a4` (plain
canonical-document ingress bound). The earlier APP-01 proof
revision is `2fc4afa8`, which adds two
purpose-isolation compile-fail cases. The latest retry-boundary correction is
`e7624406`, which removes redundant store-level snapshot retries and leaves one
bounded PostgreSQL owner. Focused checks and the historical exact composed
source gate pass. The latest replay-proof cutover is `eb38ea44`, which adds a
real store-shaped later audit suffix and verifies its offline parity; the
preceding purpose-isolation cutover `a1a78b84` keeps the full verified cursor
and object graph inside the store while exposing only an opaque offline
summary. Revision `1ca2f7cd` retains the observed-read audit bytes in the
generated corpus and rebuilds them through the store fixture. Revisions
`db1b7503` and `c832e5d8` add direct witnesses for oversized ciphertext, a wrong
account, and a matching-account key with a valid but wrong public key. Their
predecessor `bd98e8ca` removed the full actionable frontier from
public and recorded-replay evidence. Revision `b0e1de8f` expands the managed
memory/PostgreSQL parity corpus and `0a4b02e1` fixes numeric ordering for loaded
PostgreSQL prefixes; all remaining conclusions below stay conditional where
the complete corpus or deployment proof is absent. Revision `cf74053f` then
adds escaped-JSON and exact UTF-8 byte-boundary vectors, including the matching
one-byte-over rejection, and requalifies the full managed structured-history
fixture.
Revision `77a06884` then adds a committed-but-unknown configuration
acknowledgement recovery witness and a bounded scheduling yield between exact
retry classifications; broader acknowledgement, promotion, and
production-authority matrices remain conditional.
Revision `df61d1b9` then adds deterministic run and configuration snapshot
interleavings: each reader returns the old complete prefix after a concurrent
successor commits, and a later read observes that successor. The broader
acknowledgement, promotion, and production-authority matrix remains
conditional. Revision `dcb72608` adds injected `40001` serialization and
`40P01` deadlock failures, proves rollback before fresh classification, and
retries both exact append requests; cross-process and deployment fault matrices
remain conditional. Revision `f4fd9a9b` adds the run committed-but-unknown
acknowledgement witness: the exact retry resolves `ExistingSame` with one
durable batch and an actionable reload; cross-process acknowledgement and
promotion remain conditional.
Revision `5f4be506` adds a two-publication same-producer fact-scan witness:
each scan invocation reads one dense page, loads one producer prefix, and folds
three retained producer batches, with transient retry totals measured
separately. Revision `bb002b19` scopes those test-only counters by store
identity, so isolated parallel schemas cannot contaminate the proof. This
closes the repeated-growing-prefix work gap. Revision `2ac67ec9` adds exact
key/prefix predicates to every captured wallet-history `SELECT`, preserves the
64-reservation constant-query witness, and checks the domain projection's
primary-key path with sequential scans disabled; its clean managed leaf is
`run-2218000-1785998748489917662` (4/4 tests).
Revisions `25dc49e7`, `b59d820a`, and `618cadea` split the callback-free
provider proof check from the SQL historical-incarnation lookup and add a 3/3
crate-local Completion corpus. The proof is installed in the persisted
completion closure; the test rehydrates typed completion request/state-input
and derives the provider preimage from closure bytes before verifying it
without `PgConnection`. Signature/key, payload, provider/op, target-policy,
challenge, canonical-JSON, non-ASCII, and generated proof-budget substitutions
are rejected. Revision `f3a15978` adds the managed SQL omission and rewritten
historical-incarnation-row regression; its clean wallet qualification passes
all four tests. The full EVM-09 deployment/provider-trust and production
offline/public-result matrix remains conditional.

Revision `137ed63b` addresses the separately substantiated EVM-09/APP-01
public-result leak. The successful `EvmSubmissionOutput` projection now carries
only `ExecutionDisposition`, derives `PublicOutputs`, and is advertised through
the public schema descriptor; the domain regression serializes exactly the
disposition object and rejects completion/attestation/signature markers. The
managed production leaf was rerun on the clean commit and still fails before
admission with `ProductionRegistryInvalid`: the 32-candidate expanded program
is about 26.3 MiB and its RFC-required exact component-byte closure digest is
about 97.4 MiB, beyond the generated 32 MiB canonical-document budget. This
production-scale blocker is recorded as conditional rather than masked by a
larger limit or a smaller wallet policy.

Revision `f5815ccb` adds the independent detached EVM completion audit. The
new audit reconstructs the wire-shaped completion mutation from persisted
recovery-closure bytes, binds an explicit provider id, operation key, target
context, and Ed25519 public key, recomputes the canonical payload digest, and
checks the signed challenge without calling the storage verifier, PostgreSQL,
or a live provider. It then projects the public result from closure bytes and
asserts the exact `{"execution_disposition":"succeeded"}` bytes. A provider
substitution and a closure-projection substitution are rejected through the
independent verifier; the focused storage package passes 11 tests (one
managed-schema test remains ignored).
At revision `f5815ccb`, the production deployment/provider-trust and
production-scale leaf remained conditional because of the canonical
closure-size blocker.

Revision `6cd74eef` changes only the certified-program closure-digest
encoding: each already-canonical component value is inserted as raw JSON in
the enclosing canonical preimage instead of as a JSON numeric array. The
exact component bytes, generated 32 MiB canonical-document bound, and
32-candidate policy remain unchanged. Focused certification tests pass 38/38,
and the managed production-scale qualification now passes:

```text
nix run .#run -- --task evm-postgres-submission-qualification
run id: run-2375667-1786015092059379426
result: ok — 1 passed, 0 failed in 1208.51s (task total 1208.99s)
```

Revision `64517b46` extends the negative audit matrix so the forged projection
closure is rejected by the independent verifier itself, not only by the
production projection helper; the focused storage package remains green.

Revision `60554999` extends the independent negative corpus with operation-key
and target-context substitutions. Each forged proof is serialized and rejected
by the callback-free verifier; the full storage target remains 11 passed and
one managed-schema test ignored.

Revision `88af683d` completes the independent detached-proof hostile corpus for
the remaining wire and cryptographic substitutions. The callback-free verifier
now rejects forged payload digests, signatures, trust keys, malformed and
wrong-length challenges, and reordered JSON with exact branch-level errors;
the full storage target passes 12 tests with one managed-schema test ignored.

Revision `923fce06` adds independent context and parser-boundary witnesses.
Zero/invalid database and backend identities, missing transactions, snapshots,
bad application markers, invalid store epochs, empty/non-ASCII/oversized proofs,
and unknown proof fields are rejected at their exact callback-free branches; the
full storage target passes 13 tests with one managed-schema test ignored.

Revision `bc469cc8` makes the managed production qualification inject an
explicit status-137 child-process exit after the broadcast observation has
committed. The fresh resume worker closes the same run and the test asserts
exactly one `eth_sendRawTransaction` call:

```text
nix run .#run -- --task evm-postgres-submission-qualification
run id: run-2394128-1786018130470251941
result: ok — 1 passed, 0 failed in 1218.27s (task total 1218.81s)
```

This closes the post-broadcast crash/restart boundary for the repository-local
qualification scope. Receipt, finality, promotion, completion, ambiguity,
cross-process, and scale fault matrices remain conditional.

This closes the repository-local production-scale EVM leaf blocker. The
deployment-owned provider-trust and broader crash, ambiguity, latency, and
production-authority matrices remain conditional.
The plan requires a PASS only when every
Blocker/High requirement has its focused proof. The review therefore records
both the closed implementation work and the remaining proof/deployment gaps.

## Material uncertainties

- The unpublished `mfm-authority-seal` crate is treated as a workspace-only
  deployment boundary. Separate distribution would require a new authority
  review.
- SQLx/offline, SQL inventory, and model-check results are task outputs and
  were not independently regenerated by this review.
- Retained physical-release and store/checkpoint trust have test
  implementations but no concrete production deployment implementation in
  this repository.
- The managed qualification now injects one status-137 child-process exit
  immediately after a durable broadcast observation and proves resume without
  a second broadcast; the complete receipt/finality/promotion/completion,
  ambiguity, cross-process, and scale kill matrix remains unverified.
- No live application multi-hop export integration proves the production
  closure path. The application now performs kind-aware fixed-point discovery,
  retains an authenticated principal/fixed export grant/decision reference per
  selected prefix, and fails before reader access on denied root/dependency
  decisions; the production integration proof is still absent.
- The store-shaped observed-read fixture now proves a foldable later
  authorization/observation suffix and online/offline audit parity. Revision
  `1ca2f7cd` retains its exact bytes as generator-owned corpus input and the
  corpus test rebuilds and verifies that artifact; live Postgres/application
  replay evidence is still absent.
- The decision references are intentionally opaque content-addressed policy
  evidence. Offline replay binds the exact closure digest through its explicit
  trust snapshot and does not resolve policy live.
- The wallet proof now captures every observed wallet-history `SELECT` and
  requires an exact domain, reservation, candidate, or completion key/prefix,
  rejects lifetime aggregates, and verifies the domain primary-key path under
  a strict planner setting. A full production latency envelope remains
  unmeasured.
- The completion-closure reload and protected-key allocation tests are focused
  package evidence, not members of the historical composed gate. The latest
  managed fixture now reloads a persisted two-candidate completion row before
  rehydrating and projecting its closure.
- The latest keystore witnesses cover source-to-`SecureKey` handoff, truncated
  and oversized ciphertext rejection before plaintext allocation, zeroized
  cleanup after authentication/AAD and authenticated post-decrypt invalid-key
  rejection, and an explicit valid-but-wrong public/account identity. External
  termination/OOM/resource failures remain outside focused package evidence.
- The explicit offline fold seam now returns an opaque `OfflineVerifiedRun`
  containing recorded status and bounded export metadata when callers provide
  `RawRunHistory` and concrete trust. The complete cursor/object graph remains
  store-private; the broader runtime/audit/replay/export matrix is still not
  independently reproduced.
- The shared configuration append boundary now rejects an oversized serialized
  revision before backend dispatch. Managed qualification covers positive,
  exact-limit, one-byte-over, stale-predecessor, idempotent-replay, escaped
  JSON, an exact UTF-8 byte-boundary value with one-byte-over rejection, 128
  deterministic JSON shape values across 16 shape families, a 128-revision
  sequential stream, and an exact canonical depth-limit value against memory
  and PostgreSQL. The latest default-concurrency managed run also passes all
  24 tests, including the same-stream race; two earlier attempts hit that
  race intermittently. The shared
  run-envelope validator also accepts an exact `MAX_STORED_FRAME_BYTES` frame
  and rejects one byte over; the retained-object validator accepts a valid
  exact-limit object, and its shared byte predicate rejects one byte over
  without relying on malformed JSON or a stale content digest. One-level-
  over depth, float, duplicate-key, and malformed values are rejected before
  backend dispatch; the complete generated, hostile, and large-scale
  acceptance matrix remains unverified.

## Candidate and review scope

- Implementation candidate: `82e474caf83bea3338da116e5735f06367f80742`
- Focused follow-up candidate: `923fce06` (`cover detached audit context boundaries`),
  following `88af683d` (`strengthen detached audit hostile corpus`),
  following `60554999` (`expand detached audit binding proofs`),
  following `bc469cc8` (`exercise evm crash recovery boundary`),
  following `64517b46` (`strengthen detached audit substitution proof`),
  following `6cd74eef` (`fix certified closure digest encoding`),
  following `137ed63b` (`redact evm submission public output`),
  following `f3a15978` (`prove managed historical incarnation lookup tamper`),
  including `6f135845` (`remove unreachable signed native values`),
  including `6525fad6` (`expand configuration corpus breadth`),
  including `8a902d03` (`fail closed SQL file macros`),
  including `2fea7802` (`restore SQL inventory builder scope`),
  including `836e9c31` (`harden SQL inventory scope collection`),
  `018a946a` (`collect SQL inventory imports before traversal`),
  `1edcf7bb` (`close unchecked SQL inventory aliases`), and
  `40051076` (`cover unchecked SQL query macros`), and
  `a3ad0e9a` (`expand configuration acceptance corpus`),
  `2ac67ec9` (`tighten bounded wallet query
  proof`), `afec8457` (`prove closure-alone
  multi-candidate reload`), `50c8cac1` (closure-only rehydration and
  public projection), `4fd1e757` (AAD identity cleanup witness), `61bf2091`
  (shared decrypt failure cleanup), `ed7b341e` (completion closure permit and
  observation binding), `266d6889` (bounded wallet projection scan),
  `dec2f0c4` (completion closure reload), and `9390503c` (production decrypt
  allocation witness), `d2a39d7a` (post-decrypt invalid-key cleanup), and
  `8b2e64ba` (failure-path handoff-pointer witness), and `2fc4afa8`
  (purpose-isolation compile-fail cases), and the SQL inventory sequence
  `40051076`/`1edcf7bb`/`018a946a`/`836e9c31`/`2fea7802`/`8a902d03`
  (unchecked, query-file, import, and scope coverage), followed by
  `6525fad6` (expanded configuration corpus breadth) and `9f61c39a`
  (exact/one-byte-over shared frame boundaries), followed by `78dbc354`
  (`cover exact object frame boundary`), `e5f8c3b0` (predicate isolation),
  `a1ad9814` (provider-proof byte bounds), `c36e233e`
  (finish-authorization byte bound), `75ac74f5` (deployment assembly
  route-count/proof bounds), `219c588b` (canonical batch-count bounds), and
  `2512ebf8` (completion-recovery byte bound), and `49d2e77c` (nested
  collection count bounds), and `70e6900e` (prior-run source-manifest byte
  bound), and `7741a07e` (prior-run source count bounds), and `17129b21`
  (native canonical recoverability bounds), followed by `26bbb44d` (base64url
  parser ingress bound) and `6f135845` (unsigned-native primitive cutover),
  followed by `f3a15978` (managed historical-incarnation omission and rewrite
  regression).
- Retained later-audit artifact: `1ca2f7cd` (`retain observed audit artifact
  in replay corpus`), which adds the exact store-shaped observed-read bytes,
  generator acceptance/rejection vectors, and an offline parity branch in the
  corpus test.
- Prior implementation/evidence tip: `4e11e3556348cf61d27294a2caa18f5e0dba3635`.
- Final evidence refresh: this documentation-only commit after the exact gate;
  the candidate review was performed against the exact hash above.
- Implementation revisions leading to the prior candidate: `e68aca910`
  (idempotent configuration checkpoint predecessor classification), `fcd56ab09`
  (bounded transient checkpoint-read retries), and `f99e3a84c` (SQL inventory
  ownership for the retry wrappers).
- Implementation revision in the prior candidate: `6d4f48933` (configuration
  reader retry and identical append ambiguity recovery, with focused tests and
  design/architecture contract updates), followed by `4e11e3550` (durable-row
  ambiguity regression strengthening).
- Current parity/status revision: `0a4b02e1` (numeric configuration-prefix
  ordering after the shared serialized bound and expanded managed
  memory/PostgreSQL acceptance vectors, plus stale purpose-status assertions
  after the frontier cutover).
- Checkpoint durability revision: `fce1edca` persists successful `Prepared`
  successors in the test authority, rejects unprepared acknowledgements, and
  proves fresh single- and multi-key ledger restart recovery.
- Checkpoint sidecar arbitration revision: `27edd8c6` adds an OS file lock and
  reload-before-validation so independently loaded workers cannot overwrite a
  newer prepared successor.
- Configuration boundary corpus revision: `cf74053f` adds escaped-JSON and
  exact UTF-8 byte-boundary vectors with a one-byte-over rejection, then
  requalifies the managed memory/PostgreSQL fixture.
- Configuration acknowledgement revision: `77a06884` injects one committed-
  but-unknown acknowledgement, proves exact retry without a duplicate row, and
  yields between bounded stale classifications.
- Snapshot interleaving revision: `df61d1b9` adds run and configuration
  repeatable-read barriers and qualifies the old-prefix/new-prefix outcome on
  the managed PostgreSQL task.
- Contention rollback revision: `dcb72608` injects `40001` and `40P01` batch
  failures, proves zero durable rows before fresh classification, and retries
  each exact append on the managed PostgreSQL task.
- Run acknowledgement revision: `f4fd9a9b` injects one committed-but-unknown
  run acknowledgement, retries the identical candidate as `ExistingSame`, and
  audits one durable batch on the managed PostgreSQL task.
- Fact-scan work-bound revision: `5f4be506` publishes two successive facts from
  one producer and asserts one publication page, one producer-prefix load, and
  three folded producer batches per scan invocation on the managed PostgreSQL
  task.
- Fact-scan counter-scope revision: `bb002b19` keys the test-only measurements
  by store identity and requalifies the same 24-test managed lane.
- Bounded wallet query-shape revision: `2ac67ec9` requires exact key/prefix
  predicates for every captured wallet-history `SELECT`, retains the 64-row
  constant-query witness, and checks the domain primary-key path with
  sequential scans disabled; managed run `run-2218000-1785998748489917662`
  passes 4/4 tests.
- Configuration corpus revision: `a3ad0e9a` expands memory/PostgreSQL parity
  to 64 deterministic shape values and 64 sequential successors, adds an exact
  canonical depth-limit append, and rejects one-level-over depth, float,
  duplicate-key, and malformed values before dispatch; managed run
  `run-2223304-1785999851634604306` passes 24/24 tests.
- Configuration corpus breadth revision: `6525fad6` expands the same parity
  fixture to 128 values across 16 canonical shape families and 128 sequential
  successors. The serialized managed run passes 24/24; default-concurrency
  attempts separately hit the pre-existing same-stream race witness, so this
  is targeted corpus evidence rather than a default-concurrency gate pass.
- Canonical frame boundary revision: `9f61c39a` adds exact and one-byte-over
  `MAX_STORED_FRAME_BYTES` witnesses. Independent review confirms the shared
  limit equals the canonical and PostgreSQL octet-length bounds; the focused
  `mfm-store` target passes 3/3.
- Canonical object-frame revision: `78dbc354` adds a valid exact-limit object
  witness, and `e5f8c3b0` isolates the shared object/envelope byte predicate so
  the one-byte-over object assertion cannot pass because of malformed JSON or
  a stale content digest. The focused target passes 4/4 and independent review
  marks this boundary proof PASS.
- Provider evidence-bound revisions: `a1ad9814` adds exact/one-byte-over
  `MAX_PROVIDER_PROOF_BYTES` witnesses in both the EVM closure and PostgreSQL
  provider validators. `c36e233e` extracts the decoded finish-authorization
  check and proves exact/one-byte-over `MAX_PROVIDER_FINISH_AUTHORIZATION_BYTES`
  with fresh matching digests. The provider frame target passes 4/4 and
  independent review marks both bounds PASS. `75ac74f5` adds exact/one-route-over
  `MAX_PROVIDER_DEPLOYMENT_ROUTES` and exact/one-byte-over route-proof
  constructor witnesses; the provider frame target passes 6/6 and review marks
  the deployment assembly bounds PASS.
- Canonical batch-count revision: `219c588b` routes record and object counts
  through one shared minimum/maximum predicate. Exact 65,536-record and
  65,536-object counts, empty-record rejection, and one-over rejection pass in
  the focused canonical-append target (5/5); independent review marks the
  count boundary PASS.
- Completion-recovery bound revision: `2512ebf8` routes both canonical
  encode/decode paths through one `MAX_COMPLETION_RECOVERY_BYTES` predicate.
  Exact 524,288-byte and one-over tests pass in the focused EVM target (1/1),
  preserving the reviewed `BoundExceeded("completion_recovery")` error.
- Nested collection-bound revision: `49d2e77c` generalizes the shared count
  predicate to nested arrays and objects. Exact 1,048,576-item and one-over
  witnesses for both generated collection budgets, alongside the batch-count
  cases, pass in the focused canonical-append target (5/5); independent review
  marks the collection boundary PASS.
- Prior-run manifest-bound revision: `70e6900e` routes both history-object
  encode and decode through one private byte validator. Exact 16,777,216-byte
  and one-byte-over witnesses pass in the focused `mfm-journal` structured
  target (3/3); independent review marks this shared ingress/rehydration
  boundary PASS.
- Prior-run source-count revision: `7741a07e` routes rule program/descriptor
  counts and manifest rule/total-reference counts through private validators.
  Exact 4,096-program, 4,096-descriptor, 1,024-rule, and 65,536-reference
  budgets plus one-over rejection and empty-descriptor rejection pass in the
  focused `mfm-journal` structured target (5/5); wildcard empty programs remain
  accepted and independent review marks these count boundaries PASS.
- Native canonical-bound revision: `17129b21` makes the native-wire validator
  enforce generated `MAX_STRING_UTF8_BYTES` (16,777,216),
  `MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES` (1,048,576), `MAX_ARRAY_ITEMS`, and
  `MAX_OBJECT_ENTRIES` (1,048,576) before recursion. Direct exact/one-over
  `serde_json::Value` tests pass in focused `mfm-canonical` (1/1), the full
  canonical targets pass 3/11/5, and independent review marks the native
  boundary PASS.
- Base64url-ingress revision: `26bbb44d` applies generated
  `MAX_BASE64URL_CHARACTERS` before scanning or decoded allocation. Exact
  22,369,622-character input decodes to 16,777,216 bytes and one-over input
  rejects in the focused canonical-json target (1/1); independent review
  marks the parser boundary PASS.
- Unsigned-native cutover: `6f135845` removes the unreachable signed variant
  from the primitive recoverability schema, regenerates the annex/corpus and
  derived schema identities, and deletes the misleading `FactScalar::signed`
  API. Canonical negatives reject signed JSON numbers and fact regressions
  reject positive and negative `CanonicalValue::Signed`; full canonical targets
  pass 3/12/6 plus 3 doctests and facts pass 14/14. Generic signed canonical
  values remain available outside the primitive native contract.
- Plain canonical-document ingress: `da2b41a4` applies the generated
  33,554,432-byte document budget before number scanning, JSON parsing, or
  canonical allocation, and checks canonicalized output as well. Exact-limit
  input is accepted and one byte over is rejected in the focused canonical-json
  target (1/1); full canonical targets pass 3/13/6 plus 3 doctests, and the
  affected facts, values, journal, and store targets remain green. Independent
  review marks this parser boundary PASS.
- Detached provider proof cutover: `25dc49e7` removes the mixed SQL/crypto
  verifier; `b59d820a`/`618cadea` add the persisted-closure Completion corpus.
  The focused storage target passes 3/3 without `PgConnection`, including
  closure-derived typed request/state-input/preimage verification and hostile
  provider, context-policy, digest, signature, challenge, canonicality, and
  budget cases. Revision `f3a15978` adds a clean managed 4-test historical-row
  omission/rewriting regression; deployment trust remains conditional.
- Default-concurrency qualification: clean managed run
  `run-2246603-1786002530958490012` passes all 24 structured-history tests,
  including `configured_value_history_linearizes_same_stream_append_races`;
  two earlier attempts remain recorded as intermittent witnesses.
- SQL inventory revisions: `40051076` adds the pinned SQLx
  `query_unchecked!`, `query_as_unchecked!`, and `query_scalar_unchecked!`
  macro forms; `1edcf7bb` corrects SQL argument indexing and covers direct,
  renamed-alias, and glob imports; `018a946a` pre-collects after-use
  file/module imports; `836e9c31` pre-collects block imports and restores
  nested import state; `2fea7802` restores `QueryBuilder` binding state; and
  `8a902d03` recognizes all six pinned SQLx `query_file*` forms and rejects
  external-file SQL before literal ownership parsing. The source inventory and
  syntax fixture tests remain 2/2.
- SQL-inventory fixture revision: `7e467952` (generic scalar/query-as calls,
  checked macro, wrapped helper, and QueryBuilder fragment coverage).
- Retry-boundary correction: `1ef7d694` bounds configuration append/load
  ambiguity retries at eight attempts, and `e7624406` removes the redundant
  generic store snapshot retry so PostgreSQL owns the single bounded
  eight-attempt checkpoint-read retry. The exact configuration suite passes
  7/7, the full `mfm-store` library suite passes 33/33, and the managed
  recoverability task passes on `e7624406`.
- Purpose-isolation cutover: `bd98e8ca` replaces public and recorded-replay
  `StructuredFrontier` access with the data-free `RunEvidenceStatus`, updates
  app/replay projections, and adds frontier-denial compile-fail cases for both
  products.
- Offline-fold boundary cutover: `a1a78b84` removes the public
  `VerifiedStructuredRun` conversion seam. The explicit offline fold now
  returns opaque `OfflineVerifiedRun` metadata, with compile-fail coverage for
  frontier access and the removed full-fold type.
- Later-audit suffix proof: `eb38ea44` adds a store-shaped one-state Read
  history with committed authorization and observed return batches. Audit
  export folds offline with byte-identical projection, while the same suffix
  carried as Semantic is rejected at strict decode.
- Artifact provenance cutover: `1ca2f7cd` retains those exact audit bytes in
  `contracts/recoverability/v1/portable_store_observed_read_audit.hex`, emits
  acceptance and Semantic-rejection vectors from the single generator owner,
  and rebuilds the bytes through the real store fixture before offline parity.
- Keystore boundary witnesses: `db1b7503` adds explicit oversized-ciphertext
  and wrong-account coverage, while `c832e5d8` adds matching-account wrong-
  public-key coverage; the full package and Clippy checks pass on the latter.
- Current post-gate corpus/test-only revisions: `5711097b` (deterministic
  portable-vector generation, generated corpus/README, replay corpus assertion),
  `74bfa335` (generated offline-fold acceptance vector), and `6284e8d9`
  (production-shaped suffix and nested source-graph vectors); none changes
  production replay/runtime behavior. `82e474ca` then removed the normal-path
  lifetime reservation `COUNT/MAX`, added the bounded projection `EXPLAIN`
  regression and current-frontier omission regression, and was reviewed as the
  current implementation candidate.
- The retry-boundary review candidate is `e7624406` (parent `1ef7d694`): the
  generic store snapshot retry is deleted, PostgreSQL retains the sole bounded
  eight-attempt checkpoint-read retry, and configuration retry loops share the
  eight-attempt bound.
- The focused follow-up adds a 64-reservation managed history probe with
  Q/E cardinality and Q/P SQL-text assertions, final-status capture, and
  `EXPLAIN (ANALYZE)` plan/row checks for the domain projection, exact
  reservation, candidate prefix, completion, and nonce frontier. It also
  round-trips a persisted two-candidate wallet completion closure, rehydrates
  and projects it from the closure alone, and exercises decrypt to signing
  through the same protected heap allocation.
- Documentation-only provenance after the earlier candidate: `ef3e412d5`
  (wording), `42c80525` (whitespace cleanup), `a027640c` (review refresh),
  `d0459d4d` (bounded-source review refresh), `8803a585` (evidence pin),
  `ebc4f8a81`, `7530a4479`, and `f8568ff26`. None changes Rust, SQL, generated
  contracts, or test behavior. The current evidence refresh follows the
  `82e474ca` gate; the closure-binding follow-up is `ed7b341e`.
- Original implementation baseline: `07b9d7311daae32230d7a487aa82e07f0d27ff2b`
- Historical review evidence: `258059180` (FAIL), `40612039f` (FAIL),
  `16fe501a` (FAIL), `9bd6f7056` (PASS for its candidate), and `76ce09085`
  (PASS for its lint-clean candidate).
- Scope: authority sealing, PostgreSQL snapshot/checkpoint/retry behavior,
  EVM signer and wallet recovery, canonical projections, portable recursive
  evidence, generated contracts, and redaction.

## Verification evidence

The previous exact implementation gate ran in the default Nix development
environment and is historical evidence for the preceding source sequence:

```text
nix run .#ci
source: d67a3bc3aac44d6820ee47a9235a8bfdbb3a6ed2
run id: run-1587720-1785947457045693112
result: ok — 13 passed, 0 failed in 1738.58s
structured EVM submission: ok in 1229.82s
```

The immediately preceding f99 implementation gate also passed:

```text
nix run .#ci
source: f99e3a84ced87246d19a93a4a19a1ab300d5e398
run id: run-1733167-1785954807662948771
result: ok — 13 passed, 0 failed in 1933.45s
structured EVM submission: ok in 1416.48s
```

The subsequent 6d implementation gate also passed:

```text
nix run .#ci
source: 6d4f4893371a2e48dbacb0ed5f5959547306c029
run id: run-1760799-1785957370345872343
result: ok — 13 passed, 0 failed in 1802.02s
structured EVM submission: ok in 1261.66s
```

The current exact composed gate ran on the reviewed implementation candidate:

```text
nix run .#ci
source: 82e474caf83bea3338da116e5735f06367f80742
run id: run-1900515-1785969180314881458
result: ok — 13 passed, 0 failed in 1811.86s
structured EVM submission: ok in 1322.75s
```

The same run passed the PostgreSQL recoverability, wallet-nonce, Bitcoin
parity, and closing-source-revision leaves. It was run without separate
composed check/test/test-db gates immediately beforehand. The closing-source-
revision leaf observed the pinned `82e474ca` source; the evidence-only refresh
after the run does not alter that implementation source.
The portable leaf consumed the tracked generated corpus and passed 9/9 replay
tests; deterministic regeneration was checked separately with
`python3 contracts/recoverability/generate.py`. The corpus metadata is
1,115,887 bytes, SHA-256
`aaed5cbea0feb3650d8e3d912f6b8f3f688230427619185defe77adb12ac9b80`, 15
artifact vectors, and two source-graph vectors.

The final independent checkpoint reviewed the exact run summary and closing
source leaf: all 13 nodes succeeded, workspace nextest reported 590 passed and
1 skipped, and the closing leaf printed the full candidate hash above. The
review also confirms that the gate consumes tracked corpus bytes; generator
determinism is separate evidence, not a hidden gate step.

The green leaves were formatting, Clippy, metadata, SQLx offline, portable
replay corpus, workspace nextest, doctests, PostgreSQL SQLx checks,
recoverability, wallet-nonce storage qualification, structured EVM submission,
Bitcoin parity, and closing-source-revision. Focused evidence additionally
records nine portable replay tests, four flattened recursive closure tests, two
portable source/fact-route bound regressions,
exact frame/total byte limits and one-over denials, root target/tenant trust
tamper denials, bounded wallet projection `EXPLAIN` coverage, and online/offline
projection byte equality. The managed
PostgreSQL recursive parity fixture passed 17/17 on the same implementation
sequence. The full composed gate was not preceded by separate composed
check/test/test-db runs.

The focused follow-up candidate was qualified separately after the historical
composed gate. The final EVM closure-binding revision was then reviewed
directly against its exact commit:

```text
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
source: 266d6889
run id: run-1977166-1785976039045273113
result: ok — 1 task, 4 tests passed, 0 failed in 660.48s

nix develop -c cargo test -p mfm-evm
source: ed7b341e
result: 47 unit tests passed; signing UI trybuild and 2 doctests passed

independent review: ed7b341e
result: exact validation and hostile-test review passed; no implementation gap
```

The latest closure-only rehydration regression was then qualified on the exact
tip and reviewed independently:

```text
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
source: afec8457
run id: run-2003174-1785978540996350273
result: ok — 1 task, 4 tests passed, 0 failed in 671.76s

independent review: afec8457
result: persisted two-candidate completion reload, closure-only rehydration,
and canonical public projection confirmed; no implementation gap
```

The long SQL test reloads an existing completion row through both authorities
before the assertions, so the two-candidate closure is persisted PostgreSQL
state rather than an in-memory-only construction. At this historical revision
the remaining EVM-09 proof scope was an independent offline/public-result
audit, including whether the provider completion attestation was independently
verifiable without the storage verifier; revision `f5815ccb` supplies that
repository-local detached audit.

That run includes the 64 completed reserve/activate/complete reservations,
constant status and mutation statement counts, Q/P SQL-text rejection of
lifetime reservation aggregates in baseline/mutation/final-status slices, and
post-history `EXPLAIN (ANALYZE)` checks. The historical diagnostic
`run-1971258-1785975345382628345` is retained as a superseded failure: it
over-constrained the one-row domain projection to an index plan, while
PostgreSQL correctly selected a one-row sequential plan. `266d6889` records
that bounded projection case explicitly.

The retry-boundary correction was then qualified on its exact clean tip:

```text
nix run .#run -- --task recoverability-postgres-v1
source: e7624406
run id: run-2046822-1785981827251476177
result: ok — 1 task, 0 failed in 36.81s

nix develop -c cargo test -p mfm-store --lib configuration -- --nocapture
source: e7624406
result: 7 passed, 0 failed

nix develop -c cargo test -p mfm-store --lib
source: e7624406
result: 33 passed, 0 failed

nix develop -c cargo fmt --all -- --check
result: pass

independent review: e7624406
result: snapshot retry ownership is singular and finite; no correctness gap
```

The correction deletes the generic store retry, so each structured reader or
writer invokes its backend once; PostgreSQL's checkpoint-read helper is the
sole eight-attempt owner. Configuration's eight-attempt load and identity
recovery loops remain finite and reuse the canonical revision digest. The
review notes that the bound is per nested loop and that PostgreSQL retries the
classified `InvalidHistory` result even when corruption is persistent; these
are bounded resource/diagnostic residuals, not correctness failures.

The configuration parity/status revision was then qualified from clean source
tip `0a4b02e1`:

```text
nix run .#run -- --task recoverability-postgres-v1
source: 0a4b02e1
run id: run-2138056-1785988778488975922
result: ok — 18 structured-history tests passed, 0 failed in 31.91s

nix develop -c cargo fmt --all -- --check
result: pass

nix develop -c cargo clippy -p mfm-store --lib -- -D warnings
result: pass
```

The managed test includes positive, exact serialized
`MAX_CONFIGURATION_REVISION_BYTES`, one-byte-over, stale-predecessor,
idempotent replay, 32 deterministic JSON shape, and 32 sequential-successor
vectors with final reader parity. A separate Nix release no-run check compiles
the PostgreSQL integration assertions against `RunEvidenceStatus`; no purpose
wrapper calls the removed `.frontier()` API.

```text
nix develop -c cargo test --release -p mfm-integration-tests \
  --features parity-tests --test evm_postgres_submission --no-run
source: 0a4b02e1
result: pass
```

The checkpoint preparation durability fix was then qualified on the exact
source tip:

```text
nix run .#run -- --task recoverability-postgres-v1
source: fce1edca
run id: run-2150431-1785990104705897798
result: ok — 18 structured-history tests passed, 0 failed in 32.08s

nix develop -c cargo test -p mfm-storage-postgres \
  --features test-support checkpoint --lib
source: fce1edca
result: 9 checkpoint tests passed, 0 failed
```

The sidecar test authority now persists every successful prepared successor
before returning. Fresh instances reload single- and multi-key Prepared state,
reject a conflicting successor, and acknowledge the exact bytes after restart;
unprepared `acknowledge_many` calls fail closed. This is repository-local test
authority evidence, not a concrete production checkpoint deployment.

The sidecar arbitration follow-up was then qualified on its exact source tip:

```text
nix run .#run -- --task recoverability-postgres-v1
source: 27edd8c6
run id: run-2155505-1785990746657729092
result: ok — 18 structured-history tests passed, 0 failed in 32.05s

nix develop -c cargo test -p mfm-storage-postgres \
  --features test-support checkpoint --lib
source: 27edd8c6
result: 10 checkpoint tests passed, 0 failed
```

The sidecar now locks its file across workers, reloads the durable maps before
mutating validation, and proves that a stale independently loaded worker cannot
replace a newer prepared successor. Production external authority, broader
acknowledgement-loss, promotion, and cross-process fault matrices remain
outside this repository-local fixture.

The expanded configuration boundary corpus was then qualified from clean source
tip `cf74053f`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: cf74053f
run id: run-2162922-1785991603848871458
result: ok — 18 structured-history tests passed, 0 failed in 31.72s
```

The added vectors compare memory and PostgreSQL on escaped JSON, an exact
UTF-8-sized serialized revision, and a one-byte-over UTF-8 revision; the same
run retains the prior boundary, shape, scale, race, and fresh-process cases.

The acknowledgement-recovery follow-up was then qualified from clean source
tip `77a06884`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: 77a06884
run id: run-2167617-1785992166392324915
result: ok — 19 structured-history tests passed, 0 failed in 31.76s
```

The injected commit acknowledgement loss commits the row normally while
returning `AcknowledgementUnknown`; the writer retries the exact revision,
re-acknowledges the prepared successor, and the audit query confirms one row.

The snapshot interleaving follow-up was qualified from clean source tip
`df61d1b9`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: df61d1b9
run id: run-2177250-1785993122646524325
result: ok — 21 structured-history tests passed, 0 failed in 32.49s
```

The run/configuration barriers release the external fixation, commit a
successor while the reader's repeatable-read transaction remains open, and
assert the old complete prefix. A subsequent read observes the successor.
This closes the deterministic snapshot-interleaving proof; broader
acknowledgement, promotion, and production-authority matrices remain.

The contention rollback follow-up was independently rerun from clean source
tip `dcb72608`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: dcb72608
run id: run-2190510-1785994272280747833
result: ok — 22 structured-history tests passed, 0 failed in 32.29s
```

The test injects one `40001` serialization failure and one `40P01` deadlock at
batch insertion. Both attempts leave zero durable rows, classify only after
rollback in a fresh transaction, and accept the identical append after fault
removal. Cross-process acknowledgement, promotion, and production-authority
matrices remain conditional.

The run acknowledgement-recovery follow-up was independently rerun from clean
source tip `f4fd9a9b`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: f4fd9a9b
run id: run-2194313-1785994645494655426
result: ok — 23 structured-history tests passed, 0 failed in 32.33s
```

The run append commits one batch while reporting `AcknowledgementUnknown`; the
identical retry resolves `ExistingSame`, audits exactly one durable batch, and
reloads an actionable run. Cross-process acknowledgement, promotion, and
production-authority matrices remain conditional.

The fact-scan work-bound follow-up was independently rerun from clean source
tip `bb002b19`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: bb002b19
run id: run-2206767-1785995910502340324
result: ok — 24 structured-history tests passed, 0 failed in 37.57s
```

The same-producer fixture publishes two successive facts. The counters assert
one dense page, one prefix load, and three producer batches folded per scan
invocation; transient retries each repeat one bounded prefix load rather than
refolding a growing prefix for every route. The counters are keyed by store
identity, so isolated parallel schemas cannot contaminate the measurement.

The bounded wallet query-shape follow-up was independently rerun from clean
source tip `2ac67ec9`:

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

Independent review of exact `2ac67ec9` passes this current-path EVM-07 scope.
The substring-based SQL checks cover every current authority query, and the
strict projection check witnesses index-path availability. They do not prove a
default planner choice, multi-domain scale, or production latency; EVM-07
therefore remains conditional for that envelope.

Independent review of exact `a3ad0e9a` passes this STORE-05 slice. Canonical
depth 64 is accepted through both backends while depth 65, float, duplicate-key,
and malformed JSON reject at shared ingress before an append can be built; the
64-shape and 64-revision parity vectors pass in the clean 24-test run. The
review confirms that hostile forms are intentionally parser-only and that the
finite corpus does not establish production-scale acceptance.

The configuration corpus expansion was independently rerun from clean source
tip `a3ad0e9a`:

```text
nix run .#run -- --task recoverability-postgres-v1
source: a3ad0e9a
run id: run-2223304-1785999851634604306
result: ok — 24 structured-history tests passed, 0 failed in 38.75s
```

Independent review of exact `6525fad6` passes this bounded STORE-05 corpus
slice. Each of 16 valid canonical JSON shape families is repeated eight times
on isolated streams; 128 sequential successors preserve memory/PostgreSQL
parity, exact predecessors, and the final head. Exact/one-byte-over,
depth/hostile, and UTF-8 vectors remain. The serialized managed run passes
24/24; two default-thread attempts failed only the pre-existing same-stream
race witness (23/24), so this is targeted evidence rather than a
default-concurrency gate pass.

The expanded corpus ran from clean source tip `6525fad6`:

```text
RUST_TEST_THREADS=1 nix run .#run -- --task recoverability-postgres-v1
source: 6525fad6
run id: run-2240964-1786001827263556370
result: ok — 24 structured-history tests passed, 0 failed in 45.04s
```

The canonical ingress-boundary unit target ran from clean source tip `49d2e77c`:

```text
nix develop -c cargo test -p mfm-store --lib structured::canonical_append -- --nocapture
source: 49d2e77c
result: ok — 5 canonical-append unit tests passed, 0 failed
```

Independent review confirms that the shared count predicate accepts records in
`[1, 65,536]`, objects in `[0, 65,536]`, and nested arrays/objects in
`[0, 1,048,576]`, and that the ASCII boundary helper measures octets,
that `MAX_STORED_FRAME_BYTES` equals the canonical package limit, and that the
PostgreSQL `octet_length` constraint has the same exact and one-byte-over
semantics. The exact retained object is content-addressed and valid; the
object one-byte-over assertion exercises the shared length predicate directly,
so canonical/content-ref failure cannot explain the rejection.

The prior-run source-manifest boundary target ran from clean source tip
`70e6900e`:

```text
nix develop -c cargo test -p mfm-journal --lib structured:: -- --nocapture
source: 70e6900e
result: ok — 3 structured-journal unit tests passed, 0 failed
```

Independent review confirms that both history-object encode and decode call the
same private byte validator, accepting the exact generated 16,777,216-byte
budget and rejecting one byte over before deeper canonical or content-reference
validation. The broader source-manifest rule/reference corpus remains
conditional.

The prior-run source-count boundary target then ran from clean source tip
`7741a07e`:

```text
nix develop -c cargo test -p mfm-journal --lib structured:: -- --nocapture
source: 7741a07e
result: ok — 5 structured-journal unit tests passed, 0 failed
```

Independent review confirms exact/one-over checks for the generated program,
descriptor, rule, and total-reference budgets, plus the empty-descriptor
rejection; the checked reference-count overflow path and empty-program wildcard
semantics remain unchanged. The broader source-manifest semantic corpus remains
conditional.

The native canonical-bound target then ran from clean source tip `17129b21`:

```text
nix develop -c cargo test -p mfm-canonical --lib native_canonical_limit -- --nocapture
source: 17129b21
result: ok — 1 native canonical limit test passed, 0 failed
```

The direct native `serde_json::Value` witnesses accept exact and reject
one-over UTF-8 string, object-key, array-item, and object-entry budgets before
recursive traversal; the object-key budget is generated from the recoverability
annex source. Full `mfm-canonical` targets pass 3 unit, 11 canonical-JSON, and
5 recoverability integration tests, and the focused `mfm-facts` target passes
14/14. The outer 32 MiB canonical-document bound remains a separate contract
layer.

The typed base64url-bound target then ran from clean source tip `26bbb44d`:

```text
nix develop -c cargo test -p mfm-canonical --test canonical_json base64url_bytes_accept_exact_character_budget_and_reject_one_over -- --nocapture
source: 26bbb44d
result: ok — 1 base64url boundary test passed, 0 failed
```

The parser rejects the generated one-character-over input before scanning or
decoding, while exact input decodes to the generated 16,777,216-byte payload.
The enclosing canonical/retained payload bounds remain responsible for trusted
`CanonicalBytes::new` construction.

The unsigned-native primitive cutover then ran from clean source tip
`6f135845`:

```text
nix develop -c cargo test -p mfm-canonical -- --nocapture
source: 6f135845
result: ok — 3 unit, 12 canonical-json, 6 recoverability integration, and 3 doctests passed, 0 failed

nix develop -c cargo test -p mfm-facts --lib -- --nocapture
source: 6f135845
result: ok — 14 fact tests passed, 0 failed
```

The generator rerun is byte-stable; only the primitive canonical-value schema
identity changes (old `671101…` to current `b2477c…`), while the other 52
schema identities remain unchanged. The native codec rejects `-1` and
`i64::MIN`, and fact scalar construction rejects both positive and negative
`CanonicalValue::Signed` producers instead of allowing a nonnegative value to
silently round-trip as unsigned.

The plain canonical-document bound then ran from clean source tip `da2b41a4`:

```text
nix develop -c cargo test -p mfm-canonical --test canonical_json plain_json_accepts_exact_document_budget_and_rejects_one_over_before_parse -- --nocapture
source: da2b41a4
result: ok — 1 plain canonical-document boundary test passed, 0 failed

nix develop -c cargo test -p mfm-canonical -- --nocapture
source: da2b41a4
result: ok — 3 unit, 13 canonical-json, 6 recoverability integration, and 3 doctests passed, 0 failed
```

The one-byte-over input is rejected before number scanning, JSON parsing, or
canonical allocation; the exact 33,554,432-byte input remains accepted. The
facts, values, journal, and store package targets also pass on this source tip.

The detached provider-proof cutover then ran from clean source tip `618cadea`:

```text
nix develop -c cargo test -p mfm-storage-evm-postgres --lib mutation_proof_tests --no-fail-fast
source: 618cadea
result: ok — 3 detached Completion proof tests passed, 0 failed

nix develop -c cargo check -p mfm-storage-evm-postgres
source: 25dc49e7
result: ok
```

The pure path performs no `PgConnection` work. The corpus installs the signed
proof in the completion closure, rehydrates typed request/state-input values,
derives the provider preimage from those closure bytes, and rejects crypto,
payload, target-policy, canonicality, challenge, non-ASCII, and generated
budget substitutions. Reload callers perform the separate historical
incarnation lookup; deployment/provider trust and production offline/public-
result matrices remain conditional.

The managed SQL historical-incarnation regression then ran from clean source
tip `f3a15978`:

```text
nix run .#run -- --task wallet-nonce-postgres-storage-qualification
source: f3a15978
run id: run-2331854-1786010138076393417
result: ok — 1 task, 4 tests passed, 0 failed in 676.89s
```

The long SQL scenario deletes the registered historical row, asserts the
persisted completion fails closed, restores it, rewrites only the stored
incarnation JSON, asserts failure again, restores the exact column value, and
confirms status recovery. At this historical revision the remaining EVM-09
scope was deployment and provider trust plus an independent production
offline/public-result audit; `f5815ccb` supplied the repository-local
detached audit, and `6cd74eef` subsequently cleared the production-scale
closure-size blocker.

The latest default-concurrency managed qualification ran from clean source
tip `897ec4b8` (documentation-only evidence refresh after implementation
revision `9f61c39a`):

```text
nix run .#run -- --task recoverability-postgres-v1
run id: run-2246603-1786002530958490012
result: ok — 24 structured-history tests passed, 0 failed in 43.42s
```

The later focused revisions `e5f8c3b0`, `219c588b`, and `49d2e77c` change only
shared ingress predicates and their unit proofs; they do not change the managed
configuration fixture exercised by this run.

The provider-boundary budget targets ran from the focused revisions:

```text
nix develop -c cargo test -p mfm-evm --lib provider_attestation -- --nocapture
source: a1ad9814
result: ok — 1 provider-attestation unit test passed, 0 failed

nix develop -c cargo test -p mfm-storage-evm-postgres --lib provider::frame_tests -- --nocapture
source: 75ac74f5
result: ok — 6 provider-frame tests passed, 0 failed
```

Independent review confirms the generated 4,096-byte provider-proof and
1,024-byte finish-authorization budgets, the 64-route assembly budget, and
the per-route proof budget are checked before persistence or assertion
completion; the finish-authorization over-limit test uses a fresh matching
digest, so digest mismatch cannot explain the rejection. The broader
offline/provider-attestation audit remains conditional.

The completion-recovery boundary target ran from clean source tip `2512ebf8`:

```text
nix develop -c cargo test -p mfm-evm --lib completion_recovery_limit -- --nocapture
source: 2512ebf8
result: ok — 1 completion-recovery boundary test passed, 0 failed
```

Independent review confirms both canonical completion-recovery paths apply the
generated 524,288-byte bound before returning the closure; the broader
offline/public-result audit remains conditional.

Independent review of exact `8a902d03` passes the current STORE-06 inventory
scope. The AST visitor recognizes checked and `_unchecked` SQLx query macros,
fails closed on all six `query_file*` forms, uses SQLx's correct typed argument
positions, recognizes direct/renamed/glob imports and after-use declarations in
file/module/block scopes, and restores import and `QueryBuilder` binding state
on scope exit; the source and syntax fixture tests remain 2/2. The broader
independent ownership and scale audit remains conditional.

The final scoped SQL inventory check ran from clean source tip `8a902d03`:

```text
nix run .#run -- --task postgres-sql-inventory-check
source: 8a902d03
run id: run-2236542-1786001395306859242
result: ok — 2 inventory tests passed, 0 failed in 2.01s
```

The clean source sequence immediately before this checkpoint-only change also
refreshed the source-local inventory and portable corpus leaves:

```text
nix run .#run -- --task postgres-sql-inventory-check
source: b38ae5c3
run id: run-2141603-1785989203587902728
result: 2 inventory tests passed, 0 failed in 1.96s

nix run .#run -- --task portable-replay-corpus
source: b38ae5c3
run id: run-2144669-1785989584718916467
result: 10 replay corpus tests passed, 0 failed in 21.14s
```

The focused SQL inventory check on `7e467952`, `40051076`, `1edcf7bb`,
`018a946a`, `836e9c31`, `2fea7802`, and `8a902d03` passes both the source inventory and
syntax-fixture tests (2/2). Its AST visitor exercises generic scalar, generic
`query_as`, checked and `_unchecked` query macros, all six `query_file*` forms,
direct/renamed/glob and after-use imports, nested scope restoration, wrapped
generic, and `QueryBuilder` fragment forms; the broader ownership and scale
audit remains conditional.
Full independent ownership/scale proof remains conditional.

The purpose-isolation cutover was qualified on its exact clean tip:

```text
nix develop -c cargo test -p mfm-app --test application-privacy-ui
source: bd98e8ca
result: 14/14 compile-fail cases passed

nix develop -c cargo test -p mfm-store --lib
source: bd98e8ca
result: 33 passed, 0 failed

nix develop -c cargo test -p mfm-replay --lib
source: bd98e8ca
result: 9 passed, 0 failed

nix develop -c cargo test -p mfm-app --lib
source: bd98e8ca
result: 31 passed, 0 failed

nix run .#run -- --task portable-replay-corpus
source: bd98e8ca
run id: run-2057202-1785982481856354937
result: ok — 1 task, 0 failed in 21.60s

nix run .#run -- --task postgres-sqlx-check
source: bd98e8ca
run id: run-2049878-1785982143970760651
result: ok — 1 task, 0 failed in 99.91s
```

The offline-fold boundary cutover was then qualified on its exact clean tip:

```text
nix develop -c cargo test -p mfm-app --test application-privacy-ui
source: a1a78b84
result: 16/16 compile-fail cases passed

nix develop -c cargo test -p mfm-store --lib
source: a1a78b84
result: 33 passed, 0 failed

nix develop -c cargo test -p mfm-replay --lib
source: a1a78b84
result: 9 passed, 0 failed

nix develop -c cargo fmt --all -- --check
source: a1a78b84
result: pass

nix run .#run -- --task portable-replay-corpus
source: a1a78b84
run id: run-2069857-1785983372700769978
result: ok — 1 task, 0 failed in 20.76s
```

The cutover keeps the complete verified cursor, records, objects, and live
bindings inside `mfm-store`. Replay consumes only the opaque summary's status
and bounded identifier/fact-route metadata before producing recorded evidence;
the 16-case application matrix rejects both frontier access and importing the
removed full-fold type.

The cutover preserves the transport status strings while preventing callers
from reading actionable state, capability references, input, or execution
details through public or recorded evidence. Independent review of exact
`bd98e8ca` found no direct leak or contract mismatch and confirmed the
exhaustive five-tag mapping. A complete runtime/audit/replay/export isolation
matrix remains outside this focused proof; the explicit offline fold seam is
now opaque and is covered by the exact follow-up checks above. Independent
review of exact `a1a78b84` found no implementation gap: the full verified
cursor, records, objects, and live bindings remain store-private, while replay
retains its bounded route/dependency checks. This is PASS for the API scope;
TT2-APP-01 remains Conditional for the broader matrix.

The later-audit suffix proof was then qualified on exact `eb38ea44`:

```text
nix develop -c cargo test -p mfm-store --lib
source: eb38ea44
result: 33 passed, 0 failed

nix develop -c cargo test -p mfm-replay --lib
source: eb38ea44
result: 10 passed, 0 failed

nix develop -c cargo clippy -p mfm-replay --all-targets -- -D warnings
source: eb38ea44
result: pass

nix develop -c cargo fmt --all -- --check
source: eb38ea44
result: pass
```

Independent review of exact `eb38ea44` passes the REPLAY-03 implementation
scope: the store-shaped fixture contains admission, a committed Read
authorization, and a later Returned observation; Audit folds offline with
byte-identical projection, while carrying that physical suffix as Semantic is
rejected. The fixture uses `StructuredMemoryBackend` test support. Revision
`1ca2f7cd` then retains the exact bytes as a generator-owned corpus artifact;
its corpus test rebuilds the stream through `observed_read_export(203)`, runs
offline verification, and compares projection bytes. Live
Postgres/application/multi-process evidence remains conditional under
REPLAY-03/05.

Additional focused evidence on the current tree:

```text
nix run .#run -- --task postgres-sql-inventory-check
run id: run-1981332-1785976730139548702 — ok, 1/1 task in 2.67s
nix run .#run -- --task postgres-sqlx-offline-check
run id: run-1981529-1785976737932619794 — ok, 1/1 task in 5.07s
nix develop -c cargo test -p mfm-keystore
91 unit tests and 9 doctests passed on `8b2e64ba`
nix develop -c cargo test -p mfm-keystore decrypt_ -- --nocapture
8 decrypt-focused tests passed on `8b2e64ba`
nix develop -c cargo test -p mfm-keystore decrypt_oversized_ciphertext_rejects_before_allocating_plaintext -- --nocapture
1 focused test passed on `c832e5d8`
nix develop -c cargo test -p mfm-keystore qualification_rejects_a_valid_key_with_wrong_public_and_account_identity -- --nocapture
1 focused test passed on `c832e5d8`
nix develop -c cargo test -p mfm-keystore
93 unit tests and 9 doctests passed on `c832e5d8`
nix develop -c cargo clippy -p mfm-keystore --all-targets -- -D warnings
pass on `c832e5d8`
nix develop -c cargo test -p mfm-evm completed_wallet_nonce_retains_rehashable_public_recovery_closure -- --nocapture
1 focused test passed
nix develop -c cargo test -p mfm-app --test application-privacy-ui
12 application privacy trybuild cases passed on `2fc4afa8`
nix develop -c cargo test -p mfm-replay generated_portable_artifact_corpus_round_trips -- --nocapture
pass on `1ca2f7cd` (retained observed-read artifact and semantic-suffix rejection)
nix develop -c cargo test -p mfm-replay --lib
10 passed on `1ca2f7cd`
python3 contracts/recoverability/generate.py
deterministic regeneration passed on `1ca2f7cd`
nix develop -c cargo fmt --all -- --check
pass on `c832e5d8`
```

The wallet run started after `266d6889` and no commits were made during it,
so its focused source and result align. It is not a replacement for the
historical full composed gate.

The historical composed run started on `82e474ca` with no commits during the
run, so its source hash and closing-source-revision observation agree. That
run remains the exact source gate for `82e474ca`; the later focused revisions
change production/runtime behavior and are evidenced separately above rather
than being treated as retroactively covered by the composed gate.
The earlier
`run-1538729` began before the unrelated `ef3e412d5` documentation commit and
is retained only as historical evidence, not as the current gate.

The diagnostic gate sequence is retained for provenance: a same-stream
configuration race exposed a mismatch between a canonical-revision checkpoint
head and a domain content-reference digest; `e68aca910` classifies the
predecessor with the checkpoint's canonical digest. A follow-on transient
commit-before-external-ack read window exposed `InvalidHistory`; `fcd56ab09`
adds bounded retries only for that read classification and preserves the final
error for persistent mismatches. `f99e3a84c` updates the static SQL ownership
predicate for the wrapper without changing SQL text or query behavior.
`6d4f48933` extends the bounded read classification through application
configuration resolution and retries an ambiguous append only with identical
canonical bytes through the qualified backend. `4e11e3550` strengthens the
durable-row/unknown-acknowledgement fake and verifies that recovery retains one
row. `5711097b`, `74bfa335`, and `6284e8d9` add the generated portable corpus,
offline-fold acceptance, production-shaped suffix, and nested source-graph
vectors; `82e474ca` adds the bounded wallet projection cutover, and the current
exact gate exercises that reviewed source.

The new regressions are post-`a4dada89`; that historical baseline has no
corresponding test cases, so “fails against baseline” is recorded as *not
applicable* rather than inferred. Existing API rejection tests preserve the
baseline negative behavior.

The current `mfm-store` focused suite passes 33/33, including the two
configuration recovery tests (7/7 in their module); formatting and diff checks
also pass. The exact composed run independently passes all 13 leaves.

### Named focused regression matrix

| Regression | Current result | Against `a4dada89` |
| --- | --- | --- |
| `portable::tests::oversized_and_monolithic_documents_fail_before_full_decode` | pass | N/A; introduced after baseline |
| `portable::tests::exact_frame_limit_succeeds_and_one_byte_over_fails` | pass | N/A; introduced after baseline |
| `portable::tests::exact_total_limit_succeeds_and_one_byte_over_fails` | pass | N/A; introduced after baseline |
| `portable::tests::golden_frame_stream_rejects_omission_extra_substitution_reordering_and_stale_head` | pass | N/A; introduced after baseline |
| `portable::tests::serialized_authorization_decision_tampering_is_rejected` | pass | N/A; introduced after baseline |
| `portable::tests::serialized_recursive_prefix_tampering_is_rejected` | pass | N/A; introduced after baseline |
| `portable::tests::generated_portable_artifact_corpus_round_trips` | pass | N/A; introduced after baseline |
| `portable::tests::generated_nested_source_graph_vectors_exercise_expander` | pass | N/A; introduced after baseline |
| `portable::tests::production_store_export_folds_offline_and_preserves_projection_bytes` | pass | N/A; introduced after baseline |
| `structured::fold::source_bound_tests::distinct_source_bound_rejects_before_insert` | pass | N/A; introduced after baseline |
| `structured::purpose::export_source_closure_tests::fact_route_bound_rejects_before_insert` | pass | N/A; introduced after baseline |
| `structured::purpose::export_source_closure_tests::multi_hop_shared_and_deterministic` | pass | N/A; introduced after baseline |
| `structured::purpose::export_source_closure_tests::cyclic_source_graph_is_rejected` | pass | N/A; introduced after baseline |
| `structured::purpose::export_source_closure_tests::over_budget_source_graph_is_rejected` | pass | N/A; introduced after baseline |
| `structured::purpose::export_source_closure_tests::flattened_multi_hop_source_closure_is_accepted` | pass | N/A; introduced after baseline |
| `structured::purpose::export_source_closure_tests::fanout_pending_bound_rejects_before_enqueue` | pass | N/A; introduced after baseline |
| `application::tests::denied_dependency_export_emits_no_bytes` | pass | N/A; introduced after baseline |
| `structured::configuration::tests::reader_retries_bounded_transient_checkpoint_mismatch` | pass | N/A; introduced after baseline |
| `structured::configuration::tests::writer_retries_identical_append_after_unknown_acknowledgement` | pass | N/A; introduced after baseline |
| `configuration_acceptance_vectors_match_memory_and_postgres` | pass in serialized `run-2240964-1786001827263556370` (24/24 structured-history tests, including 128 values across 16 shape families, 128 revisions, and depth/hostile vectors); latest default-concurrency run `run-2246603-1786002530958490012` also passes 24/24, while two earlier race attempts remain separately recorded | N/A; introduced after baseline |
| `structured::canonical_append::tests::count_bounds_accept_exact_limits_and_reject_empty_or_one_over` | pass in focused `mfm-store` target on `49d2e77c`; records/objects accept exact 65,536, nested arrays/objects exact 1,048,576, and empty/one-over cases are classified before deeper validation | N/A; introduced after baseline |
| `structured::canonical_append::tests::{envelope_frame_accepts_exact_byte_limit,envelope_frame_rejects_one_byte_over_limit}` | pass in focused `mfm-store` target on `49d2e77c` (exact/one-byte-over `MAX_STORED_FRAME_BYTES`) | N/A; introduced after baseline |
| `structured::canonical_append::tests::object_frame_accepts_exact_limit_and_rejects_one_byte_over` | pass in focused `mfm-store` target on `49d2e77c`; exact content-addressed object accepted and shared predicate rejects one byte over | N/A; introduced after baseline |
| `structured::prior_run_source_manifest_limit_tests::exact_manifest_byte_budget_is_accepted_and_one_over_is_rejected` | pass in focused `mfm-journal` target on `70e6900e` (3/3 structured-journal tests); shared encode/decode byte predicate accepts exact 16,777,216 bytes and rejects one byte over | N/A; introduced after baseline |
| `structured::prior_run_source_count_limit_tests::{exact_rule_counts_are_accepted_and_one_over_is_rejected,exact_manifest_counts_are_accepted_and_one_over_is_rejected}` | pass in focused `mfm-journal` target on `7741a07e` (5/5 structured-journal tests); exact/one-over program, descriptor, rule, and total-reference budgets plus empty-descriptor rejection | N/A; introduced after baseline |
| `recoverability::native_canonical_limit_tests::native_string_and_collection_budgets_accept_exact_and_reject_one_over` | pass in focused `mfm-canonical` target on `17129b21` (1/1); direct native `serde_json::Value` exact/one-over string, array, object, and object-key bounds | N/A; introduced after baseline |
| `canonical_json::base64url_bytes_accept_exact_character_budget_and_reject_one_over` | pass in focused `mfm-canonical` target on `26bbb44d` (1/1); generated 22,369,622-character exact input decodes to 16,777,216 bytes and one-over input rejects before decode | N/A; introduced after baseline |
| `recoverability_v1::primitive_canonical_value_rejects_signed_json_numbers` | pass in full `mfm-canonical` target on `6f135845`; native primitive rejects `-1` and `i64::MIN`, while unsigned `1` remains accepted | N/A; introduced after baseline |
| `facts::scalar_subject_and_predicate_are_exact_float_free_annex_values` | pass in focused `mfm-facts` target on `6f135845` (14/14); positive and negative `CanonicalValue::Signed` producers are rejected | N/A; introduced after baseline |
| `canonical_json::plain_json_accepts_exact_document_budget_and_rejects_one_over_before_parse` | pass in focused `mfm-canonical` target on `da2b41a4` (1/1); generated 33,554,432-byte exact input is accepted and one-byte-over input rejects before parse/allocation | N/A; introduced after baseline |
| `wallet_authority::provider_attestation_tests::provider_attestation_accepts_exact_budget_and_rejects_one_byte_over` | pass in focused `mfm-evm` target on `a1ad9814`; exact/one-byte-over generated provider-proof budget | N/A; introduced after baseline |
| `provider::frame_tests::{provider_attestation_accepts_exact_budget_and_rejects_one_byte_over,finish_authorization_accepts_exact_budget_and_rejects_one_byte_over,deployment_route_count_accepts_exact_budget_and_rejects_one_route_over,deployment_route_proof_accepts_exact_budget_and_rejects_one_byte_over}` | pass in focused `mfm-storage-evm-postgres` target on `75ac74f5` (6/6 provider frame tests); valid route references and fresh matching digest isolate the route/proof and finish-authorization over-limit rejections | N/A; introduced after baseline |
| `wallet_authority::completion_recovery_limit_tests::completion_recovery_accepts_exact_budget_and_rejects_one_byte_over` | pass in focused `mfm-evm` target on `2512ebf8` (exact/one-over generated `MAX_COMPLETION_RECOVERY_BYTES`) | N/A; introduced after baseline |
| `configuration_commit_acknowledgement_loss_retries_identical_revision` | pass in `run-2167617-1785992166392324915` (19/19 structured-history tests) | N/A; introduced after baseline |
| `configuration_load_keeps_one_snapshot_across_a_concurrent_append` | pass in `run-2190510-1785994272280747833` (22/22 structured-history tests) | N/A; introduced after baseline |
| `run_snapshot_keeps_one_prefix_across_a_concurrent_transition` | pass in `run-2190510-1785994272280747833` (22/22 structured-history tests) | N/A; introduced after baseline |
| `contention_failures_rollback_before_exact_retry` | pass in `run-2190510-1785994272280747833` (22/22 structured-history tests) | N/A; introduced after baseline |
| `run_commit_acknowledgement_loss_retries_identical_batch` | pass in `run-2194313-1785994645494655426` (23/23 structured-history tests) | N/A; introduced after baseline |
| `prior_run_fact_scan_folds_one_shared_producer_prefix_once` | pass in `run-2206767-1785995910502340324` (24/24 structured-history tests) | N/A; introduced after baseline |
| `current_wallet_projection_uses_bounded_primary_key_lookup` | pass in focused PostgreSQL qualification | N/A; introduced after baseline |
| `real_sql_authority_preserves_activation_nonce_and_role_boundaries` (including long-history plan/row proof, exact wallet-history query-shape checks, and persisted two-candidate closure-only projection) | pass in `run-2218000-1785998748489917662` | N/A; introduced after baseline |
| `completed_wallet_nonce_retains_rehashable_public_recovery_closure` (serialized closure reload) | pass in focused `mfm-evm` test | N/A; introduced after baseline |
| `decrypt_ownership_transfer_is_witnessed_on_success_and_cleanup` (decrypt-to-sign allocation continuity) | pass in full `mfm-keystore` package | N/A; introduced after baseline |
| `decrypt_oversized_ciphertext_rejects_before_allocating_plaintext` | pass in focused `mfm-keystore` test | N/A; introduced after baseline |
| `qualification_rejects_a_valid_key_with_wrong_public_and_account_identity` | pass in focused `mfm-keystore` test | N/A; introduced after baseline |
| `qualified_evm_submission_production_restarts_after_one_broadcast_and_completes` | pass in composed gate | N/A; fresh production path added after baseline |
| `configured_value_history_linearizes_same_stream_append_races` | pass in composed gate | N/A; added after baseline |

The matrix records the focused names rather than collapsing them into category
counts; the baseline had no equivalent source-level tests to run.

## TT2 item-by-item disposition

| Item | Result | Evidence / remaining requirement |
| --- | --- | --- |
| TT2-AUTH-01 | Closed | Workspace-private authority marker and API-surface denial tests prevent ordinary external physical access. |
| TT2-AUTH-02 | Closed | PostgreSQL session authority is externally deployed, move-only, and not a public raw-login capability. |
| TT2-STORE-01 | Conditional | Target/checkpoint trust is explicit and testable; concrete external production fence implementation is absent. |
| TT2-STORE-02 | Conditional | Snapshot/head validation, deterministic run/configuration interleavings, prepared restart, strict acknowledgement, stale-worker sidecar arbitration, run/configuration committed-but-unknown acknowledgement recovery, and injected `40001`/`40P01` rollback classification pass; cross-process acknowledgement, promotion, and production-authority matrices remain. |
| TT2-STORE-03 | Closed | Fresh loads verify indexed run/configuration heads against the folded prefix. |
| TT2-STORE-04 | Closed | Contention classification leaves the aborted transaction; bounded retries reconcile raced identities and unknown configuration acknowledgements with identical bytes. |
| TT2-STORE-05 | Conditional | Shared canonical ingress bounds typed base64url parser input, unsigned-native canonical strings/object keys/collections, batch counts, nested collection counts, serialized configuration revisions, prior-run source manifests and counts, run frames, and retained objects before backend dispatch. Revision `6525fad6` expands the managed memory/PostgreSQL parity to 128 values across 16 canonical shape families and a 128-revision stream, retaining positive, exact-limit, one-byte-over, stale-predecessor, idempotent replay, escaped JSON, exact UTF-8 boundary, exact depth-limit, one-level-over depth, float, duplicate-key, and malformed vectors; serialized run `run-2240964-1786001827263556370` passes 24/24. Revision `9f61c39a` adds exact/one-byte-over envelope witnesses; `78dbc354` plus `e5f8c3b0` add a valid exact-limit object and isolate the shared one-byte-over length predicate; `219c588b` adds exact 65,536-record/object and empty/one-over count witnesses, and `49d2e77c` extends the same predicate to exact/one-over 1,048,576-item nested arrays/objects; the focused canonical-append target passes 5/5. Revision `70e6900e` adds a shared prior-run source-manifest byte predicate with exact 16,777,216-byte and one-byte-over witnesses; `7741a07e` adds exact/one-over 4,096-program, 4,096-descriptor, 1,024-rule, and 65,536-reference count witnesses plus empty-descriptor rejection; the focused journal target passes 5/5. Revision `17129b21` adds generated native string, object-key, array-item, and object-entry limits; `26bbb44d` adds exact/one-over base64url parser bounds; `6f135845` removes the unreachable signed primitive variant and rejects signed fact producers; focused canonical targets pass 3/12/6 and facts pass 14/14. Latest default-concurrency run `run-2246603-1786002530958490012` passes 24/24. Two earlier attempts hit the same-stream race intermittently; the complete generated, hostile, and production-scale acceptance matrix remains unverified. |
| TT2-STORE-06 | Conditional | Revisions `40051076` through `8a902d03` extend the AST inventory to checked and `_unchecked` SQLx query macros, fail closed on all six `query_file*` forms, correct typed SQL-argument positions, direct/renamed/glob and after-use imports, nested file/module/block scope restoration, aliases, generic forms, wrapped helpers, and QueryBuilder fragments (independent review and managed inventory run `run-2236542-1786001395306859242` pass 2/2); a full independent query ownership/scale audit remains. |
| TT2-STORE-07 | Closed | The managed two-publication same-producer witness asserts one dense page, one producer-prefix load, and three folded producer batches per scan invocation; store-scoped counters prevent isolated parallel schemas from contaminating the proof, and bounded byte/work/session limits remain enforced. |
| TT2-EVM-01 | Closed | Fresh production keystore signing/broadcast path exercised by the current release qualification. |
| TT2-EVM-02 | Closed | Recovery observes chain state before broadcasting a retained candidate. |
| TT2-EVM-03 | Closed | Replacement eligibility carries producer-authorized prefix evidence and permits. |
| TT2-EVM-04 | Closed | Exhaustion reconciles authoritative final status before closing. |
| TT2-EVM-05 | Closed | Stable caller intent and separate semantic digest conflict behavior are implemented and tested. |
| TT2-EVM-06 | Closed | Historical registered incarnations support release currentness and promotion. |
| TT2-EVM-07 | Conditional | Managed run `run-2218000-1785998748489917662` holds status and reserve/activate/complete Q/E counts constant after 64 completed reservations, rejects captured Q/P lifetime `COUNT/MAX`, requires exact key/prefix predicates for every wallet-history `SELECT`, and verifies the domain primary-key path under `enable_seqscan = off`; a production latency envelope remains. |
| TT2-EVM-08 | Closed | Retained signer integrity failures remain integrity failures rather than availability outcomes. |
| TT2-EVM-09 | Conditional | Closure/preimage, persisted reload, managed historical-row omission/rewriting, public-result redaction, detached offline audit, and the managed production-scale qualification remain green. `25dc49e7`/`b59d820a`/`618cadea` split the callback-free provider proof check from SQL history lookup and pass a 3/3 persisted-closure Completion corpus without `PgConnection`; `f3a15978` adds the 4-test managed SQL regression; `137ed63b` proves the public run view emits only typed disposition and advertises `PublicOutputs`; `f5815ccb` independently reconstructs the Completion mutation, verifies provider signature/trust/context, and asserts closure-only public bytes with no storage verifier, PostgreSQL, or live provider; `6cd74eef` encodes canonical closure values as raw JSON and the managed production leaf passes 1/1; `64517b46` sends the forged projection closure through the independent verifier; `60554999` adds independent operation-key and target-context substitution denials; `88af683d` adds independent payload, signature/key, challenge, and canonical-wire substitution denials; `923fce06` adds independent context-bound, store-incarnation, proof-size, and unknown-field denials. Deployment-owned provider trust and the broader crash, ambiguity, latency, and production-authority matrices remain conditional. |
| TT2-EVM-10 | Closed | Maximum nonce is rejected before observation and persistence. |
| TT2-EVM-11 | Closed | Pending-floor route/policy and EVM semantics are authority-qualified before mutation. |
| TT2-EVM-12 | Conditional | `bc469cc8` injects a status-137 worker exit after the durable broadcast observation; the resumed managed qualification closes the run and proves exactly one raw broadcast (`run-2394128-1786018130470251941`, 1/1). Receipt, finality, promotion, completion, ambiguity, cross-process, replacement, and scale matrices remain incomplete. |
| TT2-REPLAY-01 | Conditional | Recursive proof/fixation/trust/bounds/parity and serialized source-prefix tamper coverage are implemented; live app multi-hop production proof remains. |
| TT2-REPLAY-02 | Closed | Reproduction owns no hidden current-history comparison; old `compare_current` capability is deleted. |
| TT2-REPLAY-03 | Conditional | Exact semantic cutoff and kind-aware authorization cutoff are implemented; the retained store-shaped authorization/observation artifact passes Audit offline parity and Semantic rejection. Live production evidence remains. |
| TT2-REPLAY-04 | Closed | Frame and total budgets accept exact limits and reject one-byte-over before allocation. |
| TT2-REPLAY-05 | Conditional | Generated schema vectors, a 17-vector portable artifact corpus including retained observed-read audit bytes, two nested source-graph vectors, the generated offline-fold acceptance leaf, and online/offline parity pass; the complete live matrix remains. |
| TT2-APP-01 | Conditional | Public, recorded-replay, and offline replay products expose only fold-derived status plus bounded export metadata; the 16-case application privacy trybuild matrix rejects raw-record, frontier, and full verified-run access, and `137ed63b` adds a serialized EVM public-result redaction regression. Complete runtime/audit/replay/export isolation proof remains outstanding. |
| TT2-APP-02 | Conditional | Flattened recursive closure is kind-aware, fixed-point, graph-checked, and principal/grant/decision-bound; root/dependency app unit tests prove zero-byte denial, but live production multi-hop proof remains. |
| TT2-SEC-01 | Conditional | The shared production decrypt guard and witnesses cover one protected heap allocation, source-to-`SecureKey` handoff, cleanup on success, truncated and oversized ciphertext, ciphertext/tag/AAD authentication failure, injected unwind, authenticated post-decrypt invalid-key rejection, and explicit rejection of a valid matching-account key with the wrong public key plus a valid wrong-account key; external termination/OOM/resource-failure classes remain. |
| TT2-QUALITY-01 | Conditional | Superseded runtime/replay paths are deleted; ownership and hand-written LOC remain concentrated. |
| TT2-VERIFY-01 | Conditional | Broad/focused gates pass, but the plan's complete authority/fault/offline/security matrix is incomplete. |
| TT2-PROCESS-01 | Conditional | Evidence is now revision-pinned and honest; a PASS is withheld until the residuals close. |

## Boundary review

The marker boundary protects `RuntimeHistoryPort`, physical binding,
`ProgramVerifier`, wallet nonce authority, external checkpoints, deployment
credentials, retained release trust, and store/checkpoint trust. Portable
encoding is owned by the replay consumer; callers cannot inject trust
implementations or fabricate private fragments. PostgreSQL checkpoint work
validates the exact target tuple in the transaction. Provider mutation proofs
are canonical, signed, target-bound, and rechecked on candidate/completion
reload against registered historical incarnations.

The flattened recursive source list is now cutoff-aware and sealed against the
full reachable graph: graph validation rejects omissions, substitutions,
disconnected extras, cycles, root edges, tenant/store/target mismatches, and
cutoff/head inconsistencies. Production discovery uses a pure bounded expander
with a fixed-point requeue when a shared source is later required at a higher
head; each selected prefix retains an authenticated principal, fixed export
grant, and opaque content-addressed decision reference. The fold rejects the
first distinct producer beyond `MAX_PORTABLE_SOURCE_RUNS`, and route collection
rejects the first fact route beyond `MAX_PORTABLE_FACT_ROUTES`, so neither
source discovery nor route materialization grows past its named bound.

The PostgreSQL append path now compares idempotent configuration predecessors
using the same canonical-revision digest as the external checkpoint. Structured
readers and writers perform one backend snapshot call; PostgreSQL owns the
single bounded eight-attempt retry for the classified commit-before-ack
`InvalidHistory` window. Persistent mismatches and all other errors remain
fail-closed. The SQL inventory review covers the wrapper's direct queries
without changing their text or ownership semantics. Configuration resolution
and ambiguous configuration appends use their shared finite eight-attempt
classification and retry only the exact canonical revision. The bound is per
nested loop, and persistent `InvalidHistory` is retried until exhaustion; both
are explicit bounded residuals rather than fallback behavior.

Public and recorded-replay purpose products retain only `RunEvidenceStatus`;
the internal `StructuredFrontier` and its actionable state/capability details
are consumed before the purpose wrapper is constructed. The application and
replay projections use the status tag directly, and the focused UI matrix
rejects frontier access for both products.

The offline replay entry point now returns an opaque `OfflineVerifiedRun` with
recorded status and bounded export metadata. The full `VerifiedStructuredRun`,
cursor, object graph, and live bindings are no longer exported; replay's
callback-free fold validation consumes only the opaque summary.

## PostgreSQL and EVM matrix status

- PostgreSQL role, snapshot, deterministic interleaving, checkpoint,
  configured-value race, retry, SQL inventory/offline, and managed
  recoverability lanes pass, including the exact fact-scan work-bound run
  `run-2206767-1785995910502340324` on `bb002b19`.
  Cross-process acknowledgement-loss, promotion, copied-target, and broader
  injected-fault matrices are not complete.
- EVM domain and storage tests plus the current managed release qualification
  pass, including fresh production keystore signing/broadcast, provider-proof
  corruption, promotion/reload, recovery observations, exact bounded
  long-history query-shape checks, and indexed projection availability. A full
  kill-point matrix, provider ambiguity matrix, production latency envelope,
  and every different-run crash boundary remain unexecuted.

## Portable corpus status

Implemented and passing:

- root-only/no-source portable verification;
- recursive store fixture and PostgreSQL one-hop online/offline parity;
- exact frame and bundle bounds with one-over rejection;
- canonical frame/chain tamper checks;
- root physical-target and tenant trust denials; and
- omission, extra, substitution, reorder, and stale-head checks at the frame
  and graph-validation layers;
- principal/grant/decision retention and strict authorization-decision tamper
  rejection; and
- a generated 17-vector portable artifact corpus (recursive accept,
  offline-fold acceptance, omitted/extra/reordered/substituted source negatives,
  semantic/audit suffix handling, false frontier/publication negatives, source
  identity/fixation collisions, an over-budget source-count negative, and the
  retained store-shaped observed-read audit artifact) with
  no-service replay; and
- generated shared-DAG acceptance and nested-cycle rejection vectors that
  exercise the actual source-closure expander.

Still required by the plan:

- complete live Postgres/application replay evidence. The retained
  store-shaped suffix artifact now proves the fold and cutoff behavior in the
  generated corpus;
- live production application multi-hop export and zero-byte denied-dependency
  integration assertion; and
- concrete production retained-release/checkpoint trust implementations.

## Deletion, redaction, and size audit

An `rg` audit over `crates`, `bin`, and `tests` found no production use of the
removed authority names, raw session openers, self-fence permits,
repeated-prefix implementation, or monolithic portable encoder. `compare_current`
appears only in deliberate negative parsing/transport tests. Historical plans
and review documents retain those terms as audit vocabulary.

No new secret logging or persistence was found in manifests, events, artifacts,
portable frames, CLI/API output, or error details; the redaction/keystore tests
remain in the composed gate. The implementation range from `a4dada89` adds
approximately 18,607 lines and removes 4,652 lines (including generated/test
and documentation changes); this is a coarse `git diff --numstat` measure, not
hand-written production LOC. The growth and large storage modules are retained
as a quality residual rather than hidden as a simplification win.

## Final verdict

The current implementation is materially stronger, all recorded focused checks
are green on their applicable revisions, and the exact composed gate for
`82e474ca` is green. The strict plan acceptance condition is not met. The
review remains **CONDITIONAL / INCOMPLETE** until the residual proof matrices,
live app integration, production
trust deployment, the remaining EVM crash/ambiguity/latency/authority
matrices, remaining
keystore external termination/OOM/resource-failure witnesses, and ownership evidence are supplied or the
normative plan is deliberately amended.
