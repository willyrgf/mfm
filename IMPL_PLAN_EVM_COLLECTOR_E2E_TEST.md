# Implementation plan: CLI-driven CollectEvmBalances e2e via reth + PostgreSQL

Goal: one user-level end-to-end verification of EVM native-balance collection driven
THROUGH THE CLI BINARY, exactly as a user operates it: a config file, `mfm_cli init`,
`mfm_cli snapshot --run-id`, durable history in the managed PostgreSQL store, and a
second independent invocation (`mfm_cli show`) that reads the identical run back. The
CLI today has zero commands; this milestone builds the minimal trusted live
composition. Designed by one dedicated architect (2026-08-19) on top of the probe
facts and the three-architect library decision below.

Deferred to a later wave (user decision): token/contract deployment, configuration
effects, keystore import, and the failure/boundary/capacity e2e tests. The mocked
`crates/app/tests/portfolio_runtime.rs` loses ONLY its native hot/cold test this wave
(the CLI e2e replaces it); the rest of the file survives until the later wave covers
it (its disposition table from the previous revision applies then).

Complexity rule: minimize layers, indirection, and owned APIs. Dependency count is not
the metric.

## Verified facts

Probe session 2026-08-19 against reth 1.9.3 from the pinned nixpkgs
(`github:NixOS/nixpkgs/421eebfd0ec7bccd4abe826ce62d7e6e83129493#reth`):

1. Chain id `1337`. The 20 canonical dev accounts exist; account 1
   (`0x70997970c51812dc3a010c7d01b50e0d17dc79c8`) holds exactly `10^24` wei and
   nothing in this milestone ever spends from it.
2. `eth_call` to a codeless address returns result `"0x"` (success, empty); a revert
   surfaces as a JSON-RPC error object (code 3). Instamine: the chain stays at its
   head until a transaction arrives. `eth_getBlockByNumber` returns `{number, hash}`
   as `0x`-prefixed lowercase hex.
3. Nixfied (pinned framework): service ids `reth` (endpoints reth-http/ws/authrpc,
   http primary) and `postgres`. Task env placeholders use the SERVICE id —
   `${host:reth}`/`${port:reth}` — endpoint-id placeholders are rejected at admission.
   Service state persists across task runs on a slot (`reth` datadir AND the postgres
   database; only `clean` wipes). `cargoLeaf` (`nixfied.nix:38`) takes only
   `run`/`env`; `requires` attaches via the `// { requires = [ ... ]; }` splice
   (`postgres-test` pattern, `nixfied.nix:104-111`). Tasks run plain `cargo test`.
4. `sha256(docs/contracts/evm-portfolio/portfolio-snapshot.json trimmed)` =
   `79fa722c25a7f5ea67f6aab45eaa4918d9dff3607aae418dc2092d959a1e9e2b` (verified with
   sha256sum — the commit-7 digest pin).
5. For the deferred wave only (probe-validated, do not lose): DevSigner
   `eth_sendTransaction` works only fully-specified and only for calls/transfers —
   contract creation is rejected, so deployment needs in-test signing
   (`alloy-consensus` + `k256`, `eth_sendRawTransaction`). The validated 80-byte
   probe token (init returns runtime; `decimals()`=18, `balanceOf(any)`=10^18, else
   revert):
   `0x6044600c60003960446000f36004361060215760003560e01c8063313ce56714602757806370a08231146032575b60006000fd5b601260005260206000f35b670de0b6b3a764000060005260206000f3`

## EVM library decision record (three-architect panel, 2026-08-18)

Unanimous: alloy building blocks + mfm-owned JSON-RPC envelope; never
`alloy-provider`/`alloy-network`/`alloy-rpc-types`/`alloy-signer`. Custody is
option-invariant (32-byte digest in, 65 signature bytes out; alloy assembles external
signatures first-class and never holds keys). The 3-year surface is ~15 frozen-wire
RPC methods plus tx construction — exactly alloy-consensus/primitives. Every
silent-wrong-money codec (U256/hex, ABI, RLP, keccak) lives in alloy's stable
pure-Rust core; the provider half churns biweekly and its default TLS backend would
drag cmake into the nix build. This milestone executes the recorded production
trajectory: the JSON-RPC provider lands in `crates/live/evm` as production code.
`mfm-evm-live` becomes the ONLY crate allowed `alloy-*` (record the rule in its
README; the `cargo metadata` CI ban lands when trivial). Keystore future shape
(recorded for the keystore wave): `Keystore::sign_digest` inside `mfm-keystore`,
dedicated-thread owner behind a bounded handle implementing `mfm_signing::Signer`.

