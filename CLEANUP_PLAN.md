     MFM Pre-Redesign Cleanup Plan

     Context

     The MFM project has accumulated known limitations in both
     mfm_machine (state machine framework) and mfm_core (keystore +
     config). Before proceeding with a larger redesign, we need to:

     1. Remove the legacy ChaCha20-Poly1305 encryption module
     (superseded by the keystore's AES-256-GCM + Argon2id)
     2. Fix all 10 known limitations in mfm_machine (section 3.8 of
     CURRENT_STATE.md)
     3. Fix keystore limitations #3, #4, #5, #8 (section 4.5 — items
     #1, #2, #6, #7 are by design)
     4. Fix config limitations #1-#3, #5 (section 4.5 — #4 is deferred
     to redesign)

     Exclusions by design (will NOT be changed):
     - Keystore single-file storage (#1)
     - Keystore whole-file rewrite on mutation (#2)
     - Keystore verify_file_integrity re-reads file (#6)
     - Keystore save_to_disk serializes twice (#7)
     - Config limited type variants (#4) — deferred to redesign

     ---
     Phase 0: Remove Legacy Encryption

     Risk: Minimal | Effort: Small

     Only remove encryption.rs and clean up its references. Keep
     wallet.rs, Method enum, and Config methods.

     Step 0.1: Delete encryption module

     - DELETE mfm_core/src/config/authentication/encryption.rs

     Step 0.2: Update authentication/mod.rs

     - File: mfm_core/src/config/authentication/mod.rs
     - Remove pub mod encryption; line

     Step 0.3: Update wallet.rs

     - File: mfm_core/src/config/authentication/wallet.rs
     - Remove use
     crate::config::authentication::encryption::Encryption;
     - Simplify read_private_key(): remove the decrypt branch entirely
     - Keep only the not_encrypted path (read plaintext from file)
     - Since there's no more encryption support for wallet files, the
     method should return the file contents directly (or error if
     not_encrypted is false/missing)

     Step 0.4: Remove unused dependencies

     - File: mfm_core/Cargo.toml
     - Remove pbkdf2 = "0.12" (only used by encryption.rs via
     ring::pbkdf2)
     - Keep ring (still used by keystore HMAC at
     mfm_core/src/keystore/mod.rs:593,606)
     - Check if base64 is still used elsewhere in mfm_core after
     removing encryption.rs (keystore uses it — keep)

     Verification

     cargo build -p mfm_core
     cargo test -p mfm_core

     ---
     Phase 1: mfm_machine Bug Fixes (Non-Breaking)

     Risk: Low | Effort: Small-Medium

     Localized fixes that don't change public trait signatures yet.

     Step 1.1: Fix L3 — Tracker stores Arc clones, not snapshots

     - File: mfm_machine/src/state_machine/tracker.rs
     - In HashMapTracker::track() (line 103-106): call
     context.snapshot().unwrap_or(context.clone()) instead of
     context.clone() to store a true deep copy
     - In TrackerHistory::push() (line 44): same — snapshot before
     storing
     - recover() (line 109-111) returns .cloned() which is now an Arc
     clone of the snapshot — correct behavior
     - Reuse existing SafeContext::snapshot() at safe_context.rs:72-83

     Step 1.2: Fix L6 — Error handler checks ALL dependencies

     - File: mfm_machine/src/state_machine/scheduler.rs
     - In DefaultErrorHandler::handle_error() (lines 128-149):
       - Replace depends_on.first().unwrap() with iteration over ALL
     dependency tags
       - For each tag, collect matching indexes from
     tracker.search_by_tag()
       - Apply DependencyStrategy (fixes L5 simultaneously)

     Step 1.3: Fix L5 — Implement DependencyStrategy usage

     - File: mfm_machine/src/state_machine/scheduler.rs (same method as
      L6)
     - After collecting all dependency indexes, apply strategy:
       - Latest → highest state_index
       - Earliest → lowest state_index
       - LatestSuccessful → same as Latest for now (tracker doesn't
     store success/failure per execution yet; add TODO for future
     enhancement)

     Step 1.4: Fix L7 — Replace println! with log

     - File: mfm_machine/src/state_machine/mod.rs line 280
     - Replace println!("Skipping state...") with log::debug!("Skipping
      state...")
     - File: mfm_machine/Cargo.toml — add log = "0.4" dependency

     Step 1.5: Fix L4 — Bounded context history

     - File: mfm_machine/src/state/context.rs
     - Change Local.history: Vec<HashMap<String, Value>> to
     VecDeque<HashMap<String, Value>>
     - Add #[serde(skip)] max_history: usize field with const
     DEFAULT_MAX_HISTORY: usize = 100
     - In write() (line 66-78): after pushing to history, trim with
     while new_history.len() > self.max_history {
     new_history.pop_front(); }
     - Add Local::with_max_history(max: usize) constructor
     - Update tests to verify bounded behavior

     Verification

     cargo build -p mfm_machine
     cargo test -p mfm_machine

     ---
     Phase 2: Build-Time Validation (L9)

     Risk: Low-Medium | Effort: Medium

     Step 2.1: Validate in StateMachineBuilder::build()

     - File: mfm_machine/src/state_machine/mod.rs
     - Change build() (line 67) return type: StateMachine →
     Result<StateMachine, StateMachineError>
     - Add validation checks:
       a. No duplicate labels across states
       b. All depends_on tags are satisfied by at least one state's
     tags()
       c. No circular dependencies (topological sort)
       d. Non-empty states list
     - Add error variants to StateMachineError:
       - DuplicateLabel(Label)
       - DanglingDependency { state_label: Label, missing_tag: Tag }
       - CircularDependency(Vec<Label>)
     - Update StateMachine::new() (line 124) to call build() internally
     - Update all test code using build() to handle Result

     Step 2.2: Enhance derive macro field validation

     - File: mfm_machine_derive/src/lib.rs
     - Before generating impl, validate struct has named fields: label,
      tags, depends_on, depends_on_strategy
     - Use syn to inspect fields, emit clear compile error if missing
     - Consider updating syn from 1.0 to 2.0 in
     mfm_machine_derive/Cargo.toml

     Verification

     cargo build -p mfm_machine_derive
     cargo build -p mfm_machine
     cargo test -p mfm_machine

     ---
     Phase 3: Tag/Label Owned Strings (L8)

     Risk: Medium-High | Effort: Medium | Touches many files

     Step 3.1: Change core types

     - File: mfm_machine/src/state/mod.rs
     - Tag(pub &'static str) → Tag(pub String) — remove Copy from
     derives
     - Label(pub &'static str) → Label(pub String) — remove Copy from
     derives
     - ensure_nonempty_ascii_lowercase_underscore: change param from
     &'static str to &str, return Result<String, Error>
     - Tag::new(s: &str) and Label::new(s: &str) — no longer require
     'static
     - Tag::new_const() → rename to Tag::new_unchecked(s: &str) — skips
      validation, allocates String
     - as_str() returns &str instead of &'static str
     - From<Label> for String — change to value.0 (already String)

     Step 3.2: Update standard_tags

     - File: mfm_machine/src/state/mod.rs
     - standard_tags constants can't be const with String. Use
     std::sync::LazyLock (stable since Rust 1.80):
     pub static CONFIG: LazyLock<Tag> = LazyLock::new(||
     Tag("config".to_string()));
     - Or simpler: convert them to plain functions: pub fn config() ->
     Tag { Tag::new_unchecked("config") }
     - Update all(), kind_tags(), behavior_tags() accordingly

     Step 3.3: Update derive macro

     - File: mfm_machine_derive/src/lib.rs
     - Generated fn label(&self) -> Label needs .clone() since Label is
      no longer Copy
     - fn depends_on_strategy(&self) -> DependencyStrategy —
     DependencyStrategy still derives Copy, no change needed

     Step 3.4: Update all consumers

     - Files (all need Tag/Label usage updates):
       - mfm_machine/src/state_machine/mod.rs (tests)
       - mfm_machine/src/state_machine/scheduler.rs (tests)
       - mfm_machine/src/state_machine/tracker.rs (tests)
       - mfm_machine/tests/default_impls.rs
       - mfm_machine/tests/public_api_test.rs
       - mfm_machine/tests/retry_workflow_state_machine.rs
       - mfm_machine/tests/n_states_with_n_ctxs.rs

     Verification

     cargo build -p mfm_machine_derive
     cargo build -p mfm_machine
     cargo test -p mfm_machine
     cargo build -p mfm_core  # re-exports SafeContext

     ---
     Phase 4: Async Handlers + Parallel Execution (L1 + L2)

     Risk: High | Effort: Large | Touches every file in mfm_machine

     Step 4.1: Make StateHandler async (L1)

     - File: mfm_machine/src/state/mod.rs
     - Add async-trait dependency to mfm_machine/Cargo.toml
     - Add tokio dependency to mfm_machine/Cargo.toml
     - Change StateHandler trait:
     #[async_trait::async_trait]
     pub trait StateHandler: StateMetadata + Send + Sync {
         async fn handler(&self, context: SafeContext) -> StateResult;
     }
     - Update impl StateHandler for Box<dyn StateHandler> (line
     327-330) with #[async_trait]
     - Note: using async-trait crate because async fn in dyn Trait
     requires it on stable Rust

     Step 4.2: Make execution engine async

     - File: mfm_machine/src/state_machine/mod.rs
     - execute(), execute_from(), execute_with_filter() → pub async fn
     - execute_rec() → async fn with Box::pin for recursion:
     fn execute_rec<'a>(&'a mut self, ...) -> Pin<Box<dyn Future<Output
      = Result<...>> + 'a>> {
         Box::pin(async move { ... })
     }
     - transition() stays sync (no handler calls)
     - state.handler(context.clone()) →
     state.handler(context.clone()).await

     Step 4.3: Add parallel state execution support (L2)

     - File: mfm_machine/src/state_machine/scheduler.rs
     - Extend Scheduler trait with optional method:
     fn next_states(...) -> Result<Vec<usize>, SchedulerError> {
         // Default: return vec![self.next_state(...)?]
     }
     - Add ParallelScheduler implementation that can return multiple
     independent states
     - File: mfm_machine/src/state_machine/mod.rs
     - Add parallel execution path in execute_rec: when scheduler
     returns multiple states, spawn them with tokio::join! or
     futures::join_all
     - States running in parallel share the SafeContext (already
     Arc<RwLock>) — concurrent reads are fine, concurrent writes are
     serialized by RwLock

     Step 4.4: Update ALL test files

     - All #[test] → #[tokio::test]
     - All test functions → async fn
     - All state.handler(ctx) calls → state.handler(ctx).await
     - Files:
       - mfm_machine/src/state_machine/mod.rs (inline tests)
       - mfm_machine/src/state/mod.rs (inline tests — no handler calls,
      may not need async)
       - mfm_machine/tests/default_impls.rs
       - mfm_machine/tests/public_api_test.rs
       - mfm_machine/tests/retry_workflow_state_machine.rs
       - mfm_machine/tests/n_states_with_n_ctxs.rs
       - mfm_machine/src/state_machine/scheduler.rs (inline tests)

     Verification

     cargo build -p mfm_machine
     cargo test -p mfm_machine
     cargo build -p mfm_core
     cargo test -p mfm_core
     cargo build -p mfm_cli
     cargo test -p mfm_cli

     ---
     Phase 5: Ergonomics Macro (L10)

     Risk: Medium | Effort: Large

     Step 5.1: Create attribute macro

     - File: mfm_machine_derive/src/lib.rs
     - Add new proc-macro: #[state_handler(...)] attribute macro
     - Usage:
     #[state_handler(
         label = "setup_state",
         tags = ["setup", "config"],
         depends_on = ["input"],
         strategy = Latest
     )]
     struct Setup;

     #[async_trait]
     impl StateHandler for Setup {
         async fn handler(&self, context: SafeContext) -> StateResult {
      ... }
     }
     - The macro generates:
       - The 4 metadata fields on the struct
       - StateMetadata impl (label, tags, depends_on,
     depends_on_strategy)
       - new() constructor
       - Default impl

     Step 5.2: Keep backward compatibility

     - Keep the existing #[derive(StateMetadataReqs)] working for
     manual struct definitions
     - The new attribute macro is an ergonomic alternative, not a
     replacement

     Step 5.3: Migrate test states

     - Update test states in mfm_machine/tests/ to use the new macro
     where it simplifies code
     - Keep at least one test using the old derive approach to verify
     it still works

     Verification

     cargo build -p mfm_machine_derive
     cargo build -p mfm_machine
     cargo test -p mfm_machine

     ---
     Phase 6: Config Model Improvements (C1-C3, C5)

     Risk: Low | Effort: Medium

     Step 6.1: Fix C2 — Addresses as proper types

     - File: mfm_core/src/config/mod.rs — WalletConfig.address: String
     → Address (from alloy_primitives, already in workspace deps with
     serde feature)
     - File: mfm_core/src/config/token.rs — TokenNetwork.address:
     String → Address
     - File: mfm_core/src/config/dexes.rs — router_address,
     factory_address, settlement_contract: Option<String> →
     Option<Address>
     - Add alloy-primitives = { workspace = true } to
     mfm_core/Cargo.toml (if not already there — check it's imported,
     not just declared)

     Step 6.2: Fix C3 — Financial values as Strings with parse methods

     - File: mfm_core/src/config/network.rs
       - min_balance_coin: f64 → min_balance_coin: String
       - Add impl Network { pub fn min_balance_wei(&self, decimals: u8)
      -> Result<U256> { ... } }
     - File: mfm_core/src/config/token.rs
       - slippage: f64 → slippage: String
       - Add impl TokenNetwork { pub fn slippage_bps(&self) ->
     Result<u32> { ... } } (parse string to basis points)

     Step 6.3: Fix C1 — Cross-reference validation

     - File: mfm_core/src/config/mod.rs
     - Add Config::validate(&self) -> Result<(), Vec<String>>:
       a. Each TokenNetwork.network_id exists in Config.networks
       b. Each Dex.network_id exists in Config.networks
       c. DexConfig.provider exists in Config.dexes
       d. All Address fields are valid (enforced by type after C2)
     - Call from Config::load() after deserialization

     Step 6.4: Fix C5 — Clean up dead code annotations

     - Review #[allow(dead_code)] in:
       - mfm_core/src/config/network.rs
       - mfm_core/src/config/token.rs
       - mfm_core/src/config/dexes.rs
     - For pub methods on pub types in a library crate, these
     annotations are unnecessary — the methods ARE the public API.
     Remove annotations.

     Verification

     cargo build -p mfm_core
     cargo test -p mfm_core

     ---
     Phase 7: Keystore Improvements (K3, K4, K5, K8)

     Risk: Medium | Effort: Medium

     Step 7.1: Fix K4 — Mnemonic passphrase support

     - File: mfm_core/src/keystore/mod.rs
     - Add passphrase: Option<String> to KeyType::Mnemonic (line
     131-134) with #[serde(default)] for backward compat
     - Change import_mnemonic() signature (line 377): add passphrase:
     Option<&str> parameter
     - Update to_seed("") → to_seed(passphrase.unwrap_or("")) at lines
     392 and 470
     - Update KeyType::Mnemonic construction (line 419) to include
     passphrase
     - Update get_private_key() (line 462) to extract and use stored
     passphrase
     - File: mfm_core/src/keystore/tests.rs — update all
     import_mnemonic calls with new None arg
     - File: mfm_cli/src/cli/keystore/import.rs — add optional
     --passphrase CLI arg

     Step 7.2: Fix K3 — Add key/mnemonic export

     - File: mfm_core/src/keystore/mod.rs
     - Add pub fn export_private_key(&mut self, id: Uuid) ->
     Result<Zeroizing<String>, KeystoreError>
       - Decrypt entry, hex-encode raw private key bytes (for
     PrivateKey entries)
       - For Mnemonic entries: decrypt mnemonic, derive key, hex-encode
       - Use existing decrypt_data() (private method around line 635)
     - Add pub fn export_mnemonic(&mut self, id: Uuid) ->
     Result<Zeroizing<String>, KeystoreError>
       - Only works for KeyType::Mnemonic entries
       - Returns InvalidInput error for KeyType::PrivateKey
     - Add tests for both export methods

     Step 7.3: Fix K5 — Password change / re-encryption

     - File: mfm_core/src/keystore/mod.rs
     - Add pub fn change_password(&mut self, old_password: &str,
     new_password: &str) -> Result<(), KeystoreError>
       - Verify old password (reuse existing derive_master_key +
     create_verification_hash + constant-time compare)
       - Decrypt all entries with old master key (reuse decrypt_data())
       - Generate new salt (32 bytes from OsRng)
       - Derive new master key (reuse derive_master_key())
       - Re-encrypt all entries with new master key and fresh nonces
     (reuse encrypt_data())
       - Update KDF params, verification hash, master key in memory
       - save_to_disk()
     - Add comprehensive tests: wrong old password, successful change,
     verify keys still accessible after change

     Step 7.4: Fix K8 — Audit log

     - File: mfm_core/src/keystore/mod.rs
     - Add types:
     pub enum AuditEvent { Unlock, Lock, ImportPrivateKey{id},
     ImportMnemonic{id}, GetPrivateKey{id}, ExportPrivateKey{id},
     ExportMnemonic{id}, DeleteKey{id}, ChangePassword }
     pub struct AuditLogEntry { timestamp: DateTime<Utc>, event:
     AuditEvent, success: bool }
     - Add audit_log: Vec<AuditLogEntry> to Keystore struct
     - Add #[serde(default)] audit_log: Vec<AuditLogEntry> to
     KeystoreFile struct (backward compat)
     - Add private fn log_audit(&mut self, event: AuditEvent, success:
     bool)
     - Insert audit calls at start/end of: unlock, lock,
     import_private_key, import_mnemonic, get_private_key, delete_key,
     change_password, export_*
     - Add pub fn audit_log(&self) -> &[AuditLogEntry] getter
     - Audit log is NOT encrypted (contains only UUIDs and timestamps,
     no secrets)
     - Add tests verifying audit entries are created for each operation

     Verification

     cargo build -p mfm_core
     cargo test -p mfm_core
     cargo build -p mfm_cli
     cargo test -p mfm_cli

     ---
     Execution Order Summary












     Phase: 0
     Scope: Remove legacy encryption
     Key Files: mfm_core/src/config/authentication/encryption.rs
     (delete),
       wallet.rs, mod.rs, Cargo.toml
     ────────────────────────────────────────
     Phase: 1
     Scope: Machine bug fixes (L3-L7)
     Key Files: tracker.rs, scheduler.rs, state_machine/mod.rs,
     context.rs
     ────────────────────────────────────────
     Phase: 2
     Scope: Build-time validation (L9)
     Key Files: state_machine/mod.rs, mfm_machine_derive/src/lib.rs
     ────────────────────────────────────────
     Phase: 3
     Scope: Tag/Label owned strings (L8)
     Key Files: state/mod.rs, mfm_machine_derive/src/lib.rs, all tests
     ────────────────────────────────────────
     Phase: 4
     Scope: Async + parallel (L1+L2)
     Key Files: state/mod.rs, state_machine/mod.rs, scheduler.rs,
     Cargo.toml,
       all tests
     ────────────────────────────────────────
     Phase: 5
     Scope: Ergonomics macro (L10)
     Key Files: mfm_machine_derive/src/lib.rs, tests
     ────────────────────────────────────────
     Phase: 6
     Scope: Config improvements (C1-C3, C5)
     Key Files: config/mod.rs, network.rs, token.rs, dexes.rs
     ────────────────────────────────────────
     Phase: 7
     Scope: Keystore improvements (K3-K5, K8)
     Key Files: keystore/mod.rs, keystore/tests.rs,
     cli/keystore/import.rs
     Dependencies:
     - Phase 0 is independent
     - Phase 1 has no dependencies
     - Phase 2 depends on Phase 1 (uses same files)
     - Phase 3 depends on Phase 2 (changes types used in validation)
     - Phase 4 depends on Phase 3 (async trait uses new Tag/Label
     types)
     - Phase 5 depends on Phase 4 (macro generates async handler stubs)
     - Phase 6 is independent (can run after Phase 0)
     - Phase 7 is independent (can run after Phase 0)

     Phases 6 and 7 can be done in parallel with Phases 1-5 since they
     touch different crates.

     End-to-End Verification

     After all phases:
     cargo build --workspace
     cargo test --workspace
     cargo clippy --workspace

