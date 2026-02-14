# Three-Tier Thin-Layer Alignment Audit

Date: 2026-02-14
Status: Deep review enforcing three-tier model (bins thin -> ops thin -> states reusable)

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
- **New ops compose existing behavior.** A DeFi op needing balance reads wires the same `EthBalanceState` and `Erc20BalanceState` that `portfolio_tracker` uses.
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
- Shared reusable state implementations in `ops/common/src/states/`: 7
- Op-local state implementations across op crates: 13
- Total state implementations: 20

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
| `keystore_tx_sign` | Mixed | 1 | 0 | 1 (`TxSignState`) | Partially aligned | `keystore-tx-op/src/lib.rs:113` |
| `keystore_tx_send_raw` | Mixed | 1 | 0 | 1 (`TxSendRawState`) | Partially aligned | `keystore-tx-op/src/lib.rs:191` |
| `evm_read` | Thin | 2 | 2 (`ReadU64HexState` x2) | 0 | Aligned | `evm-read-op/src/lib.rs:61` |
| `evm_contract_from_nix` | Mixed | 1 | 0 | 1 (`ContractFromNixState`) | Partially aligned | `evm-write-op/src/lib.rs:1223` |
| `evm_deploy` | Mixed | 1 | 0 | 1 (`DeployState`) | Partially aligned | `evm-write-op/src/lib.rs:1345` |
| `evm_configure` | Mixed | 1 | 0 | 1 (`ConfigureState`) | Partially aligned | `evm-write-op/src/lib.rs:1525` |
| `evm_validate` | Mixed | 1 | 0 | 1 (`ValidateState`) | Partially aligned | `evm-write-op/src/lib.rs:1741` |
| `evm_deploy_configure_validate` | Thin | 3 (composed) | 0 (delegates to child ops) | 0 | Aligned | `evm-deploy-configure-validate-op/src/lib.rs:26` |
| `portfolio_tracker` | Mixed | 5 | 2 (`ReadU64HexState` x2) | 3 | Partially aligned | `portfolio-tracker-op/src/lib.rs:349` |
| `proof` | Mixed | 3 | 0 | 3 | Partially aligned | `proof-op/src/lib.rs:116` |
| `nix_app` | Mixed | 1 | 0 | 1 (`NixAppState`) | Partially aligned | `nix-app-op/src/lib.rs:84` |

Tier 2 summary: 5/14 ops are Thin (using only shared states or pure composition). 9/14 are Mixed (delegate to op-local states that should be extracted).

### 4.3 Tier 3: State Layer (Target: Thick, Reusable, Single-Responsibility)

#### Shared States (in `crates/ops/common/src/states/`)

| State | Module | Responsibility | Used By | Reuse Count |
|---|---|---|---|---:|
| `ReadHexStringState` | `evm.rs` | Execute JSON-RPC call, parse hex string response | evm-read-op, portfolio-tracker-op | 2 |
| `ReadU256HexState` | `evm.rs` | Execute JSON-RPC call, parse U256 hex response | evm-read-op, portfolio-tracker-op | 2 |
| `EthCallState` | `evm.rs` | Execute `eth_call` with configurable response decoding | evm-read-op, portfolio-tracker-op | 2 |
| `ReadU64HexState` | `evm.rs` | Execute JSON-RPC call, parse u64 hex with optional validation | evm-read-op, portfolio-tracker-op | 2+ |
| `KeystoreImportState` | `keystore_admin.rs` | Import key into keystore via local IO | keystore-admin-op | 1 |
| `KeystoreListState` | `keystore_admin.rs` | List keystore keys via local IO | keystore-admin-op | 1 |
| `KeystoreDeleteState` | `keystore_admin.rs` | Delete keystore key via local IO | keystore-admin-op | 1 |

Shared states summary: 7 implementations. EVM read states have cross-op reuse (2+ ops each). Keystore admin states are shared-located but single-op consumers.

#### Op-Local States (candidates for extraction)

