# MFM — Current State Report

> Snapshot taken: 2026-02-07
> Branch: `redesigning` (post-cleanup; key commits: `302d204`, `6a29085`, `d76c7f8`)
> Version: 0.1.29 (workspace), mfm_machine at 0.1.0

---

## 1. Project Overview

MFM is a Rust workspace for on-chain operations. It provides a generic state machine
framework, a secure Ethereum keystore, blockchain/DeFi configuration models, and a CLI
that currently exposes keystore management.

**Status**: Experimental, MIT licensed, not production-ready.

---

## 2. Workspace Structure

```
mfm/
├── Cargo.toml          # workspace root, v0.1.29, resolver v2
├── mfm_machine/        # async recoverable state machine framework (lib, v0.1.0)
├── mfm_machine_derive/ # proc-macro for StateMetadata boilerplate (proc-macro lib, v0.1.0)
├── mfm_core/           # keystore + config models (lib, v0.1.29)
├── mfm_cli/            # command-line interface (bin, v0.1.29)
├── flake.nix           # nix build/dev environment
└── flake.lock
```

### Dependency graph

```
mfm_cli ──► mfm_core ──► mfm_machine ──► mfm_machine_derive
```

`mfm_cli` is a pure consumer. `mfm_core` re-exports `SafeContext` from `mfm_machine`.
The CLI does not use the state machine today — it only uses `mfm_core::keystore`.

### Workspace-level dependencies (declared, not all used yet)

| Category | Crates |
|----------|--------|
| Blockchain | `alloy-primitives`, `alloy-sol-types`, `alloy-json-abi`, `alloy-dyn-abi`, `alloy-provider`, `alloy-contract`, `alloy-consensus`, `alloy-network`, `alloy-rpc-client`, `alloy-transport-http`, `alloy-signer` |
| Crypto | `ring`, `k256`, `bip39`, `bip32`, `aes-gcm`, `argon2`, `tiny-keccak`, `zeroize`, `subtle` |
| Serialization | `serde`, `serde_json`, `serde_yaml` |
| Async | `tokio` (full), `async-trait`, `futures` |
| CLI | `clap` (derive + env) |
| Utilities | `anyhow`, `thiserror`, `url`, `hex`, `rand`, `uuid`, `chrono`, `base64` |

Only `alloy-primitives` is currently imported (used for `Address`/`U256` types in
`mfm_core`). The remaining `alloy-*` crates are declared at workspace level for future
on-chain work, but are not used yet.

---

## 3. `mfm_machine` — State Machine Framework

### 3.1 Purpose

A generic, tag-based, recoverable state machine for orchestrating multi-step workflows.
States declare metadata (label, tags, dependencies), a handler function receives a
shared context, and the machine runs them in sequence with automatic error recovery.

### 3.2 Module layout

```
mfm_machine/src/
├── lib.rs                      # re-exports, FIXME about ergonomics
├── state/
│   ├── mod.rs                  # Tag, Label, StateMetadata, StateHandler, StateError
│   ├── context.rs              # Context trait, Local impl, TypedContext
│   └── safe_context.rs         # SafeContext (Arc<RwLock<Box<dyn Context>>>)
└── state_machine/
    ├── mod.rs                  # StateMachine, StateMachineBuilder, execute logic
    ├── scheduler.rs            # Scheduler trait, DefaultScheduler, DependencyScheduler, ErrorHandler
    └── tracker.rs              # Tracker trait, HashMapTracker, Index, TrackerHistory
```

### 3.3 Core abstractions

#### Tag and Label

- Newtypes over `String`, validated as non-empty lowercase ASCII letters with underscores.
- `Tag::new(&str)` / `Label::new(&str)` validate at runtime.
- `Tag::new_unchecked(&str)` / `Label::new_unchecked(&str)` skip validation (still allocate).
- `as_str() -> &str` for access to the underlying string.
- Standard tags are provided as functions in `state::standard_tags`, split into:
  - **kind tags**: `config`, `fetch_data`, `compute`, `execute`, `report`,
    `report_operator`, `report_operation`
  - **behavior tags**: `apply_side_effect`, `impure`
- `Tag::is_kind_tag()`, `is_behavior_tag()`, `is_standard_tag()` for introspection.

#### StateMetadata (trait)

Every state declares:
- `label() -> Label` — unique identifier
- `tags() -> Vec<Tag>` — classification (can have multiple)
- `depends_on() -> Vec<Tag>` — which tags must have run before this state
- `depends_on_strategy() -> DependencyStrategy` — `Latest`, `Earliest`, or `LatestSuccessful`

Derived methods:
- `has_side_effects()` — checks for `APPLY_SIDE_EFFECT` tag
- `is_impure()` — checks for `IMPURE` tag
- `kind_tag()` — returns first kind tag found (assumes at most one)

