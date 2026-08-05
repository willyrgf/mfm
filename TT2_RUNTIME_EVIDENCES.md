# TT2 runtime remediation evidence

This is the durable, append-style evidence ledger for
`IMPL_PLAN_PROBLEMS_TT2_IMPLRFC_RUNTIME.md`. It records the exact revisions that
were independently reviewed, the fixes that followed failed checkpoints, and
the final source and verification candidates. The detailed skeptical review is
kept in [`docs/tt2-independent-review.md`](docs/tt2-independent-review.md).

## Material uncertainties

- The unpublished (`publish = false`) `mfm-authority-seal` package is treated as
  a workspace deployment boundary. Distributing the marker package separately
  would reopen the sealed authority seams and require a new review.
- The final SQLx/offline, SQL-inventory, and model-check results were reported
  by the implementation owner and were not independently reproduced in the
  final skeptical review.
- The long release EVM qualification was independently run on the
  behavior-identical `55f1cafac` source; the later `7f2a792a` source change is
  lint/encapsulation-only and was covered by the full workspace Clippy run.

## Ordered implementation revisions

These are the twelve implementation commits required by the plan. Each commit
left one current design and kept its tests, generated contracts, and
documentation in the same cutover.

| Plan step | Revision | Subject |
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

## Independent review checkpoints

The review files are evidence-only commits. A `FAIL` is retained rather than
rewritten: it identifies the exact candidate that was rejected and the later
implementation revisions that closed its findings.

| Checkpoint | Exact implementation candidate | Independent evidence commit | Gate | Follow-up |
| --- | --- | --- | --- | --- |
| Initial TT2 cutover | `d9e53a5e96749f667656a94af5c2bcec2aea1ddb` | `258059180` | **FAIL** | Wallet projection and replay guards followed. |
| Wallet projection guards | `774f0b7429b22d0a145b28a6f92a528603710593` | `40612039f` | **FAIL** | Budget, signer, authority, and checkpoint repairs followed. |
| Retry/authority candidate | `4fc1baea0719ae6b88837b3a0c063895a70dffcd` | `16fe501a` | **FAIL** | Replay trust, retained-proof reload, and incarnation-registry fixes followed. |
| Registered-incarnation candidate | `55f1cafac9c06f7b23bfdfe1e8bd9f65fa710769` | `9bd6f7056` | **PASS** | Release qualification evidence was refreshed in `0562fd133`. |
| Exact lint-clean candidate | `7f2a792af3ee3f65f0756715e866b0cf136927eb` | `76ce09085` | **PASS** | Final source delta is lint/encapsulation-only; no behavior or public API change. |

The final PASS reviews cover the full TT2 problem index, the authority/API
deletion searches, PostgreSQL checkpoint and retained-proof paths, EVM wallet
promotion/recovery paths, portable replay trust, and the focused regression
tests. The exact review report records every residual and the material
uncertainties above.

## Focused evidence

The exact-candidate review recorded these successful checks in the default Nix
development shell:

- `cargo fmt --all -- --check`;
- full-workspace Clippy with `-D warnings` (1/1, 31.88s);
- the `mfm-store` API trybuild surface (1/1);
- `mfm-replay` unit tests (1/1);
- EVM storage provider tests (2/2) and EVM domain tests (47/47);
- recoverability PostgreSQL qualification (1/1, 37.54s);
- wallet-nonce PostgreSQL storage qualification (1/1, 193.33s); and
- release EVM submission qualification (1/1, 1227.58s) on the
  behavior-identical source noted above.

The managed qualification covers role separation, physical-target and
promotion checks, persisted candidate/completion provider-proof tampering, and
restoration after corruption. The source-only delta to `7f2a792a` is covered by
the exact full-workspace Clippy run.

## Final composed gate

The exact final source candidate was `7f2a792af3ee3f65f0756715e866b0cf136927eb`;
`76ce09085c1c11e6b70c9960e714b1c237ddf854` is the review-only evidence tip.
The final composed gate was run as:

```text
nix run .#ci
run id: run-1385266-1785930159337331421
result: ok — 12 passed, 0 failed in 1831.86s
```

The passing leaves were formatting, Clippy, Cargo metadata, SQLx offline
metadata, workspace tests and doctests, PostgreSQL SQLx checks, recoverability,
wallet storage qualification, structured EVM submission qualification, Bitcoin
parity, and the closing-source-revision check. Two earlier composed attempts
exposed a timing-sensitive recoverability race; the standalone leaf passed, and
the unchanged final composed run passed the complete suite.

## Final disposition

**PASS** for the reviewed implementation candidate. The implementation and
its exact skeptical review are revision-pinned above; this ledger commit is
documentation-only and does not alter the reviewed source behavior.
