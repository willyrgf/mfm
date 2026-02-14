# Three-Tier Thin-Layer Alignment Audit

Date: 2026-02-14 (updated)
Status: Progress checkpoint; Phase 2 complete, expand() boundary verified clean, utility duplication unchanged

## 0. Architectural Vision (Three-Tier Model)

This audit enforces a stricter architectural principle than the previous checkpoint:

```
bin/{cli,rest-api}   (THIN) parse input, choose op/pipeline, render output
       |
       v
  crates/ops/*       (THIN) validate config, wire state graphs via expand()
       |
       v
crates/ops/common/   (THICK) reusable single-responsibility states carrying
  src/states/*               all logic, composable across operations
       |               +
  src/utils/           shared pure-function utility modules (encoding, hex,
                       RLP, ABI) consumed by states and IO layers
       |
       v
adapters/IO drivers  collectors, local_io, storages (infrastructure)
```

Key principle: not only must business logic move out of binaries, it must also move out of ops. Ops should be thin configuration + graph wiring layers. All executable logic lives in reusable states that are single-responsibility and composable across operations.

### 0.1 Composability as Core Design (Repo-Wide Standard)

The three-tier model is not an aspirational refactoring goal; it is a **repo-wide standardization** that every existing and future crate must conform to. Composability is the first-class design principle that this architecture serves:

1. **States are the atoms of composition.** Every discrete unit of executable logic (a JSON-RPC call, a balance read, a receipt poll, a side effect) is a standalone state in `crates/ops/common/src/states/`. States are IO-abstracted, single-responsibility, and independently testable.

2. **Ops are the molecules.** An op's `expand()` selects which atoms to compose and how to wire their dependency graph. Ops carry zero execute logic; they are declarative assembly manifests.

3. **Utilities are the constants.** Pure functions for encoding, decoding, hex manipulation, RLP, and ABI live in shared utility modules under `crates/ops/common/src/`. They are consumed by states and IO layers but never own side effects.

4. **Binaries are the shell.** `bin/cli` and `bin/rest-api` parse user intent, invoke the op/pipeline catalog, and render output. They never reach past the op boundary.

This model guarantees:
- **New ops compose existing behavior.** A DeFi op needing balance reads wires the same `NativeBalanceState` and `TokenBalanceState` that `portfolio_tracker` uses.
- **Behavioral changes propagate.** Fixing a receipt-polling edge case in the shared `EvmReceiptPollState` fixes every op that polls receipts.
- **Testing is layered.** Utility functions get pure unit tests, states get IO-mocked execute tests, ops get graph-wiring tests, and integration tests validate end-to-end pipelines.

Documents that codify this standard and must stay aligned:
- `REDESIGN.md` (authoritative design contract, especially sections 1.4, 10, 17.1)
- `ARCHITECTURE.md` (one-pager, composition model section)
- `ENFORCE_BINS_THIN_LAYER.md` (binary boundary rules)
- `AGENTS.md` (AI development guide, non-negotiable architecture invariants)

## 1. Contract Baseline

The audit scores implementation against these explicit statements plus the new three-tier principle:

Existing contract (bins thin):
- `REDESIGN.md:52` binaries are thin wrappers that start/resume runs.
- `REDESIGN.md:59` everything executes inside a state machine.
- `REDESIGN.md:220` binaries are thin wrappers.
- `ARCHITECTURE.md:195` `bin/cli` and `bin/rest-api` are thin wrappers.
- `ENFORCE_BINS_THIN_LAYER.md:7` binaries are thin wrappers.
- `ENFORCE_BINS_THIN_LAYER.md:8` business execution must happen inside state-machine runs.

Existing contract (ops expand to states):
- `REDESIGN.md:527` ops expand into state graphs.
- `REDESIGN.md:830,836` route state side effects through IO provider.
- `ARCHITECTURE.md:167` `crates/ops/common/` owns reusable `State` implementations.
- `ARCHITECTURE.md:176` `crates/ops/*` reuses shared primitives from `crates/ops/common/` when possible.

New three-tier principle (this audit enforces):
- Ops are also thin: `expand()` does config validation + state graph wiring only.
- States are the primary reusable composition unit: single-responsibility, IO-abstracted, composable across ops.
- Op-local states are a code smell; the default path is shared states in `crates/ops/common/src/states/`.

## 2. Audit Rubric

### 2.1 Boundary Tags

- `B1`: Business logic in binary layer (`bin/cli` or `bin/rest-api`).
- `B2`: Business workflow logic in app adapter layer (`crates/app`).
- `B3`: Operation packaging/ownership ambiguity.
- `B4`: Ambient IO in state/business path outside IO abstraction.
- `B5`: Secret handling risk at boundaries.
- `B6`: Reusable state-library gap (op-local states where shared states should exist).
- `B7`: Op thickness (business logic embedded in op `expand()` or op-level helpers instead of delegated to reusable states).
- `B8`: Utility duplication (pure functions duplicated across crates instead of shared from a single module).

### 2.2 Three-Tier Status Labels

- `Aligned`: layer is thin (bins/ops) or thick-reusable (states) as intended.
- `Partially aligned`: mostly correct but has residual logic in wrong layer.
- `Not aligned`: significant logic in wrong layer.
- `Packaging gap`: naming/ownership ambiguity.

### 2.3 Op Thinness Rating

Each op is rated for how well it follows the "ops are thin" principle:

- `Thin`: `expand()` does only config validation + state graph wiring; all logic in shared states.
- `Mixed`: `expand()` does config validation + wiring, but delegates to op-local states instead of shared.
- `Thick`: op contains significant business logic beyond config validation and graph assembly.

## 3. Coverage

Audited surfaces:

