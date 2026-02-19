# Adapter Architecture Refactoring Plan

> Status: Draft proposal.
> Date: 2026-02-19.
> Scope: Refactor isolated transport/adapter implementations into a consistent two-layer adapter model.
> Migration policy: hard cutover. Internal breaking changes are allowed and preferred; no compatibility shims.

## 1. Motivation

Several live IO transport implementations are scattered across the workspace with no consistent location policy, no self-describing namespace registration, and ad-hoc workarounds in the app wiring layer. As the number of external integrations grows, the current ad-hoc placement makes transports hard to discover, reuse, and test independently.

This document proposes a canonical adapter architecture that generalizes the pattern already established by `mfm-collectors-evm` + `mfm-collectors-evm-jsonrpc-http`.

### Reference architecture (normative)

- [`docs/redesign.md`](redesign.md) -- design contract (source of truth).
- [`docs/architecture.md`](architecture.md) -- boundary map and placement rules.
- [`AGENTS.md`](../AGENTS.md) -- contribution and CI rules.

## 2. Current State

### 2.1 What works well

The foundational layering is sound:

```text
State::handle(ctx, io, rec)
  -> IoProvider::call(IoCall)            [machine trait]
    -> LiveIo (fact recording/replay)    [machine runtime]
      -> RouterLiveIoTransport           [namespace dispatch]
        -> concrete LiveIoTransport      [actual IO]
```

Three implementations already follow a clean pattern:

| Crate | Role | IO? |
|---|---|---|
| `mfm-collectors-evm` | Domain adapter: typed `EvmIoClient` over `IoProvider` | No |
| `mfm-collectors-evm-jsonrpc-http` | Live transport: HTTP JSON-RPC with failover/hedging | Yes |
| `mfm-evm-core` | Pure types, encoding, ABI, RLP | No |

States depend only on the domain adapter (`EvmIoClient`). The live transport is injected at the app level via namespace routing. This separation enables full testability with mock `IoProvider` implementations and transport swapping without state code changes.

### 2.2 Problems

#### P1: Transport factories have no location policy

| Transport | Current location | Problem |
|---|---|---|
| `NixFlakeTransportFactory` | `crates/ops/nix-app-op/src/nix_exec_transport.rs` | Op crate owns a transport; reuse requires depending on an unrelated op |
| `ExecProgramTransportFactory` | `crates/machine/src/exec_transport.rs` | Debatable; machine owns exec semantics, but the transport is app-level |
| `LocalEvmIoTransportFactory` | `crates/evm-runtime/src/local_evm_io.rs` | Runtime crate owns a local transport |
| `LocalFsIoTransportFactory` | `crates/ops/common/src/local_fs_io.rs` | Ops common owns a local transport |
| `LocalKeystoreIoTransportFactory` | `crates/ops/keystore-common/src/local_keystore_io.rs` | Keystore common owns a local transport |
| `EvmJsonRpcHttpTransportFactory` | `crates/collectors/evm-jsonrpc-http/` | Correct location |
| 3 passthrough transports | Inline in `crates/app/src/lib.rs` | Should not exist |

#### P2: Passthrough transports are workarounds

`AppLiveIoTransportFactory`, `PortfolioLiveIoTransportFactory`, and `AaveOutputLiveIoTransportFactory` in `crates/app/src/lib.rs` are identity functions that return `Ok(call.request)`. They exist so `LiveIo`'s fact-recording layer captures computed values. This abuses the transport layer for fact recording.

#### P3: Manual `local` sub-router duplicates routing logic

`AppLocalIoTransportFactory` in `crates/app/src/lib.rs` manually matches on `"local.keystore.*"`, `"local.evm.*"`, `"local.fs.*"`. This duplicates what `RouterLiveIoTransportFactory` already does at the top level.

#### P4: Factories do not self-describe their namespace

The route table in `make_engine_bundle()` is a hand-maintained `HashMap<String, Arc<dyn LiveIoTransportFactory>>` with string keys. Factories do not declare what namespace group they serve. The wiring is implicit and non-discoverable.

#### P5: Config resolution lives in the wrong place

`resolve_evm_rpc_config_from_env()` (reading ~12 env vars) is defined in `crates/app/src/lib.rs`, not in the EVM collector crate. The app crate has deep knowledge of domain-specific configuration.

#### P6: No transport registry

