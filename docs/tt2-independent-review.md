# TT2 independent review evidence

Status: **PASS**

This commit records a fresh independent review of the exact frozen candidate
`7f2a792af3ee3f65f0756715e866b0cf136927eb`. It is evidence-only and does not
change implementation behavior. The candidate delta after the prior reviewed
source (`55f1cafac`) is lint/encapsulation-only: it scopes the existing EVM
large-enum lint and makes the retained-proof verifier helper private with a
grouped argument context; it changes neither behavior nor public API.

## Material uncertainties

- This review applies a strict external/adversarial-consumer threat model and
  treats the unpublished (`publish = false`) `mfm-authority-seal` package as a
  hard workspace deployment boundary. If these crates are distributed with a
  separately obtainable marker package, downstream implementations could
  reopen the sealed authority seams and require a new review.
- The database SQLx/offline, SQL inventory, and model tasks were not
  independently reproduced here. The implementation owner reports the final
  `postgres-sqlx-offline-check`, `postgres-sql-inventory-check`, and
  `nix run .#model-check` tasks pass; the reported model hash begins `d8ede667`.
- The long release EVM submission qualification was independently reproduced on
  the prior behavior-identical source (`55f1cafac`; 1/1 below). It was not
  rerun after the lint/encapsulation-only delta in `7f2a792a`; no material
  execution uncertainty is introduced by that delta. The remaining
  owner-supplied uncertainty is the SQLx/offline, SQL inventory, and model
  reports.

## Frozen candidate and scope

- Candidate: `7f2a792af3ee3f65f0756715e866b0cf136927eb`
- TT2 implementation baseline: `07b9d7311daae32230d7a487aa82e07f0d27ff2b`
- Reviewed cutover commits include authority sealing (`cc49802ec`), retained
  provider mutation evidence (`dd0522646`), physical-target fixation
  (`c851a2dab`), proof-bearing wallet checkpoint support (`812cf5f23`),
  idempotent PostgreSQL retry reconciliation (`4fc1baea0`), replay trust and
  export-consumer sealing (`88523caa4`), retained-proof reload verification
  (`52d58907b`), registered-incarnation resolution (`55f1cafac`), and the
  lint/encapsulation cleanup (`7f2a792a`).
- Worktree was clean before this evidence file was added.
- Scope: canonical append and fold authority, Runtime assembly, replay/export
  provenance, EVM wallet authority, PostgreSQL target/checkpoint authority,
  idempotent retry recovery, SQL inventory, and generated recoverability and
  schema-depth budgets.

The candidate now has one workspace-private authority boundary for Runtime
history, physical binding, deterministic program verification, wallet authority,
PostgreSQL checkpoints, deployment credentials, offline replay trust, and the
portable encoder consumer. Provider activation and completion mutations retain a
canonical signed envelope. Reload verifies the exact proof-cleared mutation
preimage, provider signature, schema and current database OID, lineage and
historical epoch, and the full signed store incarnation against the append-only
local registry. Portable exports retain the exact physical target and cannot be
encoded or offline-verified with caller-supplied trust implementations.

## Verification evidence

Focused commands, including the full-workspace Clippy lane, were run in the
default Nix development shell against the exact candidate. The managed
qualification rows identify the prior behavior-identical source where they
were run. Nix automatic GC thresholds were set to zero for these invocations
only, avoiding an invalid-store-path race; no project behavior was changed.

| Check | Result |
| --- | --- |
| `git diff --check 07b9d731^..7f2a792a` | pass |
| `nix develop -c cargo fmt --all -- --check` | pass |
| `nix run .#run -- --task clippy` | pass (full workspace, all libraries/examples/tests/benches and features, `-D warnings`; 1/1; 31.88s) |
| `nix develop -c cargo test -p mfm-store --test api-surface -- --nocapture` | pass (1/1 trybuild harness, including sealed program/authority surfaces) |
| `nix develop -c cargo test -p mfm-replay --lib -- --nocapture` | pass (1/1) |
| `nix develop -c cargo test -p mfm-storage-evm-postgres --features parity-tests --lib -- --nocapture` | pass (2/2; 1 schema probe ignored as DB-gated) |
| `nix develop -c cargo test -p mfm-evm --lib -- --nocapture` | pass (47/47; run on the preceding proof-verification commit, unchanged by the registry-resolution cutover) |
| `nix run .#run -- --task recoverability-postgres-v1` | pass (managed PostgreSQL on behavior-identical source `55f1cafac`, 1/1; 37.54s; not rerun after `7f2a792a`) |
| `nix run .#run -- --task wallet-nonce-postgres-storage-qualification` | pass (managed PostgreSQL on behavior-identical source `55f1cafac`, 1/1; 193.33s; not rerun after `7f2a792a`) |
| `nix run .#run -- --task evm-postgres-submission-qualification` | pass (managed PostgreSQL release EVM restart path on behavior-identical source `55f1cafac`, 1/1; 1227.58s; not rerun after `7f2a792a`) |

The managed wallet qualification on behavior-identical source `55f1cafac`
covers the new nonce-role `SELECT` grant,
historical-incarnation lookup, promotion compatibility, candidate and
completion provider-proof corruption, and subsequent restoration. The
`-D warnings` workspace Clippy lane now passes for the full workspace on this
exact candidate, including all libraries, examples, tests, benches, and
features.
The `7f2a792a` source delta is limited to the existing EVM
`large_enum_variant` lint allowance and a private grouped retained-proof
verifier context; no behavior or public API changed. The release qualification
above therefore remains applicable without a second long run.