- CLI command surfaces: 13
- REST routes: 9
- Feature IDs: 7
- Registered runtime ops: 14
- Shared reusable state implementations in `ops/common/src/states/`: 9
- Op-local state implementations across op crates: 11
- Total state implementations: 20
- Shared state modules: 4 (`evm.rs`, `evm_dcv.rs`, `keystore_admin.rs`, `meta.rs`)

Primary entry/registry evidence:

- CLI command roots: `bin/cli/src/commands/mod.rs:42`
- REST route map: `bin/rest-api/src/lib.rs:146`
- Feature registry: `crates/app/src/lib.rs:1072`
- Runtime op registry: `crates/app/src/lib.rs:384`

## 4. Three-Tier Conformance Matrix

### 4.1 Tier 1: Binary Layer (Target: Thin)

#### CLI Commands

| CLI Surface | Tier Classification | Issues | Status |
|---|---|---|---|
| `mfm keystore import` | Pure wrapper | Minor: sets env var for BIP39 passphrase at CLI level | Aligned |
| `mfm keystore list` | Pure wrapper | None | Aligned |
| `mfm keystore delete` | Pure wrapper | None | Aligned |
| `mfm keystore tx-sign` | Pure wrapper | None | Aligned |
| `mfm keystore tx-send-raw` | Pure wrapper | None | Aligned |
| `mfm portfolio snapshot` | Light logic | JSON array validation in CLI (`B1`) | Partially aligned |
| `mfm run start` | Light logic | JSON validation in CLI | Aligned |
| `mfm run pipeline start` | Light logic | JSON/file validation in CLI | Aligned |
| `mfm run pipeline deploy-configure-validate` | Has logic | File I/O + spec parsing in CLI (`B1`) | Partially aligned |
| `mfm run resume` | Light logic | UUID validation in CLI | Aligned |
| `mfm run status` | Light logic | UUID validation in CLI | Aligned |
| `mfm run events` | Light logic | UUID validation in CLI | Aligned |
| `mfm run artifacts get` | Pure wrapper | None | Aligned |

Tier 1 summary: 11/13 Aligned, 2/13 Partially aligned. REST API is fully thin (all 9 routes are pure wrappers). CLI has minor validation logic leakage but no business/domain logic.

#### REST Routes

| Route | Tier Classification | Status |
|---|---|---|
| `GET /v1/health` | Pure wrapper | Aligned |
| `GET /v1/ready` | Infra probe (acceptable) | Aligned |
| `GET /v1/features` | Pure wrapper | Aligned |
| `POST /v1/features/:feature_id/execute` | Pure wrapper -> catalog | Aligned |
| `POST /v1/runs/start` | Pure wrapper | Aligned |
| `POST /v1/runs/:run_id/resume` | Pure wrapper | Aligned |
| `GET /v1/runs/:run_id/status` | Pure wrapper | Aligned |
| `GET /v1/runs/:run_id/events` | Pure wrapper | Aligned |
| `GET /v1/artifacts/:artifact_id` | Pure wrapper | Aligned |

REST summary: 9/9 Aligned. Fully thin.

### 4.2 Tier 2: Ops Layer (Target: Thin config + graph wiring)

| Op | Op Thinness | States Used | Shared States | Op-Local States | `B7` Status | Evidence |
|---|---|---|---|---:|---|---|
| `keystore_import` | Thin | 1 | 1 (`KeystoreImportState`) | 0 | Aligned | `keystore-admin-op/src/lib.rs:101` |
| `keystore_list` | Thin | 1 | 1 (`KeystoreListState`) | 0 | Aligned | `keystore-admin-op/src/lib.rs:160` |
| `keystore_delete` | Thin | 1 | 1 (`KeystoreDeleteState`) | 0 | Aligned | `keystore-admin-op/src/lib.rs:229` |
| `keystore_tx_sign` | Mixed | 1 | 0 | 1 (`TxSignState`) | Partially aligned | `keystore-tx-op/src/lib.rs:129` |
| `keystore_tx_send_raw` | Mixed | 1 | 0 | 1 (`TxSendRawState`) | Partially aligned | `keystore-tx-op/src/lib.rs:207` |
| `evm_read` | Thin | 2 | 2 (`ReadU64HexState` x2) | 0 | Aligned | `evm-read-op/src/lib.rs:61` |
| `evm_contract_from_nix` | Mixed | 1 | 0 | 1 (`ContractFromNixState`) | Partially aligned | `evm-write-op/src/lib.rs:1254` |
| `evm_deploy` | Mixed | 1 | 0 | 1 (`DeployState`) | Partially aligned | `evm-write-op/src/lib.rs:1384` |
| `evm_configure` | Mixed | 1 | 0 | 1 (`ConfigureState`) | Partially aligned | `evm-write-op/src/lib.rs:1560` |
| `evm_validate` | Mixed | 1 | 0 | 1 (`ValidateState`) | Partially aligned | `evm-write-op/src/lib.rs:1777` |
| `evm_deploy_configure_validate` | Thin | 3 (composed) | 0 (delegates to child ops) | 0 | Aligned | `evm-deploy-configure-validate-op/src/lib.rs:26` |
| `portfolio_tracker` | Mixed | 4+ | 4 (`ReadU64HexState` x2, `NativeBalanceState`, `TokenBalanceState` xN) | 1 (`WriteSnapshotState`) | Partially aligned | `portfolio-tracker-op/src/lib.rs:265` |
| `proof` | Mixed | 3 | 0 | 3 | Partially aligned | `proof-op/src/lib.rs:132` |
| `nix_app` | Mixed | 1 | 0 | 1 (`NixAppState`) | Partially aligned | `nix-app-op/src/lib.rs:100` |

Tier 2 summary: 5/14 ops are Thin (using only shared states or pure composition). 9/14 are Mixed (delegate to at least one op-local state). All 14/14 expand() methods are structurally thin (config validation + graph wiring only; zero business logic in expand).

Note: all expand() methods pass the op-expand boundary check (no `.await`, no file/network IO, no ambient side effects in expand). The "Mixed" rating reflects op-local state usage, not expand() thickness.