| State | Op Crate | Responsibility | Extractable | Target Shared Module |
|---|---|---|---|---|
| `TxSignState` | keystore-tx-op | Sign tx via local IO, emit event | Yes | `states/keystore_tx.rs` |
| `TxSendRawState` | keystore-tx-op | Read file + submit raw tx via RPC | Yes | `states/keystore_tx.rs` |
| `ContractFromNixState` | evm-write-op | Adapt nix output to EVM contract artifact | Yes | `states/evm.rs` (artifact parsing) |
| `DeployState` | evm-write-op | Build deploy payload, sign, submit, poll receipt | Yes (decompose) | `states/evm_write.rs` |
| `ConfigureState` | evm-write-op | Encode function calls, submit txs, collect receipts | Yes (decompose) | `states/evm_write.rs` |
| `ValidateState` | evm-write-op | Verify chain ID, read/event assertions | Yes (decompose) | `states/evm_write.rs` |
| `ReadEthBalanceState` | portfolio-tracker-op | Fetch native ETH balance at pinned block | Yes | `states/evm.rs` |
| `ReadErc20BalanceState` | portfolio-tracker-op | Fetch ERC-20 balance with decimals resolution | Yes | `states/evm.rs` |
| `WriteSnapshotState` | portfolio-tracker-op | Aggregate balances into output artifact | Partially | Keep op-local (domain-specific) |
| `ReadFactsState` | proof-op | Read input facts from namespace | Yes | `states/io.rs` |
| `ApplySideEffectState` | proof-op | Idempotent side effect with event emission | Yes | `states/side_effect.rs` |
| `WriteOutputState` | proof-op | Assemble output artifact | Partially | Keep op-local (domain-specific) |
| `NixAppState` | nix-app-op | Execute nix program with optional preflight | Partially | `states/nix.rs` |

Op-local states summary: 13 implementations. Of these, 8 are clearly extractable to shared modules, 3 are partially extractable (domain-specific but with reusable patterns), and 2 should remain op-local (genuinely domain-specific aggregation/output).

## 5. State Reuse Ratio

| Metric | Count |
|---|---:|
| Total state implementations | 20 |
| Shared (in `ops/common/src/states/`) | 7 |
| Op-local (in individual op crates) | 13 |
| Shared ratio | 35% |
| Target shared ratio | 80%+ |
| States with cross-op reuse today | 4 (EVM read states) |

Current composition: op-local states dominate at 65%. The three-tier model requires inverting this ratio so shared states become the dominant composition unit.

## 6. Findings (Ordered by Severity)

### High

1. `B7` Op-local states dominate composition; ops are not thin enough.
- Impact: 9 of 14 ops use op-local states instead of shared states. Business logic is locked inside individual op crates and cannot be reused across operations.
- Evidence: `crates/ops/evm-write-op/src/lib.rs` contains 4 op-local states (~1400 lines of execute logic). `crates/ops/portfolio-tracker-op/src/lib.rs` contains 3 op-local states. `crates/ops/proof-op/src/lib.rs` contains 3 op-local states.
- Risk: new ops cannot compose existing behavior; duplicated patterns across ops; slower evolution toward composition-first architecture.
- Remediation: extract op-local states to `crates/ops/common/src/states/` per the extraction plan in section 8.

1b. `B8` Utility functions duplicated across crates instead of shared.
- Impact: 11 pure functions are duplicated 2-3 times across `evm-write-op`, `common/states/evm_dcv.rs`, `common/local_io.rs`, and `common/keystore_tx.rs`. Maintenance changes must be applied in multiple places; bugs fixed in one copy persist in others; test coverage is fragmented (some copies tested, others not).
- Evidence: see section 7 (Utility Duplication Audit) for full inventory.
- Risk: correctness drift between copies; duplicated test burden; slower onboarding.
- Remediation: consolidate into shared utility modules per the plan in section 7.1.

### Medium