Ops have `OperationRegistry` for dynamic resolution. Transports are hardcoded in `make_engine_bundle()` with no equivalent registry or discovery mechanism.

## 3. Target Architecture

### 3.1 Two-layer adapter model

Generalize the `collectors/evm` + `collectors/evm-jsonrpc-http` pattern as the canonical model for all external integrations.

#### Layer 1: Domain adapter (logical, no IO)

A typed client that wraps `&mut dyn IoProvider` for a specific domain. States depend only on this layer.

Responsibilities:
- Typed `IoCall` construction (namespace, request shape).
- Deterministic `FactKey` derivation.
- Response parsing and domain error mapping.
- No direct network/filesystem/subprocess IO and no `LiveIoTransport` awareness.
- Methods remain async because they await `IoProvider`.

Existing example: `mfm-collectors-evm` (`EvmIoClient`).

#### Layer 2: Live transport (actual IO)

A `LiveIoTransportFactory` implementation that performs real IO. Registered at the app level.

Responsibilities:
- Implements `LiveIoTransportFactory` / `LiveIoTransport`.
- Owns its configuration and `from_env()` constructor.
- Performs actual IO (HTTP, subprocess, filesystem).
- Declares its namespace group.

Existing example: `mfm-collectors-evm-jsonrpc-http` (`EvmJsonRpcHttpTransportFactory`).

#### Pure types crates

Foundation crates that contain only types, encoding, and logic. No IO, no adapter traits.

Existing example: `mfm-evm-core`.

These do not change. They remain dependencies of domain adapters and live transports as needed.

### 3.2 Proposed workspace layout

```text
crates/
  machine/                          runtime IO contracts, executor, kernel events
    src/live_io.rs                  IoProvider/LiveIo transport contracts + LiveIo
    src/live_io_router.rs           hierarchical namespace routing
    src/live_io_registry.rs         NEW: TransportRegistry + HashMapTransportRegistry
    src/exec_transport.rs           exec transport stays here (machine-level concern)

  evm-core/                         pure types, encoding, ABI, RLP (unchanged)

  collectors/
    evm/                            domain adapter: EvmIoClient (unchanged)
    evm-jsonrpc-http/               live transport: EVM HTTP JSON-RPC (unchanged)
    nix/                            NEW: domain adapter: NixIoClient
    nix-exec/                       EXTRACTED: live transport: nix eval/build subprocess

  transports/                       NEW directory for non-collector live transports
    local-fs/                       EXTRACTED from ops/common
    local-evm/                      EXTRACTED from evm-runtime
    local-keystore/                 EXTRACTED from ops/keystore-common

  ops/
    common/                         reusable states (NixExecState, NamespaceReadState, etc.)
    nix-app-op/                     op definition only (no transport code)
    ...
```

The naming convention:
- `crates/collectors/<domain>/` -- domain adapter (logical layer) for external data sources.
- `crates/collectors/<domain>-<impl>/` -- live transport for external data sources.
- `crates/transports/<domain>/` -- live transport for local/internal concerns that are not "collectors" of external data.
- Namespace groups are stable dotted prefixes. Example: `nix.exec` handles `nix.exec.*`.

### 3.2.1 Registry ownership decision

`TransportRegistry` should live in `crates/machine` (`src/live_io_registry.rs`).

Rationale:
- It depends only on machine-owned contracts (`LiveIoTransportFactory`, `LiveIoEnv`).
- It is runtime wiring infrastructure, not domain behavior.
- It keeps `crates/app` thin (assembly only) and avoids a new crate that would only proxy machine IO traits.

### 3.3 Self-describing transport factories

Extend `LiveIoTransportFactory` (or add a companion trait) so factories declare their namespace:

```rust
pub trait LiveIoTransportFactory: Send + Sync {
    /// The namespace group this factory handles (e.g., "evm", "nix.exec", "local.fs").
    fn namespace_group(&self) -> &str;

    fn make(&self, env: LiveIoEnv) -> Box<dyn LiveIoTransport>;
}
```

The router can then be built from a `Vec<Arc<dyn LiveIoTransportFactory>>`:

```rust
impl RouterLiveIoTransportFactory {
    pub fn from_factories(factories: Vec<Arc<dyn LiveIoTransportFactory>>) -> Self {
        let routes = factories
            .into_iter()
            .map(|f| (f.namespace_group().to_string(), f))
            .collect();
        Self { routes }
    }
}
```