#### StateHandler (trait: StateMetadata + Send + Sync)

```rust
#[async_trait::async_trait]
pub trait StateHandler: StateMetadata + Send + Sync {
    async fn handler(&self, context: SafeContext) -> StateResult;
}
// StateResult = Result<(), StateError>
```

This is the user-implemented logic. Receives a thread-safe context, reads/writes data,
and returns `Ok(())` or a categorized error. The engine awaits handlers; state
implementations should avoid blocking the async runtime.

#### StateError

Six error categories, each carrying `StateErrorRecoverability` (Recoverable/Unrecoverable)
and an `anyhow::Error`:

| Variant | Intended use |
|---------|-------------|
| `Unknown` | Catch-all |
| `ParsingInput` | Deserialization / input validation failures |
| `OnChainError` | Blockchain transaction / call failures |
| `OffChainError` | External service failures |
| `RpcConnection` | Network / RPC connectivity issues |
| `StorageAccess` | Context read/write failures |

Builder methods: `StateError::recoverable_on_chain(err)`,
`StateError::unrecoverable_parsing_input(err)`, etc.

#### DependencyStrategy (enum)

```rust
pub enum DependencyStrategy {
    Latest,
    Earliest,
    LatestSuccessful,
}
```

Used by `DefaultErrorHandler` to select a recovery point among dependency matches
(`Earliest` vs `Latest`). `LatestSuccessful` currently behaves like `Latest` (the
tracker does not yet record per-execution success/failure).

### 3.4 Context system

Three layers:

**Layer 1 — `Context` trait** (`context.rs`)

Abstract key-value store with immutable write semantics:
- `read(key) -> Value` / `write(key, value) -> Box<dyn Context>` (returns new context)
- `dump() -> Value` / `snapshot() -> Box<dyn Context>` / `history() -> Option<Vec<...>>`
- Default implementation: `Local` — `HashMap<String, Value>` with bounded `VecDeque` history
  (default max 100 snapshots; configurable via `Local::with_max_history`).

**Layer 2 — `TypedContext<T>` trait** (`context.rs`)

Blanket impl over any `Context`. Converts between Rust types and `serde_json::Value`:
- `read_typed::<T>(key) -> T`
- `write_typed::<T>(key, value) -> Box<dyn Context>`

**Layer 3 — `SafeContext`** (`safe_context.rs`)

Thread-safe wrapper: `Arc<RwLock<Box<dyn Context>>>`.

- `write_value(key, value)` acquires write lock, calls `Context::write()`, replaces
  inner box — mutates in-place from the caller's perspective.
- `Clone` clones the `Arc` — all clones share the same underlying data.
- `snapshot()` does a deep clone through the `Context::snapshot()` method.
- `read_typed::<T>()` / `write_typed::<T>()` — convenience wrappers combining lock
  acquisition and serde.
- `create_default_safe_context()` — factory for `SafeContext::new(Local::default())`.

### 3.5 State machine engine

#### StateMachineBuilder

```rust
StateMachineBuilder::new(states)
    .tracker(Box::new(custom_tracker))
    .scheduler(Box::new(custom_scheduler))
    .error_handler(Box::new(custom_handler))
    .max_recoveries(20)
    .build()?;  // -> Result<StateMachine, StateMachineError>
```

All components are optional — defaults to `HashMapTracker`, `DefaultScheduler`,
`DefaultErrorHandler`, and `max_recoveries = states.len() * 3 + 1`.

`build()` performs early validation of the state graph:
- non-empty state list
- no duplicate `Label`s
- all `depends_on` tags are provided by at least one state
- no circular dependencies (topological sort)

#### Execution model

`execute(context).await` runs an async loop:

1. `transition(ctx, index, last_result)`:
   - First call (no result): returns `([start_index], ctx)` passthrough
   - After `Ok(())`: calls `scheduler.next_states()` to get next indices (unless
     filter tags are set, in which case it uses `next_state()` to preserve skip
     semantics)
   - After recoverable `Err`: calls `error_handler.handle_error()`, looks up recovery
     point in tracker, returns `([recovery_index], recovery_context)`
   - After unrecoverable `Err`: returns `StateMachineError`
   - Checks max recoveries at each transition

2. `execute_rec(ctx, index, last_result)`:
   - Calls `transition()`
   - If filtering is enabled (or only a single next state is returned): checks filter tags,
     runs `state.handler(ctx).await`, tracks execution, continues
   - If multiple runnable next states are returned: runs them concurrently via
     `futures::future::join_all` (no `tokio::spawn`), tracks per-state snapshots, and
     propagates the first error (by index) into the next transition
   - If no more states (`NoNextState`): returns final context

