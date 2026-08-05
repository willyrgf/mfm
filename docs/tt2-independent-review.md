# TT2 independent review evidence

Status: **FAIL**

This commit records a fresh independent review of the exact frozen candidate
`4fc1baea0719ae6b88837b3a0c063895a70dffcd`. It is evidence-only and does not
change implementation behavior.

## Material uncertainties

- This review applies a strict external/adversarial-consumer threat model. The
  new `mfm-authority-seal` package is workspace-private (`publish = false`); I
  treat that as a deployment boundary for the authority traits. If deployment
  intentionally permits downstream crates to implement those traits, the
  runtime, live-wallet, and PostgreSQL admission findings below reopen.
- The database SQLx/offline, SQL inventory, and model tasks were not
  independently reproduced in this review. The implementation owner reports
  the final `postgres-sqlx-offline-check`, `postgres-sql-inventory-check`, and
  `nix run .#model-check` tasks pass; the reported model hash begins `d8ede667`.
- The review treats provider-issued mutation attestations as durable,
  target-bound replay evidence, as required by the current design contract. If
  the deployment instead trusts a synchronous provider gate and does not
  require cryptographic rereading of retained attestations, that relaxed
  contract must be made explicit and tested.

## Frozen candidate and scope

- Candidate: `4fc1baea0719ae6b88837b3a0c063895a70dffcd`
- TT2 implementation baseline: `07b9d7311daae32230d7a487aa82e07f0d27ff2b`
- Reviewed cutover commits include authority sealing (`cc49802ec`), retained
  provider mutation evidence (`dd0522646`), physical-target fixation
  (`c851a2dab`), proof-bearing wallet checkpoint support (`812cf5f23`), and
  idempotent PostgreSQL retry reconciliation (`4fc1baea0`).
- Worktree was clean before this evidence file was added.
- Scope: canonical append and fold authority, Runtime assembly, replay/export
  provenance, EVM wallet authority, PostgreSQL target/checkpoint authority,
  idempotent retry recovery, SQL inventory, and generated recoverability and
  schema-depth budgets.

The candidate adds workspace-private marker traits to the Runtime history,
physical-binding, wallet, checkpoint, and deployment-credential seams. It now
retains provider-issued activation and completion mutation attestations in
wallet closures, carries an exact physical target through portable exports,
and reconciles an idempotent PostgreSQL retry after another writer has already
advanced the external checkpoint. Those changes close the previously
unrestricted authority bridges and the missing physical-target field, but they
do not yet provide independent provider-proof verification on every wallet
reload or seal the offline replay trust hooks.

## Verification evidence

Focused commands were run in the default Nix development shell against the
exact candidate. Nix automatic GC thresholds were set to zero for these
invocations only, avoiding an invalid-store-path race; no project behavior was
changed.

| Check | Result |
| --- | --- |
| `git diff --check 07b9d731^..4fc1baea` | pass |
| `nix develop -c cargo fmt --all -- --check` | pass |
| `nix develop -c cargo check -p mfm-authority-seal -p mfm-values -p mfm-canonical -p mfm-certify -p mfm-runtime -p mfm-store -p mfm-replay -p mfm-evm -p mfm-evm-live -p mfm-storage-postgres -p mfm-storage-evm-postgres -p mfm-integration-tests` | pass (warnings only) |
| scoped `nix develop -c cargo clippy ... --all-targets --no-deps -- -D warnings` | pass for authority-seal, values, canonical, certify, runtime, store, replay, evm-live, and storage-postgres |
| `nix develop -c cargo test -p mfm-store --test api-surface -- --nocapture` | pass (1/1 trybuild harness) |
| `nix develop -c cargo test -p mfm-evm --lib -- --nocapture` | pass (47/47) |
| `nix develop -c cargo test -p mfm-evm-live --lib -- --nocapture` | pass (35/35) |
| `nix develop -c cargo test -p mfm-replay --lib -- --nocapture` | pass (1/1) |
| `nix run .#run -- --task recoverability-postgres-v1` | pass (managed PostgreSQL, 1/1; 38.70s) |
| `nix run .#run -- --task evm-postgres-submission-qualification` | pass (managed PostgreSQL, release EVM restart path, 1/1; 1344.07s) |
| `nix run .#run -- --task wallet-nonce-postgres-storage-qualification` | pass (managed PostgreSQL, 1/1; 199.59s) |

The scoped `-D warnings` Clippy lane intentionally excludes `mfm-evm`, which
still reports the existing `large_enum_variant` lint at
`crates/domains/evm/src/submission.rs:795`, and
`mfm-storage-evm-postgres`, whose all-target build emits existing dead-code
warnings for provider/registry maintenance helpers. Cargo check remains green
with those warnings. The managed qualifications exercise the production parity
paths for the excluded storage crate.

## Authority bridges closed by the workspace-private marker

`RuntimeHistoryPort`, `PublicPhysicalBindingVerifier`, `WalletNonceAuthority`,
`ExternalCheckpointAuthority`, `DeploymentCredentialSink`, and
`DeploymentCredentialBroker` now inherit marker traits from the unpublished
`mfm-authority-seal` package. The Runtime assembly cutover also requires its
workspace-only assembly marker. The API-surface trybuild test confirms that an
ordinary fake implementation cannot satisfy these public traits, while the
workspace-owned store and PostgreSQL/EVM adapters opt in explicitly.

Accordingly, the previous Runtime authority bridge, injectable live EVM wallet
authority, and caller-supplied PostgreSQL target/checkpoint admission are
closed under the stated workspace-private deployment assumption. The
PostgreSQL admission now moves private credentials through a one-shot
`PostgresTargetAdmission`, and checkpoint transactions validate the exact
target tuple before commit.