2. `B6/B7` EVM write states are monolithic and not decomposed.
- Impact: `DeployState`, `ConfigureState`, and `ValidateState` in `evm-write-op` each contain large execute methods with multiple responsibilities (payload construction, signing, receipt polling, validation). These should be decomposed into smaller reusable states.
- Evidence: `crates/ops/evm-write-op/src/lib.rs:1448` (DeployState), `crates/ops/evm-write-op/src/lib.rs:1659` (ConfigureState), `crates/ops/evm-write-op/src/lib.rs:1855` (ValidateState).
- Risk: receipt polling pattern is duplicated between deploy and configure; signing delegation is duplicated; validation patterns cannot be reused by other ops.

3. `B6` Portfolio balance states are extractable EVM primitives.
- Impact: `ReadEthBalanceState` and `ReadErc20BalanceState` implement generic EVM balance-reading patterns that any DeFi-related op would need.
- Evidence: `crates/ops/portfolio-tracker-op/src/lib.rs:476` (ReadEthBalanceState), `crates/ops/portfolio-tracker-op/src/lib.rs` (ReadErc20BalanceState).
- Risk: any future op needing balance queries would duplicate this logic.

4. Guardrails for ambient IO regressions are not yet automated.
- Impact: no architecture test/check currently fails PRs when new state handlers reintroduce direct ambient IO outside intended transport.
- Evidence: missing dedicated check surface; explicit local transport boundary is in `crates/ops/common/src/local_io.rs:47`.
- Risk: regressions can land silently.

5. Parity validation is still pending for this checkpoint.
- Impact: architecture-level changes are landed but not yet validated through full parity gates.
- Evidence: expected gates are `nix run .#check`, `nix run .#test`, `nix run .#ci -- --basic --summary`, `nix run .#ci -- --parity --summary`.
- Risk: unnoticed behavior drift.

### Low

6. `B1` Minor validation logic in CLI layer.
- Impact: `portfolio snapshot` validates JSON array in CLI; `run pipeline deploy-configure-validate` reads spec files in CLI.
- Evidence: `bin/cli/src/commands/portfolio/snapshot.rs`, `bin/cli/src/commands/run/pipeline.rs`.
- Risk: low; these are input validation not business logic, but should ideally be pushed to ops.

7. `B3` Keystore ops packaging naming ambiguity.
- Impact: `mfm-op-keystore` implies executable op behavior but only re-exports core keystore types.
- Evidence: `crates/ops/keystore-op/src/lib.rs:8`.
- Risk: boundary confusion.

8. `B7` Proof op states are generic patterns locked in a domain-specific crate.
- Impact: `ApplySideEffectState` implements a generic idempotent-side-effect-with-event pattern. `ReadFactsState` implements generic namespace-read. Both are locked in proof-op.
- Evidence: `crates/ops/proof-op/src/lib.rs:132`.
- Risk: other ops needing idempotent side effects would duplicate the pattern.

## 7. Utility Duplication Audit (`B8`)

The three-tier model requires that pure utility functions live in shared modules, not scattered across op crates and IO layers. This section catalogs every duplicated function discovered in the current codebase.

### 7.1 Duplication Inventory

#### Hex Encoding/Decoding (3 copies each)

| Function | Location 1 | Location 2 | Location 3 |
|---|---|---|---|
| `normalize_hex_str()` | `evm-write-op/src/lib.rs:354` | `common/states/evm_dcv.rs:158` (pub) | `common/local_io.rs:894` (different error type) |
| `hex_to_bytes()` | `evm-write-op/src/lib.rs:372` | `common/states/evm_dcv.rs:176` (pub) | `common/local_io.rs:909` (different error type) |
| `bytes_to_hex_prefixed()` | `evm-write-op/src/lib.rs:381` | `common/states/evm_dcv.rs:185` (pub) | `common/local_io.rs:905` |

These are the most pervasive duplications. The three copies differ only in error return types (`String` vs `()`).