Entry points:
- `execute(ctx).await` — from index 0
- `execute_from(ctx, start_index).await` — from arbitrary index
- `execute_with_filter(ctx, filter_tags).await` — skips states matching given tags

#### Scheduler trait

```rust
fn next_state(current_index, states, context) -> Result<usize, SchedulerError>
fn next_states(current_index, states, context) -> Result<Vec<usize>, SchedulerError>
fn set_filter_tags(tags) / fn get_filter_tags() -> Option<Vec<Tag>>
```

Three implementations:
- **DefaultScheduler** — `current_index + 1`, sequential.
- **DependencyScheduler** — searches for next state whose `depends_on` tags overlap with
  current state's tags. Falls back to sequential if no match.
- **ParallelScheduler** — can return a contiguous block of runnable states for concurrent
  execution. Assumes `states` are ordered to satisfy dependencies.

#### ErrorHandler trait

```rust
fn handle_error(error, current_index, current_state, states, tracker) -> Result<usize>
```

**DefaultErrorHandler**:
1. Checks `error.is_recoverable()` — rejects unrecoverable errors
2. Looks at `current_state.depends_on()` and collects tracker matches across **all**
   dependency tags
3. Applies `DependencyStrategy` to pick a recovery point (`Earliest`/`Latest`;
   `LatestSuccessful` currently behaves like `Latest`)
4. Returns the selected state's index as recovery point
5. Falls back to index 0 if no dependencies or no matches

#### Tracker trait

```rust
fn track(index: Index, context: SafeContext) -> Result<bool>
fn recover(index: Index) -> Option<SafeContext>
```

Plus `TrackerMetadata`:
- `indexes()` — all tracked indexes
- `search_by_tag(tag)` — find by tag
- `search_by_index(state_index)` — find by numeric index
- `history()` — full `TrackerHistory` (vec of `(step, Index, Value)` tuples)

**HashMapTracker**: stores `HashMap<Index, SafeContext>` + appends to `TrackerHistory`.
`Index` contains `state_index: usize`, `state_label: Label`, `state_tags: Vec<Tag>`.

### 3.6 Proc-macros (`mfm_machine_derive`)

Two macros are available:

1. `#[derive(StateMetadataReqs)]`
   - For states that define metadata fields manually.
   - Validates that the input is a struct with named fields:
     `label`, `tags`, `depends_on`, `depends_on_strategy` (clear compile-time errors).
   - Generates a `StateMetadata` impl that clones `Label`/`Tag` values (they are not `Copy`).

```rust
impl StateMetadata for MyState {
    fn label(&self) -> Label {
        self.label.clone()
    }
    fn tags(&self) -> Vec<Tag> {
        self.tags.clone()
    }
    fn depends_on(&self) -> Vec<Tag> {
        self.depends_on.clone()
    }
    fn depends_on_strategy(&self) -> DependencyStrategy {
        self.depends_on_strategy
    }
}
```

2. `#[mfm_machine_derive::state_handler(...)]` (attribute macro)
   - Ergonomic alternative for unit structs.
   - Adds the metadata fields, plus `new()` and a `Default` impl.
   - Does **not** implement `StateHandler`; consumers still implement `handler()` manually.

Note: the proc-macro expansions refer to `::mfm_machine::...`. The `mfm_machine` crate
includes `extern crate self as mfm_machine;` to make this work when macros are used
within the crate itself.

### 3.7 Test coverage

| Test file | What it covers |
|-----------|---------------|
| `tests/default_impls.rs` | 9 reusable test state implementations (some using `#[state_handler(...)]`): Setup, ComputePrice, Report, ConfigState, OnChainValuesState, ValidationState, NotificationState, AnalyticsState, FinalizeState |
| `tests/public_api_test.rs` | Verifies metadata accessors and async execution with a small workflow |
| `tests/retry_workflow_state_machine.rs` | Linear workflow, data propagation between states |
| `tests/n_states_with_n_ctxs.rs` | Linear transitions, complex workflow, parallel scheduler execution, dependency strategy ordering |
| Inline unit tests | Tag/Label validation, standard tag classification, bounded context history, SafeContext typed access, scheduler behavior, tracker track/search/recover |

### 3.8 Limitations (Post-Cleanup)

#### Prior limitations (L1-L10) — status

