# TT2 independent review evidence

Status: **PASS**

This commit records a fresh independent review of the exact frozen candidate
`55f1cafac9c06f7b23bfdfe1e8bd9f65fa710769`. It is evidence-only and does not
change implementation behavior.

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
- The long release EVM submission qualification was run on the immediate
  predecessor (`52d58907b`) and passed. The current candidate adds only the
  historical-incarnation registry resolution and its ACL/schema updates; the
  current managed wallet qualification exercises that path, promotion, and
  provider-proof tamper checks. Re-running the release lane on this exact hash
  would remove this remaining execution-evidence uncertainty.

## Frozen candidate and scope

- Candidate: `55f1cafac9c06f7b23bfdfe1e8bd9f65fa710769`
- TT2 implementation baseline: `07b9d7311daae32230d7a487aa82e07f0d27ff2b`
- Reviewed cutover commits include authority sealing (`cc49802ec`), retained
  provider mutation evidence (`dd0522646`), physical-target fixation
  (`c851a2dab`), proof-bearing wallet checkpoint support (`812cf5f23`),
  idempotent PostgreSQL retry reconciliation (`4fc1baea0`), replay trust and
  export-consumer sealing (`88523caa4`), retained-proof reload verification
  (`52d58907b`), and registered-incarnation resolution (`55f1cafac`).
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

Focused commands were run in the default Nix development shell against the
exact candidate. Nix automatic GC thresholds were set to zero for these
invocations only, avoiding an invalid-store-path race; no project behavior was
changed.

| Check | Result |
| --- | --- |
| `git diff --check 07b9d731^..55f1cafa` | pass |
| `nix develop -c cargo fmt --all -- --check` | pass |
| `nix develop -c cargo test -p mfm-store --test api-surface -- --nocapture` | pass (1/1 trybuild harness, including sealed program/authority surfaces) |
| `nix develop -c cargo test -p mfm-replay --lib -- --nocapture` | pass (1/1) |
| `nix develop -c cargo test -p mfm-storage-evm-postgres --features parity-tests --lib -- --nocapture` | pass (2/2; 1 schema probe ignored as DB-gated) |
| `nix develop -c cargo test -p mfm-evm --lib -- --nocapture` | pass (47/47; run on the preceding proof-verification commit, unchanged by the registry-resolution cutover) |
| `nix run .#run -- --task recoverability-postgres-v1` | pass (managed PostgreSQL on this candidate, 1/1; 37.54s) |
| `nix run .#run -- --task wallet-nonce-postgres-storage-qualification` | pass (managed PostgreSQL on this candidate, 1/1; 193.33s) |
| `nix run .#run -- --task evm-postgres-submission-qualification` | pass (managed PostgreSQL release EVM restart path on `52d58907b`, 1/1; 1344.07s) |

The current wallet qualification covers the new nonce-role `SELECT` grant,
historical-incarnation lookup, promotion compatibility, candidate and
completion provider-proof corruption, and subsequent restoration. The scoped
`-D warnings` Clippy lane from the preceding candidate passed for
authority-seal, values, canonical, certify, runtime, store, replay, evm-live,
and storage-postgres; it intentionally excluded the existing `large_enum_variant`
warning in `mfm-evm` and existing dead-code warnings in all-target
`mfm-storage-evm-postgres`. Cargo check remained green with those warnings.

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

The `4fc1baea0` path was inspected together with the current managed PostgreSQL
recoverability qualification. `indexed_run_checkpoint_digest` binds the
external run checkpoint to the indexed database head in one target-validated
transaction. `checkpoint_mutations_for_batch` recognizes an external head that
already equals the indexed current run/fact head and emits no rewind mutation;
the contention classifier reconstructs the exact existing batch and
re-acknowledges its checkpoint mutations before returning `ExistingSame`.
The qualification passed, and the release EVM restart qualification on the
immediate predecessor passed with no duplicate broadcast. No concrete retry
regression was observed.

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
  this exact candidate; the release EVM restart qualification passes on the
  immediate proof-verification predecessor.
- EVM domain, replay, formatting, storage-provider unit, and API-surface checks
  pass; existing warning boundaries are recorded above rather than hidden.
- The owner reports the final SQLx/offline, SQL inventory, and model checks pass;
  those remain owner-supplied evidence rather than independently reproduced
  results here.

## Verdict

**PASS.** The exact candidate closes the prior public authority bridges,
retained provider-proof reload gaps, physical-target substitution gap, and
portable replay trust gaps under the documented workspace-private deployment
boundary. Focused and managed checks pass, including promotion and persisted
candidate/completion tamper regressions. The only noted uncertainty is the
unrerun long release lane on this final source hash; its changed behavior is
covered directly by the current wallet qualification and the predecessor lane
passed.