### 4.3 Tier 3: State Layer (Target: Thick, Reusable, Single-Responsibility)

#### Shared States (in `crates/ops/common/src/states/`)

| State | Module | Responsibility | Used By | Reuse Count |
|---|---|---|---|---:|
| `ReadHexStringState` | `evm.rs:77` | Execute JSON-RPC call, parse hex string response | evm-read-op, portfolio-tracker-op | 2 |
| `ReadU256HexState` | `evm.rs:127` | Execute JSON-RPC call, parse U256 hex response | evm-read-op, portfolio-tracker-op | 2 |
| `EthCallState` | `evm.rs:185` | Execute `eth_call` with configurable response decoding | evm-read-op, portfolio-tracker-op | 2 |
| `ReadU64HexState` | `evm.rs:265` | Execute JSON-RPC call, parse u64 hex with optional validation | evm-read-op, portfolio-tracker-op | 2+ |
| `NativeBalanceState` | `evm.rs:445` | Fetch native ETH balance at pinned block | portfolio-tracker-op | 1 |
| `TokenBalanceState` | `evm.rs:517` | Fetch ERC-20 balance with decimals resolution | portfolio-tracker-op | 1 |
| `KeystoreImportState` | `keystore_admin.rs:138` | Import key into keystore via local IO | keystore-admin-op | 1 |
| `KeystoreListState` | `keystore_admin.rs:215` | List keystore keys via local IO | keystore-admin-op | 1 |
| `KeystoreDeleteState` | `keystore_admin.rs:291` | Delete keystore key via local IO | keystore-admin-op | 1 |

Shared states summary: 9 implementations (was 7). EVM read states have cross-op reuse (2+ ops each). `NativeBalanceState` and `TokenBalanceState` were extracted from portfolio-tracker-op (Phase 2 complete). Keystore admin states are shared-located but single-op consumers.

Shared support modules (no State impls, but shared across states):
- `meta.rs`: `StateMeta` factory functions (`fetch_data()`, `validate()`, `pure()`, `config()`, `apply_side_effect()`, `execute()`). Used by all shared states for consistent metadata construction.
- `evm_dcv.rs`: Shared pure functions for the deploy-configure-validate pipeline (`parse_abi`, `normalize_hex_str`, `hex_to_bytes`, `bytes_to_hex_prefixed`, `parse_value_wei_to_hex`, etc.). Contains config types (`ContractArtifactConfig`, `ConfigureCallConfig`, `ReadAssertionConfig`, `EventAssertionConfig`) and validation helpers. No State implementations.

#### Op-Local States (candidates for extraction)

| State | Op Crate | Responsibility | Extractable | Target Shared Module |
|---|---|---|---|---|
| `TxSignState` | keystore-tx-op:260 | Sign tx via local IO, emit event | Yes | `states/keystore_tx.rs` |
| `TxSendRawState` | keystore-tx-op:335 | Read file + submit raw tx via RPC | Yes | `states/keystore_tx.rs` |
| `ContractFromNixState` | evm-write-op:1294 | Adapt nix output to EVM contract artifact | Yes | `states/evm.rs` (artifact parsing) |
| `DeployState` | evm-write-op:1449 | Build deploy payload, sign, submit, poll receipt | Yes (decompose) | `states/evm_write.rs` |
| `ConfigureState` | evm-write-op:1660 | Encode function calls, submit txs, collect receipts | Yes (decompose) | `states/evm_write.rs` |
| `ValidateState` | evm-write-op:1856 | Verify chain ID, read/event assertions | Yes (decompose) | `states/evm_write.rs` |
| `WriteSnapshotState` | portfolio-tracker-op:377 | Aggregate balances into output artifact | Partially | Keep op-local (domain-specific) |
| `ReadFactsState` | proof-op:192 | Read input facts from namespace | Yes | `states/io.rs` |
| `ApplySideEffectState` | proof-op:230 | Idempotent side effect with event emission | Yes | `states/side_effect.rs` |
| `WriteOutputState` | proof-op:304 | Assemble output artifact | Partially | Keep op-local (domain-specific) |
| `NixAppState` | nix-app-op:190 | Execute nix program with optional preflight | Partially | `states/nix.rs` |

Op-local states summary: 11 implementations (was 13; 2 extracted in Phase 2). Of these, 6 are clearly extractable to shared modules, 3 are partially extractable (domain-specific but with reusable patterns), and 2 should remain op-local (genuinely domain-specific aggregation/output).

## 5. State Reuse Ratio

| Metric | Previous | Current |
|---|---:|---:|
| Total state implementations | 20 | 20 |
| Shared (in `ops/common/src/states/`) | 7 | 9 |
| Op-local (in individual op crates) | 13 | 11 |
| Shared ratio | 35% | 45% |
| Target shared ratio | 80%+ | 80%+ |
| States with cross-op reuse today | 4 (EVM read states) | 6 (EVM read + balance states) |

Current composition: op-local states still dominate at 55%. The shared ratio improved from 35% to 45% through Phase 2 extraction (portfolio balance states). The three-tier model requires further extraction to reach the 80%+ target.

## 6. Findings (Ordered by Severity)

### High

1. `B7` Op-local states still dominate composition.
- Impact: 9 of 14 ops use at least one op-local state. Business logic in evm-write-op, proof-op, keystore-tx-op, and nix-app-op cannot be reused across operations.
- Evidence: `crates/ops/evm-write-op/src/lib.rs` contains 4 op-local states (~1400 lines of execute logic). `crates/ops/proof-op/src/lib.rs` contains 3 op-local states. `crates/ops/keystore-tx-op/src/lib.rs` contains 2 op-local states.
- Progress: portfolio-tracker-op reduced from 3 to 1 op-local state (Phase 2 complete).
- Risk: new ops cannot compose existing behavior; duplicated patterns across ops; slower evolution toward composition-first architecture.
- Remediation: extract op-local states to `crates/ops/common/src/states/` per the extraction plan in section 8 (Phases 1, 3, 4 remaining).