| # | Item | Status |
|---|------|--------|
| 1 | Handlers were synchronous | Fixed: `StateHandler::handler` is `async fn`. |
| 2 | No parallel execution | Fixed: scheduler can return multiple next states; engine can `join_all`. |
| 3 | Tracker stored shared `Arc` clones | Fixed: tracker stores deep `SafeContext` snapshots at track time. |
| 4 | Unbounded context history | Fixed: `Local` history is bounded (default 100; configurable). |
| 5 | `DependencyStrategy` unused | Mostly fixed: `Earliest`/`Latest` applied; `LatestSuccessful` is still TODO. |
| 6 | Error handler only checked first dependency | Fixed: checks all dependency tags. |
| 7 | Filter logic used `println!` | Fixed: uses `log::debug!`. |
| 8 | `&'static str` Tag/Label constraint | Fixed: `Tag`/`Label` own `String`. |
| 9 | No build-time validation | Fixed: builder validates empty/duplicate/dangling/circular dependencies. |
| 10 | Ergonomics poor | Improved: `#[state_handler(...)]` macro reduces boilerplate; broader ergonomics remain a redesign topic. |

#### Remaining limitations / notes

- `DependencyStrategy::LatestSuccessful` currently behaves like `Latest` (tracker does not record per-execution success/failure).
- `ParallelScheduler` assumes `states` are already in dependency order and returns a contiguous runnable block (it is not a general-purpose dynamic scheduler).

---

## 4. `mfm_core` — Core Library

### 4.1 Purpose

Provides the keystore for secure Ethereum key management, configuration models for
networks/tokens/DEXes, and auth helpers (including a plaintext wallet-file reader).

### 4.2 Module layout

```
mfm_core/src/
├── lib.rs                              # re-exports SafeContext from mfm_machine
├── config/
│   ├── mod.rs                          # Config, NetworkConfig, WalletConfig, DexConfig, SecureWallet
│   ├── network.rs                      # Network, Networks, Kind::Evm
│   ├── token.rs                        # Token, TokenNetwork, Tokens, Kind::Erc20
│   ├── dexes.rs                        # Dex, Dexes, Kind::{UniswapV2, CowSwap, UniswapV3}
│   └── authentication/
│       ├── mod.rs                      # Method::{Wallet, MetaMask}, Methods
│       ├── wallet.rs                   # Wallet (file-based key reader)
└── keystore/
    ├── mod.rs                          # Keystore, SecureKey, KeystoreConfig, KeyEntry, KeyInfo
    ├── error.rs                        # KeystoreError enum
    └── tests.rs                        # comprehensive unit tests
```

### 4.3 Config subsystem

#### Loading

`Config::load(path)` reads a YAML file into:

```rust
pub struct Config {
    pub networks: Networks,       // HashMap<String, Network> — all defined networks
    pub dexes: Dexes,             // HashMap<String, Dex> — all defined DEXes
    pub tokens: Tokens,           // HashMap<String, Token> — all defined tokens
    pub auth_methods: Methods,    // Vec<Method> — authentication methods
    pub network: NetworkConfig,   // active network selection
    pub wallet: WalletConfig,     // active wallet selection
    pub dex: DexConfig,           // active DEX selection
}
```

Pattern: define all options in top-level maps, select active one in flat fields.
`Config::load()` calls `Config::validate()` after deserialization and fails fast with a
multi-error message for invalid cross-references (e.g. `dex.provider` missing from
`dexes`, token networks referencing unknown `network_id`, DEX entries referencing unknown
`network_id`).

#### Network model

```rust
pub struct Network {
    pub name: String,
    pub kind: Kind,                     // Kind::Evm (only variant)
    pub symbol: String,                 // e.g. "ETH"
    pub decimals: Option<u8>,
    pub chain_id: u32,
    pub node_url_http: Option<String>,
    pub node_url_grpc: Option<String>,
    pub blockexplorer_url: Option<String>,
    pub min_balance_coin: String,       // decimal string (coin units)
    pub wrapped_token: Option<String>,  // e.g. WETH address
}
```

`Network::min_balance_wei(decimals)` parses `min_balance_coin` into base units (`U256`)
without floating point.

#### Token model

```rust
pub struct TokenNetwork {
    pub name: String,
    pub kind: Kind,             // Kind::Erc20 (only variant)
    pub network_id: String,     // references Networks key
    pub address: Address,       // contract address
    pub slippage: String,       // percent string (e.g. "0.50" or "0.50%")
    pub path_token: String,     // swap routing path
    pub decimals: Option<u8>,
}

pub struct Token {
    pub networks: TokenNetworks,  // HashMap<String, TokenNetwork>
}
```

`TokenNetwork::slippage_bps()` parses `slippage` into basis points (bps) without floating
point.

#### DEX model

```rust
pub enum Kind { UniswapV2, CowSwap, UniswapV3 }

pub struct Dex {
    pub name: String,
    pub kind: Kind,
    pub router_address: Option<Address>,
    pub factory_address: Option<Address>,
    pub network_id: String,
    pub settlement_contract: Option<Address>, // CowSwap
    pub api_url: Option<String>,              // CowSwap
}
```

#### Authentication