Registration collisions are invalid. If two factories report the same `namespace_group()`, registry construction fails.

### 3.4 Hierarchical routing

Support dotted namespace groups so `"local.fs"`, `"local.evm"`, `"local.keystore"` are registered as separate routes without a manual sub-router.

Current routing: splits on first `.` only (`"local.fs.read_text"` -> group `"local"`).

Proposed routing: longest-prefix match against registered groups:
- Registered groups: `["evm", "exec", "nix.exec", "local.fs", "local.evm", "local.keystore"]`
- `"evm"` -> `"evm"`
- `"local.fs.read_text"` -> `"local.fs"`
- `"local.evm.sign_legacy_create"` -> `"local.evm"`
- `"nix.exec"` -> `"nix.exec"`

This eliminates `AppLocalIoTransportFactory` entirely.

Deterministic routing algorithm:
1. Candidates are registered groups `g` where `namespace == g` or `namespace` starts with `g + "."`.
2. Choose the candidate with the longest `g`.
3. If none match, return stable error `io_unknown_namespace`.
4. Duplicate group registration is rejected during registry assembly.

Required tests:
- Overlap case: `local` and `local.fs` registered; `local.fs.read_text` resolves to `local.fs`.
- Exact match case: `nix.exec` resolves to `nix.exec`.
- No-match case: unknown namespace returns `io_unknown_namespace`.

### 3.5 Eliminate passthrough transports

The three passthrough transports (`proof`, `portfolio`, `aave`) exist because ops use `io.call()` with a fact key to get deterministic replay of computed outputs. The transport returns `Ok(call.request)` -- a no-op.

Options (choose one):

**Option A: Dedicated `identity` transport.**
Provide a reusable `IdentityTransportFactory` in `crates/machine/` that returns `Ok(call.request)` for any namespace. Register it for `"proof"`, `"portfolio"`, `"aave"`. This at least removes the inline code from `app/src/lib.rs` and makes the intent explicit.

**Option B: First-class fact recording on `IoProvider`.**
Add a method to `IoProvider` for recording computed values as facts without routing through a transport:

```rust
async fn record_value(&mut self, key: FactKey, value: serde_json::Value) -> Result<ArtifactId, IoError>;
```

States call `io.record_value(key, computed_output)` instead of `io.call(IoCall { namespace: "proof", request: computed_output, fact_key: Some(key) })`. This removes the need for passthrough transports entirely.

If implemented, this method must preserve current `LiveIo` guarantees:
- canonical JSON hashing for structured payloads
- secret-surface rejection before persistence
- single-assignment fact binding
- durable `fact_recorded` emission

**Option C: Keep passthrough transports but move to a shared utility.**
Least disruptive. Create a single `PassthroughTransportFactory` in `crates/machine/` that can be instantiated for any namespace group.

Chosen for this refactor: **Option A**.

Reason:
- Removes inline app workarounds immediately.
- Avoids changing `IoProvider` in the same refactor.
- Keeps replay/fact semantics unchanged in this phase.

Follow-up (separate breaking change): evaluate **Option B** with dedicated contract tests.

### 3.6 Transport registry

Introduce a `TransportRegistry` analogous to `OperationRegistry`:

```rust
pub struct RegistryError {
    pub message: String,
}

pub trait TransportRegistry: Send + Sync {
    fn register(&mut self, factory: Arc<dyn LiveIoTransportFactory>) -> Result<(), RegistryError>;
    fn resolve(&self, namespace_group: &str) -> Option<Arc<dyn LiveIoTransportFactory>>;
    fn all(&self) -> Vec<Arc<dyn LiveIoTransportFactory>>;
}
```

`make_engine_bundle()` in `crates/app/` then becomes:

```rust
let mut transports = HashMapTransportRegistry::new();
transports.register(Arc::new(EvmJsonRpcHttpTransportFactory::from_env()?))?;
transports.register(Arc::new(NixExecTransportFactory::from_env()?))?;
transports.register(Arc::new(ExecProgramTransportFactory::default()))?;
transports.register(Arc::new(LocalFsTransportFactory::default()))?;
transports.register(Arc::new(LocalEvmTransportFactory::default()))?;
transports.register(Arc::new(LocalKeystoreTransportFactory::default()))?;

let router = RouterLiveIoTransportFactory::from_registry(&transports);
```

### 3.7 Config ownership

Each transport factory owns its configuration and provides a `from_env()` constructor:

```rust
impl EvmJsonRpcHttpTransportFactory {
    pub fn from_env() -> Result<Self, ConfigError> {
        // reads MFM_EVM_RPC_SOURCES_JSON, MFM_EVM_RPC_URL, etc.
    }
}

impl NixExecTransportFactory {
    pub fn from_env() -> Result<Self, ConfigError> {
        // reads nix-specific env vars if any, or uses defaults
    }
}
```

`crates/app/src/lib.rs` calls these constructors without knowing domain-specific env var names.

Hard-cutover rule for this migration:
- App-level env parsing helpers are deleted.
- Transport crates own env naming and validation.
- Compatibility with previous app-level env aliases is not required.

## 4. Domain Adapter Clients To Create

### 4.1 `NixIoClient` (new: `crates/collectors/nix/`)

Typed client over `IoProvider` for nix exec preflight.

```rust
pub struct NixIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> NixIoClient<'a> {
    pub async fn resolve_flake_app(
        &mut self,
        app: &str,
        timeout_ms: u64,
    ) -> Result<NixResolveResult, IoError> {
        // builds IoCall { namespace: "nix.exec", request: { kind: "resolve_flake_app_v1", ... } }
        // derives fact key deterministically
        // parses response into NixResolveResult { program_path }
    }
}
```

States (like `NixExecState`) would use `NixIoClient` instead of building raw JSON `IoCall` requests by hand.

Request/response contract (v1):
- Request: `{ "kind": "resolve_flake_app_v1", "app": "<flake>#<fragment>", "timeout_ms": <u64> }`
- Response: `{ "program_path": "/nix/store/<hash>-<name>/bin/<app>" }`
- Namespace group: `nix.exec`

### 4.2 `ExecIoClient` (new: `crates/collectors/exec/` or inline in machine)

Typed client for program execution:

```rust
pub struct ExecIoClient<'a> { ... }

impl<'a> ExecIoClient<'a> {
    pub async fn run_program(
        &mut self,
        program_path: &str,
        input: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<ExecResult, IoError> { ... }
}
```

### 4.3 `LocalFsIoClient`, `LocalKeystoreIoClient`, `LocalEvmIoClient`

Typed clients for the `local.*` namespaces. These already have partial typed helpers (`local_io_helpers::local_call()` in `ops/common`), but each domain deserves its own client with proper request/response types.

## 5. Migration Sequence

Migration execution rule: each phase is a hard cutover. Update all in-repo consumers in the same change.

### Phase 1: Extract and relocate transports

Priority: high. Unblocks reuse and establishes location convention.

1. **Extract `NixFlakeTransportFactory`** from `crates/ops/nix-app-op/src/nix_exec_transport.rs` into `crates/collectors/nix-exec/`. Update `nix-app-op` to no longer own the transport. Update `crates/app/` import.

2. **Extract `LocalFsIoTransportFactory`** from `crates/ops/common/src/local_fs_io.rs` into `crates/transports/local-fs/`. Move required shared helpers from `crates/ops/common/src/local_transport.rs` with it. Update imports.

3. **Extract `LocalEvmIoTransportFactory`** from `crates/evm-runtime/src/local_evm_io.rs` into `crates/transports/local-evm/`. Update imports.

4. **Extract `LocalKeystoreIoTransportFactory`** from `crates/ops/keystore-common/src/local_keystore_io.rs` into `crates/transports/local-keystore/`. Update imports.

Quality gate:
- `nix run .#check`
- targeted unit tests for extracted transports

### Phase 2: Hierarchical routing

Priority: high. Eliminates the manual sub-router.

1. Update `RouterLiveIoTransportFactory` to support longest-prefix namespace matching.
2. Register `"local.fs"`, `"local.evm"`, `"local.keystore"` as separate routes (plus `"nix.exec"`).
3. Remove `AppLocalIoTransportFactory` from `crates/app/src/lib.rs`.

Quality gate:
- router overlap/no-match tests
- `nix run .#check`

### Phase 3: Self-describing factories

Priority: medium. Improves discoverability.

1. Add `namespace_group()` to `LiveIoTransportFactory` trait.
2. Update all factory implementations.
3. Add `RouterLiveIoTransportFactory::from_factories()`.
4. Simplify `make_engine_bundle()` route table construction.
5. Reject duplicate namespace-group registration.

Quality gate:
- duplicate-group registry tests
- `nix run .#check`