1b. `B8` Utility functions duplicated across crates instead of shared.
- Impact: 11 pure functions are duplicated 2-3 times across `evm-write-op`, `common/states/evm_dcv.rs`, `common/local_io.rs`, and `common/keystore_tx.rs`. Maintenance changes must be applied in multiple places; bugs fixed in one copy persist in others; test coverage is fragmented (some copies tested, others not).
- Evidence: see section 7 (Utility Duplication Audit) for full inventory.
- Progress: no change since last audit; duplication baselines are enforced by guardrail but not yet reduced.
- Risk: correctness drift between copies; duplicated test burden; slower onboarding.
- Remediation: consolidate into shared utility modules per the plan in section 7.4.

### Medium

2. `B6/B7` EVM write states are monolithic and not decomposed.
- Impact: `DeployState`, `ConfigureState`, and `ValidateState` in `evm-write-op` each contain large execute methods with multiple responsibilities (payload construction, signing, receipt polling, validation). These should be decomposed into smaller reusable states.
- Evidence: `crates/ops/evm-write-op/src/lib.rs:1449` (DeployState), `crates/ops/evm-write-op/src/lib.rs:1660` (ConfigureState), `crates/ops/evm-write-op/src/lib.rs:1856` (ValidateState).
- Risk: receipt polling pattern is duplicated between deploy and configure; signing delegation is duplicated; validation patterns cannot be reused by other ops.

3. ~~`B6` Portfolio balance states are extractable EVM primitives.~~ **RESOLVED.**
- `NativeBalanceState` (was `ReadEthBalanceState`) extracted to `crates/ops/common/src/states/evm.rs:445`.
- `TokenBalanceState` (was `ReadErc20BalanceState`) extracted to `crates/ops/common/src/states/evm.rs:517`.
- `portfolio_tracker` op now wires shared balance states; only `WriteSnapshotState` remains op-local (domain-specific aggregation, correctly kept local).

4. ~~Guardrails for ambient IO regressions are not yet automated.~~ **PARTIALLY RESOLVED.**
- Three guardrail scripts were implemented:
  - `check-op-expand-boundary.sh`: enforces that `expand()` methods contain no `.await`, file/network IO, or ambient side effects. Currently passes for all 14 ops.
  - `check-utility-single-source.sh`: enforces per-function duplication baselines (max 3 for hex, max 2 for RLP/ABI). Prevents duplication from increasing.
  - `check-shared-state-ratio.sh`: enforces minimum shared-state ratio (baseline 45%). **Note: has a counting bug** where AWK skips non-test State impls in files where `#[cfg(test)]` attributes appear early for conditional imports (e.g. proof-op, evm-write-op). Reports 9/13 (69%) instead of the actual 9/20 (45%).
- Remaining gap: these scripts are not yet integrated into CI. They should be converted to Rust-based checks or integrated into the Nix CI pipeline, and the counting bug must be fixed.

5. Parity validation is still pending for this checkpoint.
- Impact: architecture-level changes are landed but not yet validated through full parity gates.
- Evidence: expected gates are `nix run .#check`, `nix run .#test`, `nix run .#ci -- --basic --summary`, `nix run .#ci -- --parity --summary`.
- Risk: unnoticed behavior drift.

### Low

6. `B1` Minor validation logic in CLI layer.
- Impact: `portfolio snapshot` validates JSON array in CLI (`bin/cli/src/commands/portfolio/snapshot.rs:33`); `run pipeline deploy-configure-validate` reads spec files and parses JSON in CLI (`bin/cli/src/commands/run/pipeline.rs:115`).
- Risk: low; these are input validation not business logic, but should ideally be pushed to ops.

7. `B3` Keystore ops packaging naming ambiguity.
- Impact: `mfm-op-keystore` implies executable op behavior but only re-exports core keystore types.
- Evidence: `crates/ops/keystore-op/src/lib.rs:8`.
- Risk: boundary confusion.

8. `B7` Proof op states are generic patterns locked in a domain-specific crate.
- Impact: `ApplySideEffectState` implements a generic idempotent-side-effect-with-event pattern. `ReadFactsState` implements generic namespace-read. Both are locked in proof-op.
- Evidence: `crates/ops/proof-op/src/lib.rs:192` (ReadFactsState), `crates/ops/proof-op/src/lib.rs:230` (ApplySideEffectState).
- Risk: other ops needing idempotent side effects would duplicate the pattern.

9. Test doubles (`NoopRecorder`, `CountingTransport`) are duplicated across op test modules instead of shared.
- Impact: `NoopRecorder` is defined independently in 4 test modules (proof-op, nix-app-op, evm-read-op, evm.rs). `CountingTransport` is defined in 2 test modules (proof-op, evm-read-op).
- Evidence: `proof-op/src/lib.rs:429`, `nix-app-op/src/lib.rs:393`, `evm-read-op/src/lib.rs:335`, `common/src/states/evm.rs:705`.
- Risk: maintenance burden; inconsistent mock behavior.

## 7. Utility Duplication Audit (`B8`)

The three-tier model requires that pure utility functions live in shared modules, not scattered across op crates and IO layers. This section catalogs every duplicated function discovered in the current codebase.

### 7.1 Duplication Inventory

#### Hex Encoding/Decoding (3 copies each)

| Function | Location 1 | Location 2 | Location 3 |
|---|---|---|---|
| `normalize_hex_str()` | `evm-write-op/src/lib.rs:354` | `common/states/evm_dcv.rs:156` (pub) | `common/local_io.rs:993` (different error type) |
| `hex_to_bytes()` | `evm-write-op/src/lib.rs:372` | `common/states/evm_dcv.rs:174` (pub) | `common/local_io.rs:1008` (different error type) |
| `bytes_to_hex_prefixed()` | `evm-write-op/src/lib.rs:381` | `common/states/evm_dcv.rs:183` (pub) | `common/local_io.rs:1004` |