---

# Target design

## CLI surface (`bin/cli`, binary `mfm_cli`, one file `src/main.rs`, ~250 lines)

Flat clap subcommands — no session state, no output DTOs, no `anyhow`:

```
mfm_cli init     --config <PATH>
mfm_cli snapshot --config <PATH> --run-id <RUN_ID>
mfm_cli show     --config <PATH> --run-id <RUN_ID>
```

- `--run-id` is the full explicit `run:sha256-jcs-v1:<64 hex>` string via
  `RunId::parse`. The CLI never derives or defaults a RunId (design invariant).
- One EVM route per config file this wave: the CLI builds exactly one
  `EvmPhysicalTarget` from the `evm` block, so a config whose sources span two chain
  ids fails at planning as an invalid request. State this limitation in
  `bin/cli/README.md`.
- `show` requires the fully adapter-bound assembly (hence `rpc_url_env` even for
  reads) — settled, not an uncertainty: `docs/architecture.md` states association
  pre-resolves every implementation and callback, and the cold-read binding-failure
  runtime contract test proves cold reads resolve bindings.
- `init`: resolve `database_url_env` → `mfm_storage_postgres::install_schema`. Silent
  on success.
- `snapshot`: full composition → `Application::start_portfolio(run_id, selector,
  &config, &[target])` → render.
- `show`: identical composition → `Application::read(&run_id)` → identical render.
  No `resume` command this wave.

stdout contract (one shared render fn — byte-identical output for the same retained
run is exactly what the e2e diffs):

```
run_id=run:sha256-jcs-v1:<64 hex>
head_sequence=<u64>
head_digest=content:sha256-v1:<64 hex>
state=runnable|succeeded|failed
<RetainedValueView::canonical_bytes verbatim, one line, only for succeeded/failed>
```

Exit codes: `0` = view rendered with `state=succeeded` (and `init` success); `1` =
view rendered with any other state; `2` = no view (usage/config/env/store/provider/
Application error). stderr: one reviewed fixed line per failure class
(`error: configuration is invalid`, `error: environment variable <NAME> is not set`,
`error: postgres store is unavailable|incompatible`, `error: run id is invalid`,
`error: evm provider transport could not be constructed`,
`error: runtime operation failed: <RuntimeError Display>`). Env var NAMES may appear;
URL VALUES never appear anywhere. `bin/cli/README.md` is rewritten as the one current
redacted transport contract (commands, config schema, stdout lines, exit table,
redaction rules).

## Configuration contract (one JSON file; URLs env-indirect ONLY)

```json
{
  "portfolio": { "portfolio_id": "portfolio-example", "quotes": ["usd"],
    "collections": [{ "correlation": "native-collection",
      "request": { "sources": [{ "source_id": "wallet.native", "chain_id": 1337,
        "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8", "token": null }],
        "decimals": 18 } }] },
  "selector": { "target": "portfolio-example", "quote": "usd" },
  "evm": { "chain_id": 1337, "endpoint_id": "reth-dev", "rpc_url_env": "MFM_E2E_RPC_URL" },
  "store": { "database_url_env": "MFM_E2E_DATABASE_URL" }
}
```

Parsing lives in `bin/cli` (private structs). `portfolio`/`selector` deserialize
directly into `mfm_portfolio::PortfolioConfig` / `PortfolioSnapshotSelector` — their
checked `Deserialize` impls do all domain validation. `deny_unknown_fields` at every
level:

```rust
#[derive(serde::Deserialize)] #[serde(deny_unknown_fields)]
struct CliConfig { portfolio: PortfolioConfig, selector: PortfolioSnapshotSelector,
                   evm: EvmRouteConfig, store: StoreConfig }
#[derive(serde::Deserialize)] #[serde(deny_unknown_fields)]
struct EvmRouteConfig { chain_id: u64, endpoint_id: String, rpc_url_env: String }
#[derive(serde::Deserialize)] #[serde(deny_unknown_fields)]
struct StoreConfig { database_url_env: String }
```

## Domain surface extension (`crates/domains/evm/src/lib.rs`)

Two additions, one commit — both exist so live providers consume the domain's own
typed contract instead of re-declaring it:

**Typed intent access.** `EvmReadSubject` and `EvmBlockAnchor` become `pub` (a public
enum cannot carry private types), `EvmReadIntent` gains
`pub const fn subject(&self) -> &EvmReadSubject`, and `EvmBlockAnchor` gains
`pub fn number(&self) -> &str` / `pub fn hash(&self) -> &str` (fields stay private;
checked construction unchanged). The provider then deserializes the request bytes with
the EXISTING checked `EvmReadIntent` deserializer and matches typed — no serde mirror
in `mfm-evm-live`, no silent drift class. The wire was already public in practice: the
adapter serializes the whole intent as every provider's request bytes.

## New domain value: `EvmEndpoint` (same commit)

Secret-free named endpoint identity, mirroring the `EvmPhysicalTarget` pattern
(`Serialize + MfmValue + impl_checked_deserialize!`,
`#[mfm(namespace = "mfm.evm", name = "endpoint", version = "1", schema = "mfm.evm-endpoint")]`):

```rust
pub struct EvmEndpoint { endpoint_id: String }
impl EvmEndpoint {
    pub fn new(endpoint_id: impl Into<String>) -> Result<Self, EvmDomainError>; // valid_public_text(_, 256)
    pub fn endpoint_ref(&self) -> Result<ContentRef, EvmDomainError>;           // canonicalize_mfm_value
}
```

CLI builds `EvmPhysicalTarget::new(evm.chain_id, EvmEndpoint::new(&evm.endpoint_id)?.endpoint_ref()?)`.
Same `endpoint_id` ⇒ same route ref ⇒ same Program across invocations; changing the
rpc_url env VALUE never changes identity (the documented credential/handle rule).

## Production provider: `JsonRpcEvmProvider` (`crates/live/evm`)

```rust
pub struct JsonRpcEvmProvider { /* url: String, http: reqwest::Client (10 s timeout) */ }
#[derive(Debug, thiserror::Error)]
#[error("evm provider transport could not be constructed")]
pub struct EvmProviderBuildError;
impl JsonRpcEvmProvider { pub fn new(rpc_url: String) -> Result<Self, EvmProviderBuildError>; }
impl EvmProvider for JsonRpcEvmProvider { /* the fixed trait signature at src/lib.rs:34 */ }
```

Everything else private: a ~40-line JSON-RPC 2.0 envelope
(`{"jsonrpc":"2.0","id":1,method,params}` → `result`/`error`) and conversions on
`alloy_primitives`: `quantity_to_decimal` ("0x…" → exact decimal via `U256`),
`block_tag` (decimal → `{:#x}`), `word_to_u8` (exactly 66 chars, ≤255). Addresses are
parsed with `alloy_primitives::Address::from_str` and re-rendered — never spliced by
string concatenation (`EvmBalanceSource` validates the address only as lowercase
public text, not as 20 hex bytes; a malformed address must fail locally as `Internal`
BEFORE any IO, not as a mis-classed node error). Request prologue: deserialize the
bytes with the checked `serde_json::from_slice::<EvmReadIntent>` (failure →
`Err(Internal)`); cross-check `operation_and_chain_id().0 == operation.as_str()`
(mismatch → `Internal`); then match `intent.subject()`:

| `EvmReadSubject` | RPC call | `EvmReadValue` |
|---|---|---|
| `ChainIdentity` | `eth_chainId []` | `ChainId(u64)` |
| `InitialAnchor` | `eth_getBlockByNumber ["latest", false]` | `Anchor { number: decimal, hash }` |
| `NativeBalance` | `eth_getBalance [address, block_tag(anchor.number)]` | `RawUnits(decimal)` |
| `TokenDecimals` | `eth_call [{to: token, data: "0x313ce567"}, block_tag]` | `TokenDecimals(u8)` |
| `TokenBalance` | `eth_call [{to: token, data: "0x70a08231" + 12 zero bytes (24 hex chars) + 20-byte address — 36-byte calldata}, block_tag]` | `RawUnits(decimal)` |
| `ConfirmAnchor` | `eth_getBlockByNumber [block_tag(anchor.number), false]` — BY NUMBER, never latest | `Anchor { number: decimal, hash }` |

All six implemented now (token paths stay covered by the surviving mocked tests).
`ConfirmAnchor` must reproduce the initial anchor byte-identically
(`interpret_confirm_balance_anchor`, `crates/domains/evm/src/lib.rs:1747`); all
numbers/units canonical decimal (`read_value_valid`, `:2103`).

Failure map (per `docs/evm-rpc-routing.md:18-19`; NEVER `IntegrityBlocked`):