```rust
pub enum Method {
    Wallet(Wallet),   // file-based private key
    MetaMask,         // placeholder, not implemented
}
```

`Wallet` reads a **plaintext** private key file when `not_encrypted: true`. Encrypted
wallet files are no longer supported. `Config::load_wallet()` iterates auth methods and
falls back to `WalletConfig.private_key_path`.

#### Legacy encryption (removed)

The legacy ChaCha20-Poly1305 + PBKDF2 wallet encryption module was removed (CLEANUP_PLAN
Phase 0). Keystore encryption remains AES-256-GCM + Argon2id.

### 4.4 Keystore subsystem

The most mature component. Security-hardened Ethereum key storage.

#### Lifecycle

```
Keystore::new(path) / new_with_config(path, config)
  └─ if file exists → load_from_disk() (parse JSON, populate entries/params/MACs)
  └─ if not → empty state, no file created yet

keystore.unlock(password)
  └─ early_file_validation()
       └─ size: 100B .. 10MB
       └─ JSON parse check
       └─ version == 1
       └─ entry count <= 10,000
       └─ per-entry ciphertext <= 1MB
  └─ if file exists → unlock_existing(password)
       └─ Argon2id(password, stored_salt, stored_params) → master_key
       └─ HMAC-SHA256(master_key, "keystore_verification_v1") → compare with stored (constant-time)
       └─ verify_file_integrity(): re-serialize with placeholder MAC, HMAC, compare (constant-time)
       └─ store master_key in memory
  └─ if new → initialize_new(password)
       └─ random 32-byte salt
       └─ Argon2id → master_key
       └─ HMAC verification hash
       └─ save_to_disk() (with computed file MAC)
       └─ reload from disk to verify round-trip

keystore.lock()
  └─ master_key = None

keystore.import_private_key(alias, hex)
  └─ validate: strip "0x", 64 hex chars, non-zero, valid secp256k1
  └─ derive Ethereum address: Keccak-256(uncompressed_pubkey[1..]) → last 20 bytes
  └─ random UUID + random 12-byte nonce
  └─ AES-256-GCM encrypt(master_key, nonce, key_bytes, aad=UUID)
  └─ save_to_disk()
  └─ zeroize intermediates

keystore.import_mnemonic(alias, mnemonic, derivation_path, passphrase)
  └─ validate BIP39 mnemonic and BIP32 derivation path
  └─ seed = mnemonic.to_seed(passphrase.unwrap_or(""))
  └─ XPrv::derive_from_path(seed, path) → private key → Ethereum address
  └─ encrypt *mnemonic phrase* (not derived key)
  └─ save with KeyType::Mnemonic { derivation_path, passphrase }

keystore.get_private_key(uuid) → SecureKey
  └─ decrypt entry, reconstruct key (or re-derive from mnemonic + stored passphrase)
  └─ appends audit log entry + saves

keystore.list_keys() → Vec<KeyInfo>  (works when locked)

keystore.export_private_key(uuid) → Zeroizing<String>
  └─ decrypt entry (or derive from mnemonic), return 0x-prefixed hex
  └─ appends audit log entry + saves

keystore.export_mnemonic(uuid) → Zeroizing<String>
  └─ decrypt mnemonic (mnemonic keys only)
  └─ appends audit log entry + saves

keystore.change_password(old_password, new_password)
  └─ verify old password → decrypt all entries → new salt → derive new master key
  └─ re-encrypt all entries with fresh nonces → save_to_disk()

keystore.audit_log() → &[AuditLogEntry]
keystore.delete_key(uuid) → remove + save_to_disk()
```

#### Crypto stack

| Layer | Algorithm | Parameters |
|-------|-----------|-----------|
| KDF | Argon2id v0x13 | Production: 1GB memory, 8 iterations, parallelism 1 |
| Per-entry encryption | AES-256-GCM | Key: master_key (32B), Nonce: random 12B per entry |
| AAD | Entry UUID (16 bytes) | Binds ciphertext to entry, prevents swap attacks |
| Password verification | HMAC-SHA256 | `HMAC(master_key, "keystore_verification_v1")` |
| File integrity | HMAC-SHA256 | Over serialized file with placeholder MAC field |
| Comparison | `subtle::ConstantTimeEq` | Constant-time for both password and MAC checks |
| Key material | `zeroize::Zeroizing<[u8; 32]>` | Auto-zeroize on drop throughout |
| BIP support | BIP39 mnemonics + BIP32 derivation | `m/44'/60'/0'/0/0` standard Ethereum path |
| Signing | k256 secp256k1 ECDSA | Via `PrehashSigner` on `SigningKey` |
| Address derivation | Keccak-256 | On uncompressed public key (skip 0x04 prefix) |

#### SecureKey