### Phase 4: Eliminate passthrough transports

Priority: medium. Removes workarounds.

1. Implement **Option A** (`IdentityTransportFactory`) in `crates/machine`.
2. Register identity transport for `proof`, `portfolio`, and `aave` namespace groups.
3. Remove `AppLiveIoTransportFactory`, `PortfolioLiveIoTransportFactory`, `AaveOutputLiveIoTransportFactory` from `crates/app/src/lib.rs`.
4. Update affected ops to use the shared identity transport wiring.

Quality gate:
- replay regression tests for proof/portfolio/aave outputs
- `nix run .#test`

### Phase 5: Domain adapter clients

Priority: lower. Improves state code ergonomics.

1. Create `crates/collectors/nix/` with `NixIoClient`.
2. Create `crates/collectors/exec/` with `ExecIoClient` (or add to machine).
3. Create typed clients for `local.*` namespaces.
4. Update states to use typed clients instead of raw `IoCall` construction.

Quality gate:
- state-level tests for typed adapter request/response parsing
- `nix run .#test`

### Phase 6: Transport registry and config ownership

Priority: lower. Good for scale.

1. Define `TransportRegistry` in `crates/machine/src/live_io_registry.rs`.
2. Move `resolve_evm_rpc_config_from_env()` into `mfm-collectors-evm-jsonrpc-http` as `EvmJsonRpcHttpTransportFactory::from_env()`.
3. Add `from_env()` constructors to other transport factories.
4. Delete app-level transport config helpers and refactor `make_engine_bundle()` to use registry-based assembly.

Quality gate:
- registry construction tests
- transport `from_env()` validation tests
- `nix run .#check && nix run .#test`

### Phase 7: Optional `IoProvider::record_value` redesign

Priority: deferred. Separate breaking change.

1. Design and implement Option B from section 3.5.
2. Prove invariants with tests: canonical JSON enforcement, secret rejection, durable `fact_recorded` binding.
3. Remove identity transport usage for pure computed-value recording if Option B fully replaces it.

## 6. What Does Not Change

- **`mfm-evm-core`** -- Correctly placed as a pure types/encoding crate. Not an adapter.
- **`mfm-collectors-evm`** -- Already the gold standard domain adapter. No changes needed.
- **`mfm-collectors-evm-jsonrpc-http`** -- Already the gold standard live transport. Config ownership moves in (Phase 6); env parsing may change as part of hard cutover.
- **`ExecProgramTransportFactory` in machine** -- Exec is a machine-level concern. Keeping it in machine is defensible since the engine itself needs to execute programs for child runs.
- **The `IoProvider` / `LiveIoTransport` split** -- This is the correct abstraction boundary. States see `IoProvider`; transports implement `LiveIoTransport`. Do not merge them.
- **`NixExecState` in `ops/common`** -- The state stays in ops/common; only the transport factory moves out of nix-app-op.

## 7. Invariants

All changes must preserve:

1. States never depend on `LiveIoTransport` or transport factories. States depend only on `IoProvider` (and optionally domain adapter clients that wrap `IoProvider`).
2. Transport factories never depend on state implementations or op crates.
3. Domain adapter clients (Layer 1) never perform ambient IO. All external effects flow through `IoProvider`.
4. The `LiveIo` fact-recording layer remains the single point for replay/determinism.
5. No secrets in any adapter request/response surface (per design contract 4.8).
6. Append-only event stream semantics are not affected (per design contract 4.1).
7. Each extraction/move is a hard cutover with all consumers updated in the same change (per architecture doc section 5: breaking internal refactors).
8. Routing decisions are deterministic (`longest-prefix`), and unknown namespaces fail with stable error codes.

## 8. Success Criteria

After all phases:

- Every `LiveIoTransportFactory` lives in a dedicated crate under `crates/collectors/` or `crates/transports/`.
- `TransportRegistry` lives in `crates/machine` and is used by `crates/app` for transport assembly.
- Every transport factory declares its namespace group via the trait.
- `crates/app/src/lib.rs` contains no inline transport implementations.
- `crates/app/src/lib.rs` contains no domain-specific config resolution logic.
- No op crate owns a transport factory.
- States that need typed IO use domain adapter clients (like `EvmIoClient`), not raw `IoCall` JSON construction.
- The router supports hierarchical namespace matching without manual sub-routers.
- Router tests cover overlapping prefix resolution and unknown namespace behavior.
