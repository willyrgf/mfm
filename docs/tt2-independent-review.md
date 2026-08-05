# TT2 independent review evidence

Status: **FAIL**

This commit records an independent review of the frozen implementation candidate.
It is evidence-only and does not change implementation behavior.

## Material uncertainties

- This review applies a strict external/adversarial-consumer threat model. Several
  public authority traits and constructors may be acceptable if deployment
  integrators are explicitly trusted, but the candidate does not state that
  relaxed boundary. Under the strict contract, those surfaces remain findings;
  the scope must be documented or the bridges sealed before PASS.
- The database-backed SQLx/offline, SQL inventory, and model tasks were not
  independently reproduced in this review. The implementation owner reports the
  final `postgres-sqlx-offline-check`, `postgres-sql-inventory-check`, and
  `nix run .#model-check` tasks pass. The reported model hash begins `d8ede667`.
- The provider-attestation findings assume provider-issued evidence must remain
  retained and target-bound in the durable domain closure. If deployment instead
  treats the provider as a trusted synchronous gate whose attestation need not be
  replayable, that contract must be made explicit.

## Frozen candidate and scope

- Candidate: `d9e53a5e96749f667656a94af5c2bcec2aea1ddb`
- TT2 implementation baseline: `07b9d7311daae32230d7a487aa82e07f0d27ff2b`
- Worktree was clean before this evidence file was added.
- Scope: canonical append and fold authority, Runtime assembly, replay/export
  provenance, EVM wallet authority, PostgreSQL target/checkpoint authority, SQL
  inventory, and generated recoverability and schema-depth budgets.

This rebuild folds PostgreSQL checkpoint/reload/fact-scan changes into the frozen
implementation: test-support checkpoint state has a sidecar persistence path,
concurrent read fixations share one barrier, configuration reload rechecks the
reconstructed canonical head against the external fixation, envelope reload uses
explicit field closure, and parity fact-scan fixtures run complete folds on worker
stacks. These changes do not seal the public production authority traits recorded
below.

## Verification evidence

Focused commands were run in the default Nix development shell against the exact
candidate. Nix's automatic GC thresholds were set to zero for these invocations
only, avoiding an infrastructure invalid-store-path race; no project behavior was
changed.

| Check | Result |
| --- | --- |
| `git diff --check 07b9d731^..d9e53a5e` | pass |
| `nix develop -c cargo fmt --all -- --check` | pass |
| `nix develop -c cargo check -p mfm-values -p mfm-canonical -p mfm-certify -p mfm-runtime -p mfm-store -p mfm-evm -p mfm-evm-live -p mfm-storage-evm-postgres -p mfm-storage-postgres` | pass (warnings only) |
| `nix develop -c cargo clippy -p mfm-values -p mfm-canonical -p mfm-certify -p mfm-runtime -p mfm-store -p mfm-evm-live -p mfm-storage-postgres --all-targets -- -D warnings` | pass |
| `nix develop -c cargo test -p mfm-values` | pass (10 descriptor, 4 retained-contract, 5 schema-validation, and 2 doc tests) |
| `nix develop -c cargo test -p mfm-store --lib -- --nocapture` | pass (27/27) |
| `nix develop -c cargo test -p mfm-certify --test structured-certification` | pass (38/38) |
| `nix develop -c cargo test -p mfm-certify --test structured-ui` | pass (13 trybuild cases plus harness) |
| `nix develop -c cargo test -p mfm-evm-live -p mfm-storage-postgres -p mfm-storage-evm-postgres --lib` | pass (35/35, 6/6, and 2/2; 1 schema test ignored as DB-gated) |
| `nix develop -c cargo test -p mfm-storage-postgres --features test-support --lib -- --nocapture` | pass (13/13, including checkpoint barrier/reload tests) |
| `nix develop -c cargo test -p mfm-storage-postgres --features parity-tests --test structured-history --no-run` | pass (parity integration target compiles) |
| `nix develop -c cargo test -p mfm-canonical -p mfm-facts -p mfm-replay -p mfm-runtime --lib -- --nocapture` | pass (2/2, 14/14, 1/1, and 0 tests) |
| `mfm-store` feature tree and authority-surface searches | pass for the recorded findings; `store-authority` still unifies `runtime-authority` |

The owner additionally reports successful final PostgreSQL SQLx/offline and SQL
inventory tasks and the model check; those remain owner-supplied evidence rather
than independently reproduced results here.

## Blocker: Runtime authority bridge remains public

`mfm-store` enables `store-authority`, exposing
`CertifiedAccessAuthorization::from_committed_successor` and
`QualifiedProgramRegistry::into_runtime_parts` from `mfm-certify`, together with
`RuntimeProcessRegistry::from_certified` and `Runtime::from_assembled` from
`mfm-runtime`. A consumer can obtain a real process registry and invoke public
certified process operations without being forced through the intended Runtime
history-head bracket. `RuntimeHistoryPort` and
`PublicPhysicalBindingVerifier` remain public caller-implementable traits, and
public `assemble_structured_runtime` accepts those trust objects directly. Under
the strict threat model this leaves a forgeable bridge around the claimed sole
store-owned authority.