#### RLP Encoding (2 copies each)

| Function | Location 1 (`keystore_tx.rs`) | Location 2 (`local_io.rs`) | Variation |
|---|---|---|---|
| `trim_leading_zero_bytes()` | `keystore_tx.rs:486` | `local_io.rs:919` | Identical |
| `u128_to_min_be()` | `keystore_tx.rs:508` | `local_io.rs:927` | Identical |
| `usize_to_min_be()` | `keystore_tx.rs:561` | `local_io.rs:980` | Different: 0 returns `vec![]` vs `vec![0]` |
| `rlp_encode_bytes()` | `keystore_tx.rs:522` | `local_io.rs:941` | Identical |
| `rlp_encode_list()` | `keystore_tx.rs:541` | `local_io.rs:960` | `local_io` calls `rlp_encode_bytes()` on each item |

These are critical for EIP-1559 transaction encoding and are entirely untested in one copy.

#### ABI Parsing/Encoding (2 copies each)

| Function | Location 1 (`evm-write-op`) | Location 2 (`evm_dcv.rs`) | Notes |
|---|---|---|---|
| `parse_abi()` | `evm-write-op/src/lib.rs:316` | `common/states/evm_dcv.rs:118` (pub) | Identical logic, extracts constructor/functions/events |
| `encode_params()` | `evm-write-op/src/lib.rs:563` | `common/states/evm_dcv.rs:249` | evm-write-op version is more complete (handles dynamic types) |
| `parse_value_wei_to_hex()` | `evm-write-op/src/lib.rs:695` | `common/states/evm_dcv.rs:388` (pub) | Identical |

#### Signer Helpers

| Function | Location 1 | Location 2 | Notes |
|---|---|---|---|
| `signer_address_hex()` | `common/local_io.rs:799` | `evm-write-op/src/lib.rs:1067` (`#[cfg(test)]`) | Test-only in evm-write-op |

### 7.2 Semantic Duplication (Same Pattern, Different Implementations)

Beyond copy-pasted functions, several patterns are reimplemented with slight variations:

| Pattern | Instances | Evidence |
|---|---|---|
| 32-byte word encoding (address padding) | `encode_address_word()` in evm_dcv.rs:198; `parse_address_hex()` + manual padding in evm-write-op:401 | Same intent, different APIs |
| Function selector computation | `function_selector()` in evm_dcv.rs:277 (keccak256 first 4 bytes); `selector()` in evm-write-op:597 | Same algorithm, different names |
| `u64_to_min_be()` | `keystore_tx.rs:494` | Only one copy, but should be co-located with `u128_to_min_be()` |
| ERC-20 selectors (hardcoded bytes) | `erc20_selector_balance_of()` in evm.rs:421; manual `[0x70,0xa0,0x82,0x31]` in portfolio-tracker-op | Constant duplication risk |

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

### Phase 1: EVM Write States (Highest impact)

Extract from `crates/ops/evm-write-op/src/lib.rs` into `crates/ops/common/src/states/evm_write.rs`:

| Current Op-Local State | Target Shared State(s) | Decomposition |
|---|---|---|
| `ContractFromNixState` | `NixArtifactToEvmContractState` | Move as-is; artifact parsing is reusable |
| `DeployState` | `EvmDeployState` + `EvmReceiptPollState` | Decompose: payload build + sign + submit is one state; receipt polling is a reusable state |
| `ConfigureState` | `EvmMulticallState` + reuse `EvmReceiptPollState` | Decompose: function encoding + submission per call; reuse receipt polling |
| `ValidateState` | `EvmAssertionState` | Move as-is or decompose into read-assertion + event-assertion |

Exit criteria: `evm_deploy`, `evm_configure`, `evm_validate` ops become thin wrappers that only validate config and wire shared states in `expand()`.

### Phase 2: Portfolio Balance States

Extract from `crates/ops/portfolio-tracker-op/src/lib.rs` into `crates/ops/common/src/states/evm.rs`:

| Current Op-Local State | Target Shared State | Notes |
|---|---|---|
| `ReadEthBalanceState` | `EthBalanceState` | Generic "read native balance at block" |
| `ReadErc20BalanceState` | `Erc20BalanceState` | Generic "read ERC-20 balance with decimals" |
| `WriteSnapshotState` | Keep op-local | Domain-specific aggregation; but extract artifact-write pattern to shared helper |

Exit criteria: `portfolio_tracker` op wires shared EVM balance states and only keeps domain-specific aggregation logic.

### Phase 3: Keystore TX States

Extract from `crates/ops/keystore-tx-op/src/lib.rs` into `crates/ops/common/src/states/keystore_tx.rs`:

| Current Op-Local State | Target Shared State | Notes |
|---|---|---|
| `TxSignState` | `KeystoreTxSignState` | Already delegates to `local_call()`; move to shared |
| `TxSendRawState` | `KeystoreTxSendRawState` | Already delegates to shared `send_raw_transaction_via_io()`; move to shared |

Exit criteria: `keystore_tx_sign` and `keystore_tx_send_raw` ops become as thin as the keystore-admin ops.

### Phase 4: Generic Patterns

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

| Metric | Current | After Phase 1-4 + Utility Consolidation |
|---|---:|---:|
| Shared state implementations | 7 | 18+ |
| Op-local state implementations | 13 | 3-4 |
| Shared ratio | 35% | 82%+ |
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
- Evidence: `crates/ops/evm-read-op/src/lib.rs:80`

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

## 11. Test Opportunities Enabled by Reusability

The three-tier extraction and utility consolidation unlock test categories that are currently impossible, fragmented, or impractical. This section catalogs what becomes testable and estimates the scope.

### 11.1 Current Test Landscape

| Test Category | Files | Approx. Tests | Coverage |
|---|---:|---:|---|
| Integration tests (full pipelines) | 7 | ~40 scenarios | Thorough for happy paths |
| CLI e2e / output tests | 8 | ~100 invocations | Good |
| Shared state unit tests (`evm.rs`) | 1 | ~30 | Strong for EVM read states |
| Op-level unit tests (scattered `#[cfg(test)]`) | 5 | ~30 | Sparse, per-op only |
| Utility function unit tests | 2 | ~10 | Fragmented; many functions untested |
| State execute tests (non-EVM) | 0 | 0 | **Gap**: evm_dcv, keystore_admin, all op-local states |
| Cross-op composition tests | 0 | 0 | **Gap**: impossible while states are op-local |

Key test infrastructure already in place:
- `test_support.rs` provides `MemEventStore`, `MemArtifactStore`, `MapContext`, pipeline launch/resume helpers.
- `FixedIo` (in `evm.rs` tests) provides a HashMap-based IO double for mocking JSON-RPC responses.
- `CountingTransport` / `MockEvmTransport` / `EchoTransport` exist per-op but are not shared.

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
| `EthBalanceState` | Valid balance, zero balance, block pinning, RPC error | Execute unit |
| `Erc20BalanceState` | Balance with decimals, decimals call failure fallback, zero balance | Execute unit |
| `KeystoreTxSignState` | Sign with mock local IO, missing key error, invalid data error | Execute unit |
| `KeystoreTxSendRawState` | Submit via mock RPC, RPC rejection, already-known tx | Execute unit |
| `IdempotentSideEffectState` | First execution, idempotent retry, side effect failure | Execute unit |
| `NamespaceReadState` | Valid namespace read, missing namespace, empty facts | Execute unit |
| `NixExecState` | Successful execution, preflight failure, timeout | Execute unit |

Each state above enables 3-6 test cases covering: happy path, input validation failure, IO error propagation, context read/write verification, and event emission verification.

#### States already testable but gaining new test opportunities