These are the most pervasive duplications. The three copies differ only in error return types (`String` vs `()`).

#### RLP Encoding (2 copies each)

| Function | Location 1 (`keystore_tx.rs`) | Location 2 (`local_io.rs`) | Variation |
|---|---|---|---|
| `trim_leading_zero_bytes()` | `keystore_tx.rs:486` | `local_io.rs:1018` | Identical |
| `u128_to_min_be()` | `keystore_tx.rs:508` | `local_io.rs:1026` | Identical |
| `usize_to_min_be()` | `keystore_tx.rs:561` | `local_io.rs:1079` | Different: 0 returns `vec![]` vs `vec![0]` |
| `rlp_encode_bytes()` | `keystore_tx.rs:522` | `local_io.rs:1040` | Identical |
| `rlp_encode_list()` | `keystore_tx.rs:541` | `local_io.rs:1059` | `local_io` calls `rlp_encode_bytes()` on each item |

These are critical for EIP-1559 transaction encoding and are entirely untested in one copy.

#### ABI Parsing/Encoding (2 copies each)

| Function | Location 1 (`evm-write-op`) | Location 2 (`evm_dcv.rs`) | Notes |
|---|---|---|---|
| `parse_abi()` | `evm-write-op/src/lib.rs:316` | `common/states/evm_dcv.rs:118` (pub) | Identical logic, extracts constructor/functions/events |
| `encode_params()` | `evm-write-op/src/lib.rs:563` | `common/states/evm_dcv.rs:251` | evm-write-op version is more complete (handles dynamic types) |
| `parse_value_wei_to_hex()` | `evm-write-op/src/lib.rs:695` | `common/states/evm_dcv.rs:397` (pub) | Identical |

#### Signer Helpers

| Function | Location 1 | Location 2 | Notes |
|---|---|---|---|
| `signer_address_hex()` | `common/local_io.rs:898` | `evm-write-op/src/lib.rs:1067` (`#[cfg(test)]`) | Test-only in evm-write-op |

### 7.2 Semantic Duplication (Same Pattern, Different Implementations)

Beyond copy-pasted functions, several patterns are reimplemented with slight variations:

| Pattern | Instances | Evidence |
|---|---|---|
| 32-byte word encoding (address padding) | `encode_address_word()` in evm_dcv.rs:198; `parse_address_hex()` + manual padding in evm-write-op:401 | Same intent, different APIs |
| Function selector computation | `function_selector()` in evm_dcv.rs:277 (keccak256 first 4 bytes); `selector()` in evm-write-op:597 | Same algorithm, different names |
| `u64_to_min_be()` | `keystore_tx.rs:494` | Only one copy, but should be co-located with `u128_to_min_be()` |
| ERC-20 selectors (hardcoded bytes) | `erc20_selector_balance_of()` in evm.rs:420; manual `[0x70,0xa0,0x82,0x31]` in portfolio-tracker-op | Constant duplication risk (reduced: portfolio-tracker-op now imports from shared evm.rs) |

### 7.3 Test Coverage of Duplicated Functions

| Function | Tested Copy | Untested Copies | Gap Severity |
|---|---|---|---|
| `normalize_hex_str()` | evm-write-op `#[cfg(test)]` | local_io.rs, evm_dcv.rs | Medium |
| `hex_to_bytes()` | evm-write-op `#[cfg(test)]` | local_io.rs, evm_dcv.rs | Medium |
| `bytes_to_hex_prefixed()` | evm-write-op `#[cfg(test)]` | local_io.rs, evm_dcv.rs | Medium |
| `parse_abi()` | evm-write-op (via `resolve_function_call` tests) | evm_dcv.rs | Low |
| `trim_leading_zero_bytes()` | **None** | keystore_tx.rs, local_io.rs | High |
| `u128_to_min_be()` | **None** | keystore_tx.rs, local_io.rs | High |
| `rlp_encode_bytes()` | **None** | keystore_tx.rs, local_io.rs | High |
| `rlp_encode_list()` | 1 test in keystore_tx.rs | local_io.rs | Medium |
| `usize_to_min_be()` | **None** | keystore_tx.rs (returns `vec![]` for 0), local_io.rs (returns `vec![0]`) | High (behavioral divergence) |
| `parse_value_wei_to_hex()` | **None** | evm-write-op, evm_dcv.rs | Medium |

Note: `usize_to_min_be()` has an actual **behavioral divergence** between copies: `keystore_tx.rs` returns empty vec for input 0, while `local_io.rs` returns `vec![0]`. This is a correctness risk.

### 7.4 Consolidation Plan

Target: create shared utility modules in `crates/ops/common/src/` that all crates import.

| Target Module | Functions to Consolidate | Current Locations |
|---|---|---|
| `hex.rs` | `normalize_hex_str`, `hex_to_bytes`, `bytes_to_hex_prefixed`, `hex_nibble` | evm-write-op, evm_dcv.rs, local_io.rs |
| `rlp.rs` | `trim_leading_zero_bytes`, `u64_to_min_be`, `u128_to_min_be`, `usize_to_min_be`, `rlp_encode_bytes`, `rlp_encode_list` | keystore_tx.rs, local_io.rs |
| `abi.rs` | `parse_abi`, `ParsedAbi`, `function_selector`, `encode_params`, `encode_address_word`, `parse_value_wei_to_hex`, `is_dynamic_type`, `pad_to_32`, `parse_bool_word`, `parse_bytes_m_word` | evm-write-op, evm_dcv.rs |
| `evm_encoding.rs` | `encode_u64_word`, `encode_len_word`, `parse_address_hex`, ERC-20 selector constants | evm-write-op, evm.rs |