```rust
pub struct SecureKey {
    key_bytes: Zeroizing<[u8; 32]>,
}
// impl ZeroizeOnDrop for SecureKey {}
```

Methods:
- `sign_hash(&[u8; 32]) -> k256::ecdsa::Signature` — creates ephemeral SigningKey, signs,
  relies on k256's ZeroizeOnDrop
- `ethereum_address() -> Address` — Keccak-256 on uncompressed public key
- `public_key() -> k256::PublicKey`

#### Disk format

JSON, human-readable:
```json
{
  "version": 1,
  "kdf_params": {
    "salt": [32 bytes],
    "memory_kb": 1048576,
    "iterations": 8,
    "parallelism": 1
  },
  "master_key_verification": [32 bytes],
  "audit_log": [{
    "timestamp": "ISO8601",
    "event": "unlock | lock | import_private_key | import_mnemonic | get_private_key | export_private_key | export_mnemonic | delete_key | change_password",
    "success": true
  }],
  "entries": [{
    "id": "uuid",
    "alias": "name",
    "address": "0x...",
    "key_type": "PrivateKey" | { "Mnemonic": { "derivation_path": "m/44'/60'/0'/0/0", "passphrase": "..." } },
    "encrypted_data": [bytes],
    "nonce": [12 bytes],
    "created_at": "ISO8601"
  }],
  "file_integrity_mac": [32 bytes]
}
```

#### Thread safety

Intentionally `!Send + !Sync`:
```rust
_not_thread_safe: *const ()  // raw pointer prevents Send + Sync auto-impl
```
Verified by compile-time assertion: `assert_not_impl_any!(Keystore: Send, Sync)`.

#### KDF configurations

| Profile | Memory | Iterations | Use case |
|---------|--------|-----------|----------|
| `production()` / `default()` | 1 GB | 8 | Real usage |
| `development()` | 8 MB | 2 | Dev testing (cfg(test) only) |
| `insecure_integration_test()` | 64 KB | 1 | CI/fast tests (public) |

#### Error types

```rust
pub enum KeystoreError {
    InvalidPassword,
    Locked,
    KeyNotFound(Uuid),
    InvalidPrivateKey,
    InvalidMnemonic(String),
    InvalidDerivationPath(String),
    CryptoError(String),
    FileError(String),
    SerializationError(String),
    InvalidInput(String),
}
```

Auto-conversions from: `std::io::Error`, `serde_json::Error`, `aes_gcm::Error`,
`argon2::Error`, `bip32::Error`, `bip39::Error`.

#### Test coverage

| Test | What it verifies |
|------|-----------------|
| `keystore_is_not_send_nor_sync` | Compile-time !Send + !Sync assertion |
| `test_new_keystore_creation` | Empty initial state, no file before unlock |
| `test_private_key_import_and_retrieval` | Import → get → sign → address → public key |
| `test_mnemonic_import_and_retrieval` | Import BIP39 → derive → sign → consistent address |
| `test_mnemonic_passphrase_support` | BIP39 passphrase support changes derived key |
| `test_concurrent_operations` | Multiple keys produce different signatures/addresses |
| `test_keystore_persistence` | Drop and reload keystore, key survives |
| `test_wrong_password` | InvalidPassword error on wrong password |
| `test_locked_operations` | Locked error on import/get/delete without unlock |
| `test_key_deletion` | Delete + verify gone + KeyNotFound on re-delete |
| `test_export_private_key_for_private_key_entries` | Export private key as 0x-hex |
| `test_export_mnemonic_and_derived_private_key` | Export mnemonic phrase and derived private key |
| `test_change_password_reencrypts_entries` | Password change re-encrypts all entries and persists |
| `test_audit_log_entries_created_for_operations` | Audit log records success/failure for operations |
| `test_keystore_new_variants` | Default vs development vs production configs |
| `test_unlock_edge_cases` | File creation, re-unlock, empty password |
| `test_lock_comprehensive` | Lock/unlock cycles, list_keys works when locked |
| `test_import_private_key_edge_cases` | Valid keys (with/without 0x, max curve), invalid keys |
| `test_import_mnemonic_edge_cases` | Valid paths, invalid paths, invalid mnemonics |
| `test_get_private_key_comprehensive` | Both key types, non-existent key, locked access |
| `test_list_keys_comprehensive` | Empty → add → list → metadata check → works when locked |
| `test_secure_key_methods` | Deterministic signatures, consistent address/pubkey |
| `test_error_conditions` | Systematic error type verification |
| `test_keystore_file_corruption_handling` | Corrupted JSON detection |
| `test_keystore_version_handling` | Unsupported version rejection |
| `test_file_integrity_protection` | Alias tampering detected by MAC |
| `test_file_integrity_protection_entry_swap` | Entry field tampering detected by MAC |
| `test_aad_prevents_entry_swapping` | Encrypted data swap detected by AAD mismatch |
| `test_early_file_validation_dos_protection` | Oversized, undersized, malformed JSON |