Relevant surfaces: `crates/kernel/authority-seal`,
`crates/kernel/runtime/src/history/port.rs`,
`crates/kernel/store/src/structured/assembly.rs`,
`crates/live/evm/src/structured_wallet.rs`,
`crates/storages/postgres/src/checkpoint.rs`, and
`crates/storages/postgres/src/session.rs`.

## High findings

### Candidate reload does not independently verify retained provider activation proof

Provider mutation attestations are now channel-verified and retained in
`ActiveWalletCandidate`, and the provider-side checkpoint prefix includes the
candidate JSON. That is a material improvement over the prior discard path.

The PostgreSQL reload path is still weaker than the contract requires:
`load_candidates` and `load_candidate_by_key` check semantic keys, ordinals,
activation evidence references, and progression, but do not call
`validate_active_wallet_candidate_prefix` and do not even check the retained
provider-attestation syntax. No reload path verifies the retained provider
signature against the target-bound activation mutation preimage, provider key,
or current provider incarnation. A persisted candidate with a substituted or
copied provider string can therefore pass these reads until a later provider
checkpoint or mutation catches it.

Action: make candidate reload perform the complete prefix validation and
independently verify the retained provider proof against the exact target-bound
mutation preimage (or replace the string with a typed, verifiable proof).

Surfaces: `crates/domains/evm/src/wallet_authority.rs` and
`crates/storages/evm-postgres/src/authority.rs`.

### Completion reload checks provider-proof self-consistency, not provider authenticity

`CompletedWalletNonce` and its private recovery closure now retain the provider
completion attestation. `validate()` checks that the attestation is bounded,
non-empty, and equal in the outer and inner closure; it also validates the
candidate prefix. `retained_completion_is_valid` invokes that self-check during
normal reload.

However, no reload path verifies the retained completion attestation against a
provider signature, exact completion mutation preimage, target identity, or
provider incarnation. Rewriting both the outer field and the private closure
field with another syntactically valid value can satisfy the self-consistency
check without proving that the provider issued it for this completion. This
violates the design requirement that reload reject absent, substituted, or
inconsistent provider evidence.

Action: retain a typed provider proof with its operation/preimage binding and
verify it during `load_completion`/`retained_completion_is_valid`; a format and
outer/inner equality check is insufficient.

Surfaces: `crates/domains/evm/src/wallet_authority.rs` and
`crates/storages/evm-postgres/src/authority.rs`.

### Portable replay trust hooks remain caller-implementable

Portable exports now carry `PhysicalTargetIdentity` (target key, database OID,
fence generation, release epoch, and current incarnation), export readers reject
identities without a physical target, and source-prefix validation requires an
exact target match. This closes the previous omission of physical target
fixation.

Strict offline provenance is nevertheless still forgeable through the public
`RetainedPhysicalReleaseTrust` and `StoreCheckpointTrust` traits:
`ReplayTrustSnapshot::with_authorized_closure` accepts caller implementations
of both, and `ExportRunEvidence::with_encoder_view` remains a public callback
view. `PortableFixation` itself has public serializable fields. An external
consumer can therefore construct/deserialize a forged fixation or supply trust
hooks that return `true`, bypassing deployment-issued release/checkpoint
authority. The physical-target field alone does not seal that trust boundary.

Action: make replay trust material deployment-issued and non-implementable by
ordinary downstream crates (or explicitly document and enforce a trusted
integrator boundary), and restrict construction/inspection of sealed export
evidence accordingly.

Surfaces: `crates/kernel/replay/src/portable.rs` and
`crates/kernel/store/src/structured/purpose.rs`.

## PostgreSQL retry reconciliation reviewed

The `4fc1baea0` path was inspected together with the full managed PostgreSQL
recoverability qualification. `indexed_run_checkpoint_digest` binds the
external run checkpoint to the indexed database head in one target-validated
transaction. `checkpoint_mutations_for_batch` recognizes an external head that
already equals the indexed current run/fact head and emits no rewind mutation;
the contention classifier reconstructs the exact existing batch and
re-acknowledges its checkpoint mutations before returning `ExistingSame`.
The qualification passed, and the release EVM restart qualification also passed
with no duplicate broadcast. No concrete retry regression was observed.

## Closed or passing areas observed

- The workspace-private authority markers prevent ordinary downstream
  implementations of the Runtime, live-wallet, and PostgreSQL authority seams;
  focused API-surface tests pass.
- Provider attestations are now verified at mutation time and retained in both
  activation and completion closures; the remaining issue is independent
  reload-time authenticity verification, not mutation-time channel binding.
- Exact physical target identity is carried through structured store identity,
  export evidence, portable fixation, and source-prefix matching.
- Managed PostgreSQL recoverability, wallet storage, and release EVM
  submission/restart qualifications pass on this exact candidate.
- EVM domain, live-EVM, replay, formatting, and scoped compile/test checks pass;
  the Clippy exclusions and existing warnings are recorded above rather than
  hidden.
- The owner reports the final SQLx/offline, SQL inventory, and model checks
  pass; those remain owner-supplied evidence rather than independently
  reproduced results here.

## Verdict

**FAIL.** The exact candidate passes the focused and managed qualification
lanes, and the Runtime/live-wallet/PostgreSQL authority markers close the prior
public bridges under the stated deployment assumption. Strict provenance still
fails because wallet reload does not independently authenticate retained
provider proofs and portable replay accepts public caller-controlled trust
hooks. PASS requires those residual proof/replay boundaries to be sealed and
covered by adversarial regression tests.