Error handling unification: all shared utility functions should return `Result<T, UtilError>` where `UtilError` is a lightweight enum that callers map into their domain-specific error types (e.g. `StateError`, IO error). This eliminates the divergent error signatures that caused the duplication in the first place.

## 8. State Extraction Plan (Priority Order)

### Phase 1: EVM Write States (Highest impact) -- PENDING

Extract from `crates/ops/evm-write-op/src/lib.rs` into `crates/ops/common/src/states/evm_write.rs`:

| Current Op-Local State | Target Shared State(s) | Decomposition |
|---|---|---|
| `ContractFromNixState` | `NixArtifactToEvmContractState` | Move as-is; artifact parsing is reusable |
| `DeployState` | `EvmDeployState` + `EvmReceiptPollState` | Decompose: payload build + sign + submit is one state; receipt polling is a reusable state |
| `ConfigureState` | `EvmMulticallState` + reuse `EvmReceiptPollState` | Decompose: function encoding + submission per call; reuse receipt polling |
| `ValidateState` | `EvmAssertionState` | Move as-is or decompose into read-assertion + event-assertion |

Exit criteria: `evm_deploy`, `evm_configure`, `evm_validate` ops become thin wrappers that only validate config and wire shared states in `expand()`.

### Phase 2: Portfolio Balance States -- COMPLETE

Extracted from `crates/ops/portfolio-tracker-op/src/lib.rs` into `crates/ops/common/src/states/evm.rs`:

| Previous Op-Local State | New Shared State | Status |
|---|---|---|
| `ReadEthBalanceState` | `NativeBalanceState` (`evm.rs:445`) | Done |
| `ReadErc20BalanceState` | `TokenBalanceState` (`evm.rs:517`) | Done |
| `WriteSnapshotState` | Kept op-local (`portfolio-tracker-op/src/lib.rs:377`) | Done (correctly domain-specific) |

Result: `portfolio_tracker` op now wires 4 shared states (`ReadU64HexState` x2, `NativeBalanceState`, `TokenBalanceState` xN) plus 1 domain-specific op-local state. This validates the composition model: new ops needing balance reads can wire the same shared states.

### Phase 3: Keystore TX States -- PENDING

Extract from `crates/ops/keystore-tx-op/src/lib.rs` into `crates/ops/common/src/states/keystore_tx.rs`:

| Current Op-Local State | Target Shared State | Notes |
|---|---|---|
| `TxSignState` | `KeystoreTxSignState` | Already delegates to `local_call()`; move to shared |
| `TxSendRawState` | `KeystoreTxSendRawState` | Already delegates to shared `send_raw_transaction_via_io()`; move to shared |

Exit criteria: `keystore_tx_sign` and `keystore_tx_send_raw` ops become as thin as the keystore-admin ops.

### Phase 4: Generic Patterns -- PENDING

Extract from `crates/ops/proof-op/src/lib.rs` into `crates/ops/common/src/states/`:

| Current Op-Local State | Target Shared State | Target Module |
|---|---|---|
| `ApplySideEffectState` | `IdempotentSideEffectState` | `states/side_effect.rs` |
| `ReadFactsState` | `NamespaceReadState` | `states/io.rs` |
| `WriteOutputState` | Keep op-local (domain-specific) | N/A |

Move `NixAppState` pattern:

| Current Op-Local State | Target Shared State | Target Module |
|---|---|---|
| `NixAppState` | `NixExecState` | `states/nix.rs` |

Exit criteria: proof-op and nix-app-op become thin wrappers wiring shared states.

## 9. Target State After Extraction

| Metric | Current | After Phase 1,3,4 + Utility Consolidation |
|---|---:|---:|
| Shared state implementations | 9 | 18+ |
| Op-local state implementations | 11 | 3-4 |
| Shared ratio | 45% | 82%+ |
| Ops rated "Thin" | 5/14 | 12/14 |
| Ops rated "Mixed" | 9/14 | 2/14 |
| Duplicated utility functions | 11 (across 24 copies) | 0 (single canonical copy each) |
| Shared utility modules | 0 | 4 (`hex.rs`, `rlp.rs`, `abi.rs`, `evm_encoding.rs`) |

## 10. Exemplar Ops (Reference Patterns)

### Exemplar: Thin Op (evm_read)

`evm_read` is the reference implementation for the three-tier model:
- `expand()` validates config (which queries to enable) and wires 1-2 shared `ReadU64HexState` instances.
- Zero op-local states. Zero business logic in the op crate.
- All execution logic is in `crates/ops/common/src/states/evm.rs`.
- Evidence: `crates/ops/evm-read-op/src/lib.rs:61`

### Exemplar: Thin Composed Op (evm_deploy_configure_validate)

`evm_deploy_configure_validate` is the reference for pipeline composition:
- `expand()` delegates to three child ops, chains their state graphs, deduplicates exports.
- Zero op-local states. Pure orchestration.
- Evidence: `crates/ops/evm-deploy-configure-validate-op/src/lib.rs:26`

### Exemplar: Thin Admin Op (keystore_import/list/delete)

Keystore admin ops are the reference for local-IO-backed flows:
- `expand()` validates config and wraps a single shared state.
- States delegate to `local_call()` helper in shared module.
- Evidence: `crates/ops/keystore-admin-op/src/lib.rs:101`

### Exemplar: Mixed Op with Maximal Shared Reuse (portfolio_tracker)

`portfolio_tracker` demonstrates the target pattern for ops with domain-specific output:
- `expand()` wires 4 shared states (`ReadU64HexState` x2 for chain_id/block_number, `NativeBalanceState` for ETH balance, `TokenBalanceState` xN for ERC-20 balances) into a sequential chain.
- Only `WriteSnapshotState` remains op-local because it performs domain-specific aggregation (assembling all balances into a snapshot artifact).
- Evidence: `crates/ops/portfolio-tracker-op/src/lib.rs:265`
- This is the target pattern: maximize shared state reuse, keep only genuinely non-reusable aggregation logic op-local.