| State | New Tests Enabled | Why |
|---|---|---|
| `KeystoreImportState` | 4-6 execute tests | Currently 0 state-level tests despite being in shared module |
| `KeystoreListState` | 3-4 execute tests | Currently 0 state-level tests |
| `KeystoreDeleteState` | 3-4 execute tests | Currently 0 state-level tests |
| `evm_dcv.rs` states | 8-12 execute tests | Currently 0 tests; complex validation logic |

### 11.4 New Tests Enabled: Cross-Op Composition Tests (est. 15-25 new tests)

These tests are **impossible today** because states are locked in their op crates. Once extracted:

#### Scenario: Reuse across ops

```
Test: "EthBalanceState used by portfolio_tracker AND a hypothetical airdrop_checker op"
- Construct two different op configs that wire the same EthBalanceState
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
Test: "EthBalanceState (from evm) + IdempotentSideEffectState (from generic)"
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
| `NoopRecorder` in `evm.rs` tests | `NoopRecorder` in `test_support.rs` | Standard event recording in all state tests |
| `CountingTransport` in evm-read-op | `CountingTransport` in `test_support.rs` | Call-count assertions available everywhere |
| `MapContext` duplicated in evm.rs | Single `MapContext` in `test_support.rs` | Already exists but duplicate should be removed |
| No shared mock factory | `IoMockBuilder::new().method("eth_call", json!("0x...")).build()` | Declarative IO mock construction |

### 11.6 Test Scope Summary

| Test Category | Current | After Extraction | Delta |
|---|---:|---:|---:|
| Utility unit tests | ~10 | 70-90 | +60-80 |
| State execute unit tests | ~30 (evm.rs only) | 80-100 | +50-70 |
| Cross-op composition tests | 0 | 15-25 | +15-25 |
| Integration tests | ~40 | ~40 (unchanged) | 0 |
| **Total** | **~80** | **~205-255** | **+125-175** |

The integration test count stays flat because those tests already cover end-to-end behavior. The gain is entirely in the lower layers: utility tests catch encoding bugs before they reach states, state tests catch logic bugs before they reach integration, and composition tests validate the wiring contract that ops declare.

### 11.7 Architecture Guard Tests (New Category)

In addition to functional tests, the extraction enables structural tests that enforce the three-tier model in CI:

1. **No ambient IO in states**: grep-based or AST-based check that state `execute()` methods never call `std::fs`, `std::net`, `tokio::fs`, etc. directly.
2. **No business logic in op expand()**: verify that `expand()` methods contain only config validation + `StateGraph` assembly (no `.await`, no IO calls).
3. **Utility single-source**: verify that functions in `hex.rs`, `rlp.rs`, `abi.rs` are not duplicated elsewhere (import-only).
4. **Shared ratio threshold**: count state implementations in `ops/common/src/states/` vs op-local; fail if ratio drops below 80%.

## 12. Acceptance Criteria for Three-Tier Completion

1. No business workflow logic in `bin/cli` or `bin/rest-api`.
2. No business execution logic in op `expand()` methods; ops are config validation + state graph wiring only.
3. Shared states in `crates/ops/common/src/states/` are the default composition unit (80%+ of all states).
4. Op-local states exist only for genuinely domain-specific, non-reusable aggregation/output logic.
5. Every reusable pattern (receipt polling, balance reading, side effect idempotency, artifact writing) exists as a shared state.
6. State execution side effects routed through IO abstraction boundaries (no ambient IO).
7. Architecture guard prevents ambient IO regressions in CI.
8. CI parity gates (`check/test/basic/parity`) pass with no behavior regressions.
9. Zero duplicated utility functions; all pure helpers consolidated into shared modules (`hex.rs`, `rlp.rs`, `abi.rs`, `evm_encoding.rs`).
10. Shared test harness (`FixedIo`, `NoopRecorder`, `IoMockBuilder`) available in `test_support.rs` for all state-level tests.
11. Utility unit test suite covers all consolidated functions (est. 70-90 tests).
12. Every shared state has at least one `execute()` unit test using IO mocks.