| condition | result |
|---|---|
| transport/timeout | `Err(ReadAdapterError::Unavailable)` |
| JSON-RPC error object (revert) | `Ok(EvmProviderResponse::SafeFailure)` |
| `eth_call` result exactly `"0x"` | `Ok(EvmProviderResponse::SafeFailure)` |
| any other decode failure (null block, malformed hex, oversized word) | `Err(Unavailable)` |
| response body > 512 KiB | `Err(Unavailable)` |
| intent decode / operation mismatch | `Err(ReadAdapterError::Internal)` |

Deps — workspace `[workspace.dependencies]`:
`reqwest = { version = "0.12", default-features = false, features = ["json"] }`,
`alloy-primitives = { version = "1.6.1", default-features = false, features = ["std"] }`;
`crates/live/evm/Cargo.toml` adds `reqwest`, `alloy-primitives`, `serde`, `thiserror`
(workspace). Gate: `cargo tree -e features -p mfm-evm-live` → no TLS, no cmake, no
`alloy-provider/network/rpc-types/signer`.

In-crate unit tests: conversion/calldata pure tests; intent-mismatch → `Internal`;
a std `TcpListener` canned-HTTP stub (no new deps) for happy `eth_chainId`,
revert → `SafeFailure`, `"0x"` → `SafeFailure`, malformed hex → `Unavailable`,
unbound port → `Unavailable`.

## Shared composition (`crates/app/src/lib.rs` — the `layer = "assembly"` crate)

The 13-registration list moves out of the mocked test into:

```rust
pub fn register_portfolio_states(builder: &mut RuntimeAssemblyBuilder) -> mfm_runtime::Result<()>
pub fn portfolio_assembly(target: EvmPhysicalTarget, provider: Arc<dyn EvmProvider>)
    -> mfm_runtime::Result<RuntimeAssembly>   // states + register_evm_reads + finish
```

`mfm-evm-live` moves from dev-dependency to dependency of `mfm-app`.
`portfolio_runtime.rs` drops its local `state_builder()`/`assembly()` and calls these
(the missing-adapter negative test uses `register_portfolio_states` + `finish`; give
that fn a rustdoc line saying standalone use exists for adapterless composition, so it
does not read as a general API). One composition site for CLI and tests.
`docs/architecture.md` Application row gains "trusted Portfolio assembly composition".
Recorded cost, not a defect: every `mfm-app` consumer now links reqwest and
alloy-primitives transitively (`bin/cli` needs both anyway; `bin/rest-api` has zero
dependencies today and is unaffected). Avoiding it would need a new crate split —
rejected under the complexity rule.

CLI composition line:

```rust
let store = PostgresStore::connect(&database_url).await?;
let provider = Arc::new(JsonRpcEvmProvider::new(rpc_url)?);
let app = Application::new(Runtime::new(portfolio_assembly(target, provider)?, Arc::new(store)));
```

## Postgres provisioning (`crates/storages/postgres`)

One public entry, owned by the storage crate (the migration file, schema contract, and
admission gate already live there; Store-trait ownership rules constrain trait
semantics, not physical self-provisioning):

```rust
/// Installs the fresh run-history schema when absent; verifies an existing installation.
/// Never modifies an incompatible existing installation.
pub async fn install_schema(database_url: &str) -> Result<(), StoreOpenError>
```

Behavior: verify Ok → Ok (idempotent); incompatible + zero `mfm_`-prefixed relations →
execute `MIGRATION_SQL` (the `include_str!` loses `#[cfg(test)]`) → re-verify;
incompatible with any `mfm_` relation → `Incompatible` (never touch); IO failure →
`Unavailable`. New ignored test in `src/tests.rs` under the existing `postgres-test`
task: fresh → install twice Ok → connect Ok; hostile leftover → `Incompatible`.

## The e2e test (`bin/cli/tests/cli_e2e.rs`)

One `#[tokio::test]`,
`#[ignore = "requires the managed postgres and reth services provided by the cli-e2e task"]`,
hard `.expect` on `MFM_E2E_RPC_URL` and `MFM_E2E_DATABASE_URL`. Child processes via
`env!("CARGO_BIN_EXE_mfm_cli")` + `std::process::Command` (no assert_cmd). `sqlx`
dev-dep only for the schema reset (`DROP SCHEMA IF EXISTS public CASCADE; CREATE
SCHEMA public` — mirrors `tests.rs:61-69`; required because `postgres-test` leaves
hostile schemas in the same database and service state persists across runs; the reset
also makes `init` exercise the real install path every run). `bin/cli/Cargo.toml`
gains the `[[test]] name = "cli_e2e"` entry (`autotests = false`).