## 11. Test Opportunities Enabled by Reusability

The three-tier extraction and utility consolidation unlock test categories that are currently impossible, fragmented, or impractical. This section catalogs what becomes testable and estimates the scope.

### 11.1 Current Test Landscape

| Test Category | Files | Approx. Tests | Coverage |
|---|---:|---:|---|
| CLI e2e / output tests | 8 | ~67 | Good (`cli_tests`, `json_output_integration`, `keystore_integration`, etc.) |
| Shared state unit tests (`evm.rs`) | 1 | 7 | Covers EVM read states; `NativeBalanceState`/`TokenBalanceState` not yet unit-tested |
| Shared meta tests (`meta.rs`) | 1 | 9 | Strong (all factory functions covered) |
| Common module tests (ctx, idempotency, keystore_tx, local_io, rpc) | 5 | ~21 | Moderate |
| Op-level unit tests (scattered `#[cfg(test)]`) | 7 | ~44 | evm-write-op has 14, proof-op has 5, nix-app-op has 14, portfolio-tracker-op has 5 |
| State execute tests (non-EVM) | 0 | 0 | **Gap**: keystore_admin, all op-local states have no isolated execute tests |
| Cross-op composition tests | 0 | 0 | **Gap**: impossible while most states are op-local |

Total test functions: ~148 (81 in ops crates, 67 in CLI tests).

Key test infrastructure already in place:
- `test_support.rs` provides `MemEventStore`, `MemArtifactStore`, `MapContext`, pipeline launch/resume helpers.
- `FixedIo` (in `evm.rs:660` tests) provides a HashMap-based IO double for mocking JSON-RPC responses.
- `NoopRecorder` duplicated in 4 test modules (proof-op, nix-app-op, evm-read-op, evm.rs).
- `CountingTransport` duplicated in 2 test modules (proof-op, evm-read-op).

### 11.2 New Tests Enabled: Utility Unit Tests (est. 60-80 new tests)

Once utilities are consolidated into single-source modules, each function gets one comprehensive test suite instead of fragmented coverage.

#### `hex.rs` tests (est. 15-20)

- `normalize_hex_str`: valid with/without 0x prefix, odd-length padding, empty string, non-hex chars, uppercase input.
- `hex_to_bytes`: round-trip with `bytes_to_hex_prefixed`, empty input, odd-length input, large input.
- `bytes_to_hex_prefixed`: empty bytes, single byte, 32-byte word.
- `hex_nibble`: all 16 valid chars, invalid char.

#### `rlp.rs` tests (est. 20-25)

- `trim_leading_zero_bytes`: empty, all-zero, no leading zeros, single zero byte.
- `u64_to_min_be` / `u128_to_min_be` / `usize_to_min_be`: zero, 1, max value, boundary values (255, 256, 65535, 65536).
- `rlp_encode_bytes`: single byte <0x80, single byte >=0x80, 2-55 bytes, >55 bytes, empty.
- `rlp_encode_list`: empty list, single item, multiple items, nested encoding.
- **Critical**: test that `usize_to_min_be(0)` has a single canonical behavior (resolves the current divergence).

#### `abi.rs` tests (est. 15-20)

- `parse_abi`: minimal ABI, constructor-only, functions + events, malformed JSON.
- `function_selector`: known selectors (e.g. `transfer(address,uint256)` = `0xa9059cbb`).
- `encode_params`: address, uint256, bool, bytes (dynamic), mixed static+dynamic.
- `parse_value_wei_to_hex`: decimal input, hex input, zero, large values, invalid input.

#### `evm_encoding.rs` tests (est. 10-15)

- `encode_u64_word` / `encode_len_word`: zero, max u64, boundary values.
- `parse_address_hex`: valid 20-byte, too short, too long, missing 0x prefix.
- ERC-20 selector constants: verify against known keccak256 hashes.

### 11.3 New Tests Enabled: Shared State Execute Tests (est. 40-60 new tests)

Extracting op-local states to shared modules makes them independently testable with the existing `FixedIo` + `MapContext` harness.

#### States currently untestable in isolation (locked in op crates)

| State | Tests Enabled by Extraction | Category |
|---|---|---|
| `EvmDeployState` | Deploy payload construction, gas estimation injection, nonce handling, signed tx encoding | Execute unit |
| `EvmReceiptPollState` | Success receipt, reverted receipt, timeout, null receipt retry | Execute unit |
| `EvmMulticallState` | Multi-call encoding, per-call receipt handling, partial failure | Execute unit |
| `EvmAssertionState` | Read assertion pass/fail, event assertion pass/fail, chain ID mismatch | Execute unit |
| `KeystoreTxSignState` | Sign with mock local IO, missing key error, invalid data error | Execute unit |
| `KeystoreTxSendRawState` | Submit via mock RPC, RPC rejection, already-known tx | Execute unit |
| `IdempotentSideEffectState` | First execution, idempotent retry, side effect failure | Execute unit |
| `NamespaceReadState` | Valid namespace read, missing namespace, empty facts | Execute unit |
| `NixExecState` | Successful execution, preflight failure, timeout | Execute unit |

Each state above enables 3-6 test cases covering: happy path, input validation failure, IO error propagation, context read/write verification, and event emission verification.

#### States already testable but gaining new test opportunities

| State | New Tests Enabled | Why |
|---|---|---|
| `NativeBalanceState` | 3-4 execute tests | Newly extracted; currently 0 state-level tests |
| `TokenBalanceState` | 4-6 execute tests | Newly extracted; currently 0 state-level tests; decimals fallback path |
| `KeystoreImportState` | 4-6 execute tests | Currently 0 state-level tests despite being in shared module |
| `KeystoreListState` | 3-4 execute tests | Currently 0 state-level tests |
| `KeystoreDeleteState` | 3-4 execute tests | Currently 0 state-level tests |
| `evm_dcv.rs` helpers | 8-12 unit tests | Currently 0 tests; complex validation logic |

