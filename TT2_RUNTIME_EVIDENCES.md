# TT2 runtime remediation evidence

This append-only ledger records the implementation candidate, focused evidence,
and the independent review for `IMPL_PLAN_PROBLEMS_TT2_IMPLRFC_RUNTIME.md`.
It is deliberately not a blanket pass: the runtime cutover is substantially
implemented and the current source gate is green, but the plan's strict §12/§13
acceptance bar still has identified proof and deployment-boundary gaps.

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
- The application production path has no live multi-hop export integration
  fixture. Store-level recursive fixtures and a real PostgreSQL one-hop parity
  test exist, while the app still materializes a flat source list and
  reauthorizes it transiently.
- The portable closure digest binds tenant, target, heads, graph, and purpose
  data available to the current API, but does not yet retain an authenticated
  principal/grant/decision object as a separately verifiable authorization
  artifact.

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

`ef3e412d5` (`wording in code-quality`) was committed while the earlier gate was
running. It changes only prose and whitespace in `docs/code-quality.md`. The
later `42c80525` whitespace cleanup and `a027640c` evidence refresh are
documentation-only commits. The corrected-source gate below starts after all
three and is pinned to `a7ef6b636`.

## Independent review checkpoints

| Checkpoint | Candidate | Review evidence | Result |
| --- | --- | --- | --- |
| Initial TT2 cutover | `d9e53a5e9` | `258059180` | FAIL; wallet/replay guards followed |
| Wallet projection guards | `774f0b742` | `40612039f` | FAIL; budget/signer/authority fixes followed |
| Retry/authority candidate | `4fc1baea0` | `16fe501a` | FAIL; replay trust/incarnation fixes followed |
| Registered-incarnation candidate | `55f1cafac` | `9bd6f7056` | PASS for that scope |
| Lint-clean candidate | `7f2a792af` | `76ce09085` | PASS for that scope |
| Current implementation candidate | `a7ef6b636acbc3aaa65b26c06563a254b3c1bc1d` | prior review `a027640c90735a4e762fb83eb0d889ebf28176f4` plus this refresh | CONDITIONAL; residuals below |

## Focused and composed verification

All direct Rust tooling was run in the default Nix development shell. The
current source gate was run once on the exact implementation candidate, without
preceding it with separate composed check/test/test-db gates.

| Check | Evidence |
| --- | --- |
| formatting | pass (`nix develop -c cargo fmt --all -- --check`) |
| workspace Clippy | pass (`-D warnings`) |
| metadata and SQLx offline | pass |
| portable replay leaf | pass; `mfm-replay` unit corpus 5/5 |
| workspace nextest and doctests | pass |
| PostgreSQL SQLx check | pass |
| recoverability PostgreSQL v1 | pass |
| wallet-nonce PostgreSQL storage qualification | pass |
| structured EVM submission qualification | pass |
| Bitcoin parity | pass |
| closing source revision | pass |

Exact composed gate:

```text
nix run .#ci
source: a7ef6b636acbc3aaa65b26c06563a254b3c1bc1d
run id: run-1564803-1785945104353521495
result: ok — 13 passed, 0 failed in 1937.35s
structured EVM submission: ok in 1377.19s
```

Additional focused evidence on the current implementation sequence includes
four flattened recursive source-closure tests, five portable replay tests,
exact frame and total-byte limit/one-over checks, root target/tenant trust
tamper denials, and online/offline projection byte parity. The managed
PostgreSQL recursive parity fixture passed 17/17 on the corresponding current
storage sequence. The final composed gate includes the source-bound fix in
`a7ef6b636`; its closing-source-revision leaf observed the post-cleanup tree,
while the run's source implementation pin is the exact hash above. The run
started after `a7ef6b636` and no commits were made during it, so the source pin
and closing leaf agree for this final gate.

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
| REPLAY-01 | Conditional | Recursive source proof, fixation, trust, limits, and parity are implemented; source-prefix tamper/publication corpus and live multi-hop app proof remain. |
| REPLAY-02 | Closed | Reproduction/current-history comparison and the old capability surface are deleted. |
| REPLAY-03 | Conditional | Exact semantic cutoff is enforced; no real later-audit semantic suffix fixture exists yet. |
| REPLAY-04 | Closed | Exact frame/total limits, one-over failures, large-frame and many-small-frame paths pass. |
| REPLAY-05 | Conditional | Generated schema vectors and a no-service test leaf exist; a generated full portable artifact corpus is still missing. |
| APP-01 | Conditional | Purpose-specific evidence types and redaction exist; complete data-isolation proof is not independent. |
| APP-02 | Conditional | Flattened recursive closure is accepted and graph-checked; principal/grant/decision retention and app-level zero-byte denial are not fully proven. |
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

1. add serialized source-prefix false-frontier/publication tamper cases and a
   cyclic/shared/over-budget/omission/substitution portable artifact corpus;
2. add a real semantic export with a later audit suffix, a live app multi-hop
   export integration, and an app-level denied-dependency zero-byte assertion;
3. generate and consume a full portable artifact corpus rather than only schema
   vectors and unit fixtures;
4. bind an authenticated principal/grant/decision into the retained export
   authorization closure;
5. provide concrete production retained-release/checkpoint trust implementations;
6. complete the EVM injected-kill and cross-process PostgreSQL fault/acknowledgement
   matrices; and
7. substantiate same-allocation key lifetime and long-history cost/LOC ownership
   evidence.

Until those items are resolved or the normative plan is deliberately amended,
the honest disposition is **CONDITIONAL / INCOMPLETE**, not PASS.