Relevant surfaces: `crates/kernel/store/Cargo.toml`,
`crates/kernel/runtime/Cargo.toml`, `crates/kernel/certify/src/structured.rs`,
`crates/kernel/runtime/src/structured.rs`,
`crates/kernel/runtime/src/history/port.rs`, and
`crates/kernel/store/src/structured/assembly.rs`.

## High findings

### Candidate activation lacks retained provider-issued evidence

`CandidateActivationPermit` and the gated public
`derive_exact_candidate_activation_permit` are available to storage consumers
through `authority-integration`. The derivation is pure over caller-provided
reservation, prefix, ordinal, and observation values. PostgreSQL creates an
evidence reference from request/state/resource lineage and asks the provider to
attest the mutation, but `WriteTransactionLease::prepare_mutation` verifies and
discards that provider attestation. No retained provider-issued, target-bound
activation evidence is present in the candidate closure.

Surfaces: `crates/domains/evm/src/wallet_authority.rs`,
`crates/storages/evm-postgres/src/authority.rs`, and
`crates/storages/evm-postgres/src/provider.rs`.

### Completion closure has no retained terminal provider proof

`CompletedWalletNonce::with_recovery_closure` is public and checks internal
self-consistency, but PostgreSQL builds the closure from request and database
rows. Provider prepare/finish attestations are verified and discarded; no fresh
provider terminal reread or preimage evidence is retained in the completion
closure. A strict consumer cannot distinguish a DB-derived terminal result from a
provider-issued terminal observation.

Surfaces: `crates/domains/evm/src/wallet_authority.rs`,
`crates/storages/evm-postgres/src/authority.rs`, and
`crates/storages/evm-postgres/src/provider.rs`.

### Live EVM wallet authority is injectable through a public trait

`WalletNonceAuthority` is a public, freely implementable trait. Public
`EvmStructuredWalletBindings::new` accepts any `Arc<dyn WalletNonceAuthority>`
and only checks callback-reported activation/current-incarnation values and
release-history consistency. A fake implementation can supply those certificates
without a storage-issued opaque binding or private factory.

Surfaces: `crates/domains/evm/src/wallet_authority.rs` and
`crates/live/evm/src/structured_wallet.rs`.

### PostgreSQL target/checkpoint admission trusts caller authority

`ExternalCheckpointAuthority`, `DeploymentCredentialBroker`,
`DeploymentCredentialSink`, `PostgresTargetAdmission`, and `from_broker` are
public. The admission sink validates only non-empty credential strings and accepts
any caller-supplied checkpoint authority. The new test-support sidecar and
multi-reader barrier tests improve fixture durability and race coverage, but do
not make production admission opaque or deployment-issued at the type boundary.
A strict deployment boundary needs an opaque target admission and a
private/internal broker bridge.

Surfaces: `crates/storages/postgres/src/checkpoint.rs` and
`crates/storages/postgres/src/session.rs`.

### Portable replay fixation lacks exact physical target binding

`PortableFixation` records semantic/journal heads, store scope/epoch, and tenant
scope, but not an exact physical target, database identity, fence, release, or
current incarnation. `RetainedPhysicalReleaseTrust`, `StoreCheckpointTrust`, and
`ReplayTrustSnapshot::with_authorized_closure` are public caller-implementable
trust hooks. `ExportRunEvidence::with_encoder_view` exposes a public callback view
of encoder inputs. Strict replay provenance requires deployment-issued opaque trust
material or an explicitly documented trusted deployment boundary.

Surfaces: `crates/kernel/replay/src/portable.rs` and
`crates/kernel/store/src/structured/purpose.rs`.

## Closed or passing areas observed

- The prior valid-admission regression is fixed: the complete `mfm-store` library
  suite passes 27/27, including the formerly failing fold/admission tests.
- PostgreSQL checkpoint support now retains test-sidecar state, preserves multiple
  simultaneous read fixations, and rechecks reconstructed configuration heads;
  the test-support unit suite passes 13/13.
- PostgreSQL batch reload uses explicit envelope and record field closure, and the
  parity integration target compiles with the updated fact-scan worker-stack tests.
- Expected authorization matching recomputes record hashes and checks scope and
  lineage; substituted-ordinal certification coverage passes.
- Live EVM authorized-provider guards and exact transport inventories pass their
  focused tests.
- SQL catalog/inventory unit tests pass; the owner reports the final SQLx offline
  and inventory tasks pass.
- Generated recoverability limits and annex consistency are wired through the
  canonical crate; facts, canonical, replay, and certification boundary tests
  pass.
- Schema/value depth now reserves two schema-identity object levels from the
  canonical JSON budget (`MAX_SCHEMA_DEPTH = MAX_CANONICAL_JSON_DEPTH - 2`); the
  full value integration and schema-validation suites pass.

## Verdict

**FAIL.** The candidate's focused implementation tests pass, but strict external
authority and provenance boundaries remain publicly forgeable. PASS requires
sealing those bridges or explicitly adopting and documenting a trusted-integrator
threat model.