Scenario `native_snapshot_run_and_read_back_via_cli`:

1. Read env; reset `public` schema via one sqlx connection.
2. Write the config JSON above into a uniquely named file under `std::env::temp_dir()`.
3. Fresh RunId per execution (durable store + warm chain):
   `format!("run:sha256-jcs-v1:{:064x}", u128-from-SystemTime-nanos ^ pid)`.
4. `init` → exit 0, empty stdout; `init` again → exit 0 (idempotence).
5. Invocation 1: `snapshot` → exit 0; stderr empty; stdout: `run_id=<id>`,
   `head_sequence=11` (same Program shape as the deleted mock), `head_digest=` prefix,
   `state=succeeded`; final line contains `"raw_units":"1000000000000000000000000"`,
   `"decimals":18`, `"kind":"native"`, `"chain_id":1337`,
   `"total_value_dec":"1000000"` (pin exact strings on first green run). Capture full
   stdout.
6. `snapshot` again, same run id → exit 0, stdout byte-identical (durable idempotent
   re-admission across processes).
7. Invocation 2: `show` → exit 0, stdout byte-identical to invocation 1 (one
   assertion covers digest, sequence, state, and canonical output parity across
   independent CLI processes over the durable store).

## Nixfied + docs

`nixfied.nix`: (1) line 61 → `imports = [ adapters.postgres adapters.reth ];`
(2) new leaf next to `postgres-test`:

```nix
    cli-e2e = (cargoLeaf {
      run = [ "cargo" "test" "-p" "mfm" "--test" "cli_e2e" "--" "--include-ignored" "--test-threads=1" ];
      env = {
        MFM_E2E_RPC_URL = "http://\${host:reth}:\${port:reth}";
        MFM_E2E_DATABASE_URL = "postgresql://postgres@\${host:postgres}:\${port:postgres}/postgres";
      };
    }) // {
      requires = [ "postgres" "reth" ];
    };
```

(3) `ci` steps: insert `"cli-e2e"` after `"test-db"` (if `model-check` rejects a
`requires`-bearing leaf directly in the seq, wrap it in a single-step composite
exactly like `test-db`). (4) No `capacity-app` change this wave; no `placement.ports`
change (4 endpoints < window 11). Serialize broad gates as always (reth's unmodeled
p2p listener binds its default port).

`docs/build-and-verification.md`: focused-matrix line 43 becomes
`cargo test -p mfm --all-targets` (keep `cargo check -p mfm-rest-api`); add the manual
loop line (`MFM_E2E_RPC_URL=... MFM_E2E_DATABASE_URL=... nix develop -c cargo test -p
mfm --test cli_e2e -- --include-ignored --test-threads=1`, annotated: serial-only,
locally started pinned `reth --dev` + postgres); Nixfied table row between lines 55-56:
`| nix run .#run -- --task cli-e2e | Run ignored CLI-driven EVM snapshot e2e against
the managed reth and postgres services; missing service/URL is a failure. |`; extend
the `.#ci` row.

---

# Commit sequence (each verifies before the next)

1. `extend evm domain surface for live routing` — public `EvmReadSubject` +
   `EvmBlockAnchor` with accessors, `EvmReadIntent::subject()`, `EvmEndpoint`; tests
   in `crates/domains/evm/tests/target_contract.rs` + unit tests;
   `docs/evm-rpc-routing.md` derivation paragraph.
   Verify: `cargo test -p mfm-evm --all-targets`.
2. `add production json-rpc evm provider` — workspace deps, `JsonRpcEvmProvider` +
   `EvmProviderBuildError` + envelope/conversions (typed intent match, no mirror) +
   stub-server unit tests + `crates/live/evm/README.md` (alloy-only-here rule) +
   routing-doc failure note.
   Verify: `cargo test -p mfm-evm-live --all-targets`; the `cargo tree` gate;
   `nix run .#run -- --task cargo-check` once (sandbox resolves the new deps).
3. `move portfolio runtime composition into application` —
   `register_portfolio_states`/`portfolio_assembly`, `mfm-evm-live` promoted to
   dependency, `portfolio_runtime.rs` rewired to the shared fns,
   `docs/architecture.md` Application row, `crates/app/README.md`.
   Verify: `cargo test -p mfm-app --all-targets`.