### 11.4 New Tests Enabled: Cross-Op Composition Tests (est. 15-25 new tests)

These tests are **impossible today** because most states are locked in their op crates. Once extracted:

#### Scenario: Reuse across ops

```
Test: "NativeBalanceState used by portfolio_tracker AND a hypothetical airdrop_checker op"
- Construct two different op configs that wire the same NativeBalanceState
- Verify identical IO calls, identical context writes
- Confirms the state is truly op-agnostic
```

#### Scenario: State graph composition

```
Test: "EvmDeployState -> EvmReceiptPollState -> EvmAssertionState pipeline"
- Wire three shared states into a mini-graph
- Feed mock IO responses through the pipeline
- Verify context propagation across state boundaries
- Verify event emission order
```

#### Scenario: Mix-and-match from different domains

```
Test: "NativeBalanceState (from evm) + IdempotentSideEffectState (from generic)"
- Wire a balance check followed by an idempotent side effect
- Verify the states compose without conflict
- Confirms cross-domain composability
```

#### Scenario: Error isolation

```
Test: "State A fails; State B (independent branch) still executes"
- Wire two independent state branches in a graph
- Fail one branch via IO mock
- Verify the other branch completes
```

### 11.5 New Tests Enabled: Shared Test Harness Improvements

The extraction motivates standardizing the test doubles currently scattered across op crates.

| Current (Scattered) | Target (Shared in test_support.rs) | Benefit |
|---|---|---|
| `FixedIo` in `evm.rs` tests only | `FixedIo` in `test_support.rs` | Any state test can use IO mocking |
| `NoopRecorder` in 4 test modules | `NoopRecorder` in `test_support.rs` | Standard event recording in all state tests |
| `CountingTransport` in 2 test modules | `CountingTransport` in `test_support.rs` | Call-count assertions available everywhere |
| `MapContext` in `test_support.rs` + 2 duplicates | Single `MapContext` in `test_support.rs` | Already exists; remove duplicates from `app/src/lib.rs:191` and `sdk/src/unstable.rs:780` |
| No shared mock factory | `IoMockBuilder::new().method("eth_call", json!("0x...")).build()` | Declarative IO mock construction |

### 11.6 Test Scope Summary

| Test Category | Current | After Extraction | Delta |
|---|---:|---:|---:|
| Utility unit tests | ~21 | 80-100 | +60-80 |
| State execute unit tests | ~7 (evm.rs only) | 60-80 | +50-70 |
| Cross-op composition tests | 0 | 15-25 | +15-25 |
| Op-level tests | ~44 | ~44 (unchanged) | 0 |
| CLI tests | ~67 | ~67 (unchanged) | 0 |
| **Total** | **~148** | **~270-320** | **+125-175** |

The op-level and CLI test counts stay flat because those tests already cover their respective layers. The gain is entirely in the lower layers: utility tests catch encoding bugs before they reach states, state tests catch logic bugs before they reach integration, and composition tests validate the wiring contract that ops declare.

### 11.7 Architecture Guard Tests (New Category)

In addition to functional tests, the extraction enables structural tests that enforce the three-tier model in CI:

1. **No ambient IO in states**: grep-based or AST-based check that state `execute()` methods never call `std::fs`, `std::net`, `tokio::fs`, etc. directly. (Prototype exists in `check-op-expand-boundary.sh`; should be extended to cover state handlers.)
2. **No business logic in op expand()**: verify that `expand()` methods contain only config validation + `StateGraph` assembly (no `.await`, no IO calls). (Implemented in `check-op-expand-boundary.sh`; currently passes all 14 ops.)
3. **Utility single-source**: verify that functions in `hex.rs`, `rlp.rs`, `abi.rs` are not duplicated elsewhere (import-only). (Baseline enforcement in `check-utility-single-source.sh`; strict mode available via `MFM_UTILITY_STRICT=1`.)
4. **Shared ratio threshold**: count state implementations in `ops/common/src/states/` vs op-local; fail if ratio drops below threshold. (Implemented in `check-shared-state-ratio.sh` with threshold 45%; needs counting bug fix for accurate measurement.)

## 12. Acceptance Criteria for Three-Tier Completion

1. No business workflow logic in `bin/cli` or `bin/rest-api`.
2. No business execution logic in op `expand()` methods; ops are config validation + state graph wiring only. **MET** (all 14 ops pass boundary check).
3. Shared states in `crates/ops/common/src/states/` are the default composition unit (80%+ of all states). Current: 45%.
4. Op-local states exist only for genuinely domain-specific, non-reusable aggregation/output logic.
5. Every reusable pattern (receipt polling, balance reading, side effect idempotency, artifact writing) exists as a shared state. Balance reading: **MET**. Others: pending.
6. State execution side effects routed through IO abstraction boundaries (no ambient IO).
7. Architecture guard prevents ambient IO regressions in CI. **PARTIALLY MET** (scripts exist but not in CI; counting bug in ratio script).
8. CI parity gates (`check/test/basic/parity`) pass with no behavior regressions.
9. Zero duplicated utility functions; all pure helpers consolidated into shared modules (`hex.rs`, `rlp.rs`, `abi.rs`, `evm_encoding.rs`). Current: 11 functions with 24 copies.
10. Shared test harness (`FixedIo`, `NoopRecorder`, `IoMockBuilder`) available in `test_support.rs` for all state-level tests.
11. Utility unit test suite covers all consolidated functions (est. 70-90 tests).
12. Every shared state has at least one `execute()` unit test using IO mocks. Current: only EVM read states have execute tests; newly extracted `NativeBalanceState` and `TokenBalanceState` need tests.