### 4.5 Known limitations

#### Keystore

| # | Limitation | Status / detail |
|---|-----------|-----------------|
| 1 | **Single-file storage** | By design (unchanged). |
| 2 | **Whole-file rewrite on mutation** | By design (unchanged). |
| 3 | **No export** | Fixed: `export_private_key()` and `export_mnemonic()`. |
| 4 | **Mnemonic passphrase hardcoded to empty** | Fixed: per-entry `passphrase: Option<String>`. |
| 5 | **No password change / re-encryption** | Fixed: `change_password(old, new)` re-encrypts all entries. |
| 6 | **`verify_file_integrity` re-reads file** | By design (unchanged). |
| 7 | **`save_to_disk` serializes twice** | By design (unchanged). |
| 8 | **No audit log** | Fixed: audit log persisted in the keystore file and covered by the file MAC. |

#### Config

| # | Limitation | Status / detail |
|---|-----------|-----------------|
| 1 | **No cross-reference validation** | Fixed: `Config::validate()` called from `Config::load()`. |
| 2 | **Addresses are plain Strings** | Fixed: address fields are `alloy_primitives::Address`. |
| 3 | **Floats for financial values** | Fixed: stored as strings with parse helpers (`min_balance_wei`, `slippage_bps`). |
| 4 | **Limited type variants** | Still a limitation (deferred to redesign). |
| 5 | **Several accessors are `#[allow(dead_code)]`** | Fixed: dead-code allowances removed. |

#### Legacy encryption

Removed in CLEANUP_PLAN Phase 0 (no longer applicable).

---

## 5. `mfm_cli` — Command-Line Interface

> Included briefly for completeness. The CLI is a consumer of `mfm_core::keystore`.

### 5.1 Commands

```
mfm [--output-format text|json] <command>

mfm keystore import  --import-type <privatekey|mnemonic> [--label] [--derivation-path] [--passphrase] [--keystore] [--stdin]
mfm keystore list    [--filter-label <regex>] [--sort-by <label|created|type>] [--show-addresses]
mfm keystore delete  [<uuid>] [--by-label] [--yes] [--keystore]
```

### 5.2 Key patterns

- **`CommandContext`** carries `OutputFormat` to all handlers.
- **`CommandResult<T>`** = `Result<CommandOutput<T>, CommandError>` — standardized responses.
- **`CommandError`** has typed codes: `KeyNotFound`, `InvalidUuid`, `MissingArgument`,
  `AmbiguousLabel`, `OperationCancelled`.
- **`KeystoreManager`** resolves path: `--keystore` flag > `MFM_KEYSTORE_PATH` env > `~/.mfm/keystore`.
- Password: `MFM_KEYSTORE_PASSWORD` env > interactive `rpassword` prompt.
- All handlers exit via `std::process::exit()` (`-> !`).
- JSON output uses `SuccessResponse<T>` / `ErrorResponse` wrapper structs.
- `tabled` for ASCII table formatting in text mode.

### 5.3 Environment variables

| Variable | Purpose |
|----------|---------|
| `MFM_OUTPUT_FORMAT` | Default output format (text/json) |
| `MFM_KEYSTORE_PATH` | Override keystore location |
| `MFM_KEYSTORE_PASSWORD` | Non-interactive password for scripting |
| `MFM_INTEGRATION_TEST` | Use fast insecure KDF config |

### 5.4 Test coverage

6 test files: CLI structure, e2e workflows, environment handling, JSON output format,
keystore integration, utilities.

---

## 6. What Exists vs. What Is Wired

| Component | Built | Tested | Used end-to-end |
|-----------|-------|--------|----------------|
| State machine framework | Yes | Yes | **No** — no concrete states exist outside tests |
| State machine derive macro | Yes | Yes (via test states) | **No** |
| Keystore (core) | Yes | Comprehensive | Yes — via CLI |
| Config loading (YAML) | Yes (with validation) | No dedicated tests | **No** — nothing calls `Config::load()` |
| Network/Token/Dex models | Yes | No | **No** — defined but unused |
| Auth methods (Wallet/MetaMask) | Partial | No | **No** — Wallet supports plaintext key files; MetaMask is a placeholder |
| CLI keystore commands | Yes | Yes | Yes |
| On-chain interaction | **No** | N/A | N/A |
| alloy-* blockchain crates | Declared | N/A | `alloy-primitives` imported by `mfm_core`; others not used yet |

---

## 7. Development History Summary

High-level phases:

**Phase 1 — Keystore security hardening** (bulk of commits):
Nonce-reuse prevention, zeroization, constant-time comparison, !Send+!Sync markers,
file integrity HMAC, AAD encryption, per-entry nonces, early file validation for DoS
protection, strict Argon2 output length, residual key material cleanup.

**Phase 2 — CLI standardization** (recent commits):
Structured command results, command context, standardized inputs/outputs for automation
(JSON output, env vars, stdin mode), documentation migration to READMEs.

**Phase 3 — Pre-redesign cleanup (CLEANUP_PLAN)** (2026-02-07):
Removed legacy wallet-file encryption; updated `mfm_machine` for async handlers, parallel
execution support, build-time validation, and improved proc-macro ergonomics; improved
config typing/validation; added keystore export, mnemonic passphrase support, password
change, and audit logging.

---

## 8. Architecture Diagram

```
                    ┌─────────────────────────────────────────┐
                    │               mfm_cli                    │
                    │  Commands: keystore {import,list,delete} │
                    │  Output: text / JSON                     │
                    │  Input: interactive / stdin / env vars   │
                    └────────────────┬────────────────────────┘
                                     │ uses
                    ┌────────────────▼────────────────────────┐
                    │              mfm_core                    │
                    │                                          │
                    │  ┌──────────────┐  ┌─────────────────┐  │
                    │  │   keystore   │  │     config       │  │
                    │  │              │  │                   │  │
                    │  │ Keystore     │  │ Config (YAML)    │  │
                    │  │ SecureKey    │  │ Network, Token   │  │
                    │  │ KeyEntry     │  │ Dex, Wallet      │  │
                    │  │ KeyInfo      │  │ Validation       │  │
                    │  │              │  │ + parsing helpers│  │
                    │  └──────────────┘  └─────────────────┘  │
                    │                                          │
                    │  re-exports: SafeContext                  │
                    └────────────────┬────────────────────────┘
                                     │ depends on
                    ┌────────────────▼────────────────────────┐
                    │            mfm_machine                   │
                    │                                          │
                    │  ┌──────────────┐  ┌─────────────────┐  │
                    │  │    state     │  │  state_machine   │  │
                    │  │              │  │                   │  │
                    │  │ Tag, Label   │  │ StateMachine     │  │
                    │  │ StateHandler │  │ Scheduler        │  │
                    │  │ StateError   │  │ ErrorHandler     │  │
                    │  │ Context      │  │ Tracker          │  │
                    │  │ SafeContext  │  │                   │  │
                    │  └──────────────┘  └─────────────────┘  │
                    └────────────────┬────────────────────────┘
                                     │ depends on
                    ┌────────────────▼────────────────────────┐
                    │         mfm_machine_derive               │
                    │  #[derive(StateMetadataReqs)]            │
                    └─────────────────────────────────────────┘
```

---

## 9. File Index

```
mfm/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── .gitignore
├── .cargo/audit.toml
├── flake.nix
├── flake.lock
│
├── mfm_machine/
│   ├── Cargo.toml
│   ├── README.md
│   ├── src/
│   │   ├── lib.rs
│   │   ├── state/
│   │   │   ├── mod.rs
│   │   │   ├── context.rs
│   │   │   └── safe_context.rs
│   │   └── state_machine/
│   │       ├── mod.rs
│   │       ├── scheduler.rs
│   │       └── tracker.rs
│   └── tests/
│       ├── default_impls.rs
│       ├── public_api_test.rs
│       ├── retry_workflow_state_machine.rs
│       └── n_states_with_n_ctxs.rs
│
├── mfm_machine_derive/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
│
├── mfm_core/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── config/
│   │   │   ├── mod.rs
│   │   │   ├── network.rs
│   │   │   ├── token.rs
│   │   │   ├── dexes.rs
│   │   │   └── authentication/
│   │   │       ├── mod.rs
│   │   │       ├── wallet.rs
│   │   └── keystore/
│   │       ├── mod.rs
│   │       ├── error.rs
│   │       └── tests.rs
│
├── mfm_cli/
│   ├── Cargo.toml
│   ├── README.md
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   └── cli/
│   │       ├── mod.rs
│   │       ├── command_result.rs
│   │       ├── keystore/
│   │       │   ├── mod.rs
│   │       │   ├── import.rs
│   │       │   ├── list.rs
│   │       │   └── delete.rs
│   │       └── utils/
│   │           ├── mod.rs
│   │           ├── input.rs
│   │           ├── keystore.rs
│   │           └── output.rs
│   └── tests/
│       ├── cli_tests.rs
│       ├── cli_e2e_tests.rs
│       ├── cli_environment_tests.rs
│       ├── json_output_integration.rs
│       ├── keystore_integration.rs
│       └── utils_tests.rs
```