4. `add postgres schema provisioning entry` — `install_schema`, ignored install test,
   storage README, `docs/design.md` sentence ("schema provisioning is one idempotent
   storage-crate entry that installs only into an empty store").
   Verify: `cargo test -p mfm-storage-postgres --lib`;
   `nix run .#run -- --task postgres-test`.
5. `add cli live snapshot commands` — `bin/cli/src/main.rs` (init/snapshot/show);
   `bin/cli/Cargo.toml` dependencies made explicit: `mfm-app`, `mfm-evm`,
   `mfm-evm-live`, `mfm-ids`, `mfm-portfolio`, `mfm-runtime`, `mfm-storage-postgres`,
   `serde`, `serde_json`, `tokio` (for `#[tokio::main]`; also serves the test target
   later — integration tests see normal dependencies, no tokio dev-dep needed), plus
   existing `clap`; `bin/cli/README.md` transport contract (incl. the one-route
   limitation), `docs/architecture.md` Binaries row.
   Verify: `cargo check -p mfm --all-targets`; manual smoke against a local pinned
   reth + postgres.
6. `replace mocked native run e2e with live coverage and fixture keeper` — delete
   ONLY `portfolio_runtime.rs:149-227` (the native hot/cold test); add
   `frozen_snapshot_fixture_remains_exact_canonical_json` to
   `crates/domains/portfolio/tests/planning_contract.rs`: canonical validity via
   `PlainCanonicalJsonBytes::from_canonical_json_slice`, digest pin
   `content:sha256-v1:79fa722c25a7f5ea67f6aab45eaa4918d9dff3607aae418dc2092d959a1e9e2b`
   (verified fact 4), plus checked `serde_json::from_str::<PortfolioSnapshotOutput>`
   if the type implements `Deserialize`. The failure-fixture consumer
   (`portfolio_runtime.rs:533`) survives this wave.
   Verify: `cargo test -p mfm-app -p mfm-portfolio --all-targets`.
7. `add gated cli e2e for native snapshot against managed services` — the test and
   its gate land together: `bin/cli/tests/cli_e2e.rs`,
   `[[test]] name = "cli_e2e" path = "tests/cli_e2e.rs"` (explicit path;
   `autotests = false`), sqlx dev-dep; nixfied edits (imports, `cli-e2e` leaf, `ci`
   seq); `docs/build-and-verification.md` rows. Pin `head_sequence`/totals strings
   from the manual red-green loop BEFORE finalizing the commit. During development,
   iterate with `nix run .#run -- --task cli-e2e`. Final-candidate verification, in
   order and nothing else: `nix run .#model-check`, `nix flake check --no-build`, one
   `nix run .#ci` (CI composes cli-e2e — do not run the task immediately before it);
   no concurrent gates.

Manual node for the red-green loop (repo-pinned rev, matches the CI service version):

```bash
nix build --no-link --print-out-paths \
  'github:NixOS/nixpkgs/421eebfd0ec7bccd4abe826ce62d7e6e83129493#reth'
<out>/bin/reth node --dev --ipcdisable --datadir /tmp/reth-e2e --http --http.port 18545 &
```

# Material uncertainties

1. Requires-bearing leaf directly in the `ci` seq. Assumption: admits like the
   composite precedent. Consequence: model admission failure. Validate:
   `nix run .#model-check` in commit 7; fallback single-step composite.
2. reqwest 0.12 no-default-features graph. Assumption: pure Rust, no TLS, no cmake in
   the nix sandbox. Consequence: sandbox build failure. Validate: `cargo tree` +
   `cargo-check` task in commit 2.
3. alloy-primitives 1.6.1 API names. Consequence: mechanical renames. Validate:
   compiler, commit 2.
4. Pinned e2e values (`head_sequence=11`, totals/amount strings). Assumption: live
   single-native-source Program shape matches the deleted mock. Consequence:
   assertion churn. Validate: manual red-green loop in commit 7; freeze thereafter.
5. Warm-chain exactness of account 1's `10^24` wei (no fee credit ever). Consequence:
   raw_units drift on warm slots. Validate: repeated `cli-e2e` on a warm slot;
   fallback: a higher-index never-used account.
6. Sandbox resolution of newly added deps (reqwest/alloy/sqlx-dev). Consequence:
   first task run fails on download. Validate: `cargo-check` task after commits 2
   and 7.
7. reth `--dev` serves anchored state for the seconds-old blocks the run touches.
   Consequence: loud `SafeFailure`/`Unavailable`. Validate: first manual/e2e run.

(Resolved, previously listed: `show` needing the fully bound assembly is settled by
`docs/architecture.md` — association pre-resolves every callback — and the cold-read
binding-failure runtime contract test.)