## Authority and replay boundaries closed by the workspace-private marker

`RuntimeHistoryPort`, `PublicPhysicalBindingVerifier`, `ProgramVerifier`,
`WalletNonceAuthority`, `ExternalCheckpointAuthority`,
`DeploymentCredentialSink`, `DeploymentCredentialBroker`,
`RetainedPhysicalReleaseTrust`, and `StoreCheckpointTrust` all inherit marker
traits from the unpublished `mfm-authority-seal` package. The Runtime assembly
cutover and portable encoder additionally require workspace-only consumer
markers. The store API trybuild test confirms that ordinary fake
implementations cannot satisfy the public authority/program surfaces.

`ExportRunEvidence` has private fragments and exposes its encoder view only
through the marker-gated method; `PortableRunExport` is built solely by the
private replay consumer. `ReplayTrustSnapshot` cannot be supplied with a
caller-implemented program, retained-release, or checkpoint verifier. Together
with the exact `PhysicalTargetIdentity` in every portable fixation and source
prefix, this closes the previous portable replay provenance finding.

The same marker boundary closes the prior Runtime authority bridge, injectable
live EVM wallet authority, and caller-supplied PostgreSQL target/checkpoint
admission. PostgreSQL admission remains move-only with private credentials, and
checkpoint transactions validate the exact target tuple before commit.

Relevant surfaces: `crates/kernel/authority-seal`,
`crates/kernel/replay/src/portable.rs`,
`crates/kernel/store/src/structured/{assembly,fold,purpose}.rs`,
`crates/kernel/runtime/src/history/port.rs`,
`crates/live/evm/src/structured_wallet.rs`,
`crates/storages/postgres/src/{checkpoint,session}.rs`.

## Provider mutation proof closure

Provider mutation replies are canonical bounded JSON envelopes with a provider
identity, per-channel challenge, full target context, operation key, canonical
payload digest, and Ed25519 signature. Mutation-time code verifies the channel
envelope and signature before attaching the exact proof to the durable candidate
or completion closure. Completion's recovery preimage clears both the outer and
inner completion-proof fields; candidate reload clears its activation-proof
field before recomputing the signed preimage.

Every candidate path (`load_candidates` and `load_candidate_by_key`) and every
completion path (`load_completion` and `load_completion_by_key`) now:

- decodes and canonicalizes the envelope;
- resolves the signed `WalletNonceStoreIncarnation` by lineage and writer epoch
  in `wallet_store_incarnations` using the nonce application role;
- requires the retained row's canonical incarnation reference and JSON to equal
  the signed object exactly;
- checks provider identity, operation key, expected schema, current database
  OID, current lineage, and a historical writer epoch no newer than the current
  authority; and
- verifies the payload digest and provider signature against the exact recovery
  preimage.

The append-only registry lookup preserves proofs from authorized historical
targets after promotion while rejecting same-lineage target, key, or attestation
substitution. Existing managed tests corrupt candidate and completion signatures
in persisted rows and observe an integrity fault; the same qualification then
restores the rows and verifies successful status, promotion, and new mutation
paths.

Relevant surfaces: `crates/storages/evm-postgres/src/{authority,provider,schema}.rs`,
`crates/storages/evm-postgres/migrations/0001_wallet_authority.sql`, and
`crates/storages/evm-postgres/tests/wallet_authority.rs`.

## PostgreSQL retry reconciliation reviewed

The `4fc1baea0` path was inspected together with the managed PostgreSQL
recoverability qualification on behavior-identical source `55f1cafac`.
`indexed_run_checkpoint_digest` binds the
external run checkpoint to the indexed database head in one target-validated
transaction. `checkpoint_mutations_for_batch` recognizes an external head that
already equals the indexed current run/fact head and emits no rewind mutation;
the contention classifier reconstructs the exact existing batch and
re-acknowledges its checkpoint mutations before returning `ExistingSame`.
Both qualifications passed with no duplicate broadcast. The
lint/encapsulation-only `7f2a792a` delta does not alter this behavior. No
concrete retry regression was observed.

## Closed or passing areas observed

- Workspace-private authority markers prevent ordinary downstream
  implementations of Runtime, live-wallet, PostgreSQL, program-verifier, and
  offline replay authority seams.
- Provider attestations are verified at mutation time and at every relevant
  reload, including canonical preimage, signature, current database OID/schema,
  and registered historical target identity.
- Exact physical target identity is carried through structured store identity,
  export evidence, portable fixation, and source-prefix matching.
- Managed PostgreSQL recoverability and wallet-storage qualifications pass on
  behavior-identical source `55f1cafac`; the release EVM submission/restart
  qualification passes there as well, with the exact-candidate delta limited to
  lint/encapsulation and full-workspace Clippy passing on `7f2a792a`.
- EVM domain, replay, formatting, full-workspace Clippy, storage-provider unit,
  and API-surface checks pass; the source delta's lint/encapsulation scope is
  recorded above rather than hidden.
- The owner reports the final SQLx/offline, SQL inventory, and model checks pass;
  those remain owner-supplied evidence rather than independently reproduced
  results here.

## Verdict

**PASS.** The exact candidate closes the prior public authority bridges,
retained provider-proof reload gaps, physical-target substitution gap, and
portable replay trust gaps under the documented workspace-private deployment
boundary. Focused and managed checks pass, including promotion and persisted
candidate/completion tamper regressions; the full workspace Clippy lane passes
on this exact source hash. The release qualification was run on the immediately
preceding behavior-identical source, and the only remaining uncertainty is the
owner-supplied SQLx/offline, SQL inventory, and model reports.
