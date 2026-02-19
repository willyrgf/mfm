# PROMPT
This is all context for our research. I'm using helios as the fastest way I found to instance a minimal ethereum node that start syncing as fast as possible so users can query their own local ethereum RPC. With this all in mind. Are there better options? If so, how would it fit this project? If helios is our best hope, how to make it sync as fast as possible? Since this is a opensource project the ideal scenario it should use open on internet, safe and reliable RPCs. What are our best options? If we can't find open, safe and reliable RPCs, what would be the next best option? parallel RPC execution and use the best information possible (in this case, I need to plan a new layer to handle it)? other best next option?

# ANSWER
You’re using Helios for a very specific (and honestly pretty hard) goal:

* **Give end users a “local” Ethereum JSON-RPC endpoint quickly**
* **Minimize sync time + hardware**
* **Avoid trusting a single remote RPC for correctness**

Helios is one of the few things that directly targets that combination: it’s a light client that **turns an untrusted RPC into a local RPC** and is designed to **sync in seconds with essentially no state DB**. ([GitHub][1])

That said, Helios doesn’t remove your dependency on *some* upstreams (execution RPC + consensus/light-client data + a weak-subjectivity checkpoint). It mostly changes the trust model and the startup profile.

Below is how I’d think about “better options”, how they fit MFM, and what I’d do to make Helios “as fast and reliable as possible” without shipping secrets.

---

## Are there better options than Helios?

“Better” depends on which axis you’re optimizing:

### Option A: A full node (best independence, worst startup cost)

Run an execution client (Reth/Geth/Nethermind/Erigon) + a consensus client (Nimbus/Lighthouse/Teku/etc). This is still the **most trustless + private** way to query Ethereum (no third-party RPC dependency), but it’s heavier. Ethereum.org explicitly calls out full nodes as the most trustless/private approach, while noting the hardware cost. ([ethereum.org][2])

How it could be “less awful”:

* **Snapshots** can radically reduce initial sync time for *some* clients and configurations. For example, Reth has a `download` command that supports public snapshot sources. ([reth.rs][3])
  (And there are providers publishing Reth snapshots claiming multi-hour restore-to-sync vs days from scratch, depending on hardware.) ([Merkle][4])

Fit for MFM:

* Add a `service::reth` (and optionally `service::nimbus`) module similar to `service::helios`.
* Have `MFM_EVM_RPC_URL` point at the local node when available.
* This becomes the **“power user / operator mode”** for people who want maximum independence.

When I’d recommend it:

* If “local RPC” must support **high-throughput / wide JSON-RPC surface** reliably (especially heavy `eth_getLogs` usage).
* If you care about **privacy** (Helios still leaks queries to upstream RPCs).

### Option B: Keep Helios (best startup UX) + make upstream selection robust

For your stated goal (“minimal node that starts syncing as fast as possible”), Helios is still one of the best “default” experiences available today: it’s explicitly built to sync quickly and run locally while verifying data from upstream endpoints. ([GitHub][1])

But: Ethereum.org also notes that light clients generally still rely on providers for data availability (today), and historically “production-ready” status has been a moving target. ([ethereum.org][2])
So for an open-source project, you want to **treat upstreams as unreliable** and design around it.

### Option C: Portal-network / fully P2P light data (promising, not a drop-in replacement yet)

The long-term direction is to reduce reliance on centralized RPCs via networks like the Portal Network. Ethereum.org explicitly mentions this as the future path for light clients to obtain data in a more decentralized way. ([ethereum.org][2])

Fit for MFM:

* This is a **future collector backend** (think `collectors/portal/*`), not something to bet your “default user RPC” on today unless you’re okay with limited method support and more experimentation.

---

## How this fits your project architecture

Your repo’s architecture is already set up for this kind of swap:

* **Services** (Nixfied modules) should manage “node-like processes” (Helios, Reth, etc).
* **Collectors** normalize IO and become the place to implement “smart RPC selection / retry / quorum”.
* **State logic** stays clean and deterministic (IO via provider; facts recorded for replay), per your design contract.

So the clean integration shape is:

1. **Keep `service::helios` as the default local RPC** (fast start).
2. Introduce a new **collector layer** for Ethereum JSON-RPC that can operate in multiple modes:

   * `local_helios` (preferred when running)
   * `local_fullnode` (optional)
   * `remote_pool` (fallback)
3. Wire it through your IO abstraction so:

   * Live runs record facts (`FactKey -> payload_id`)
   * Replay runs never touch the network

That’s completely aligned with your “no ambient IO in state logic” + fact recording rules.

---

## If Helios is the best default: how to make it sync/start as fast as possible

Helios startup speed in practice is dominated by:

* **Checkpoint freshness and retrieval**
* **Consensus RPC responsiveness**
* **Execution RPC responsiveness + method support (notably `eth_getProof`)**

Helios’ own config docs are very clear about the knobs that matter:

* `checkpoint` should be **trusted** and **not older than ~2 weeks**
* `data_dir` should be persistent and ideally on fast storage
* `max_checkpoint_age` controls when Helios tries to refresh
* `fallback` and `load_external_fallback` exist, but `load_external_fallback` is explicitly called **insecure / not recommended** ([GitHub][5])

### Concrete improvements I would apply in *your* Nixfied integration

1. **Make Helios data dir persistent by default**
   Helios recommends a persistent `data_dir`. ([GitHub][5])
   In MFM terms: prefer `SERVICE_OWNER_SCOPE=persistent` for Helios (or equivalent default in your module), because the stored checkpoint DB is tiny but saves repeated bootstrap work.

2. **Stop treating “derive checkpoint dynamically” as the normal path**
   If your lifecycle currently derives checkpoints when missing, that’s fine as a fallback, but it shouldn’t be the happy path if “fast start” is the product goal.

A practical strategy:

* Use Helios’ default consensus RPC when possible (it defaults to `lightclientdata.org` per Helios’ docs). ([GitHub][5])
* Set a **reasonable `max_checkpoint_age`** so most restarts don’t refresh.
* Only fall back to external checkpoint sources if your primary path fails.

3. **Ensure your execution RPC supports `eth_getProof`**
   Helios requires an execution RPC that provides `eth_getProof`. ([GitHub][1])
   So whatever “public RPC” defaults you ship must be validated for `eth_getProof` at startup (quick probe), otherwise users get mysterious failures.

---

## “Open on the internet, safe, reliable” RPCs: what are the best realistic options?

You have two upstream categories:

### A) Execution RPC for Helios (must support `eth_getProof`)

Two “no API key” candidates that explicitly document `eth_getProof` support:

* **Cloudflare Ethereum Gateway (trial endpoint)** — Cloudflare documents a public “trial” gateway and lists supported methods including `eth_getProof`. ([Cloudflare Docs][6])
* **dRPC public Ethereum endpoint** — dRPC documents a public endpoint and documents `eth_getProof` as supported. ([dRPC][7])

Example (putting URLs in a code block so you can copy/paste):

```bash
# Execution RPC candidates (probe eth_getProof support at startup)
CLOUDFLARE_ETH_RPC="https://web3-trial.cloudflare-eth.com/v1/mainnet"
DRPC_ETH_RPC="https://eth.drpc.org"
```

Important reality check:

* “Open” endpoints are often **rate-limited, best-effort, or change over time**.
* Even if correctness is verified by Helios, **privacy isn’t**: upstreams still see what you query.

If you care about privacy as well as openness, dRPC also documents Tor access (slower, but more private). ([dRPC][8])

### B) Consensus / light-client data RPC for Helios

Helios requires a consensus endpoint that supports the **light client beacon chain API** and defaults to `lightclientdata.org` if not provided. ([GitHub][5])
Nimbus documents that its beacon node serves light client data by default. ([nimbus.guide][9])

So the open-ish options are:

* Use Helios default (`lightclientdata.org`) ([GitHub][5])
* Use a known public beacon endpoint that serves light client routes (needs ongoing validation)
* Or: let advanced users run their own Nimbus beacon node (more setup, more independence) ([nimbus.guide][9])

---

## If “open, safe, reliable” can’t all be true at once: what’s next best?

In practice, you pick two:

* **Open + Safe** (Helios + public endpoints) → reliability varies
* **Safe + Reliable** (Helios + paid RPC) → not open
* **Open + Reliable** (hard) → you usually end up building a pool, or you become an RPC provider

So I’d implement a *tiered* fallback strategy in MFM:

### Tier 1: Helios + a small curated pool of public RPCs (no keys)

* Default config ships with 2–3 execution RPC candidates + 2–3 consensus candidates.
* At startup, run **capability + health probes**:

  * `eth_chainId`, `eth_blockNumber`
  * `eth_getProof` (execution RPC must pass)
  * timeouts + latency sampling
* Choose “best” endpoint and keep rotating if it degrades.

This gives you “open-ish” and “mostly reliable”.

### Tier 2: Parallel RPC execution (hedged requests) + scoring

Yes: this is the “new layer” you’re hinting at, and it fits your architecture best as a **collector**.

But do it in a way that’s compatible with your replay model:

* In **Live**:

  * You may race multiple upstreams.
  * Record:

    * which upstream “won”
    * the request + response as the fact payload
* In **Replay**:

  * Return the recorded response
  * Never re-race

Mechanically, this is just a higher-level collector that produces deterministic replay via facts.

Practical policies:

* **Hedged requests**: send to primary, wait N ms, send to secondary, take first success.
* **Failover**: on specific error codes (timeouts, 429s), rotate.
* **Method-aware limits**:

  * `eth_getLogs` gets special handling (range chunking, provider-specific limits, caching), because it’s where public RPCs tend to hurt you.

### Tier 3: Add a “bring your own RPC” path (paid keys) but keep it secret-safe

This is often the “best reliability per user” solution.

The key MFM constraint: **never persist secrets** (URLs with API keys included). So:

* Read provider URLs from environment at runtime only
* Avoid capturing them in run manifests
* Avoid logging them

Helios itself recommends providers like Alchemy/Infura for execution RPC in examples, but those typically require keys. ([GitHub][1])
So treat that as an opt-in “reliability mode” rather than default.

### Tier 4: Full node mode (Reth snapshot restore) for users who want maximum independence

This is the “I don’t want to depend on public RPCs at all” option.

Reth’s CLI includes snapshot download support with documented snapshot sources. ([reth.rs][3])
This is how you give advanced users a path to “reliable and private local RPC”, at the cost of disk/CPU.

---

## A concrete plan I’d implement in MFM

### 1) Add a first-class “RPC Source Policy” abstraction

A small model that lives in `crates/core` config structs (no secrets inside), something like:

* `rpc_policy.mode = helios | local_fullnode | remote_pool`
* `rpc_policy.execution_sources = [cloudflare_trial, drpc_public, user_env]`
* `rpc_policy.consensus_sources = [lightclientdata_default, user_env]`
* `rpc_policy.strategy = failover | hedged | quorum_for_unverified_methods`

Where “cloudflare_trial” / “drpc_public” are *identifiers*, not URLs (URLs resolved at runtime).

### 2) Implement the pool in `crates/collectors/*`

A new collector (or extend your existing EVM collector) that:

* probes endpoints
* scores them
* routes calls
* emits structured domain events like `RpcEndpointDegraded`, `RpcEndpointSelected`, etc. (no secrets)

### 3) Keep binaries thin

CLI/REST only select policy + start services; no logic leaks into transport.

### 4) Keep Helios as the “fast default”, but make it resilient

* persistent `data_dir` ([GitHub][5])
* avoid insecure checkpoint fallback by default ([GitHub][5])
* multi-endpoint execution RPC fallback (must pass `eth_getProof`) ([GitHub][1])

---

## Bottom line

* If your top priority is **fast startup + minimal local resources**, Helios is still the best default **class** of solution for a local RPC experience. ([GitHub][1])
* The “real” engineering work is making **upstreams replaceable + probeable + poolable** without violating:

  * deterministic replay
  * no-secrets surfaces
  * thin transport layers

If you want, I can sketch (at the level of Rust modules + traits + fact-key strategy) what the `collectors/evm-rpc-pool` API should look like so it plugs cleanly into your `IoProvider` + replay model, including how to record “winner endpoint” deterministically without ever persisting URLs or keys.

[1]: https://github.com/a16z/helios "https://github.com/a16z/helios"
[2]: https://ethereum.org/developers/docs/nodes-and-clients/light-clients/ "https://ethereum.org/developers/docs/nodes-and-clients/light-clients/"
[3]: https://reth.rs/cli/reth/download/ "https://reth.rs/cli/reth/download/"
[4]: https://www.merkle.io/blog/reth-snapshots-faster-syncing-ethereum "https://www.merkle.io/blog/reth-snapshots-faster-syncing-ethereum"
[5]: https://raw.githubusercontent.com/a16z/helios/master/config.md "https://raw.githubusercontent.com/a16z/helios/master/config.md"
[6]: https://developers.cloudflare.com/web3/how-to/use-ethereum-gateway/ "https://developers.cloudflare.com/web3/how-to/use-ethereum-gateway/"
[7]: https://drpc.org/docs/ethereum-api "https://drpc.org/docs/ethereum-api"
[8]: https://drpc.org/docs/tor "https://drpc.org/docs/tor"
[9]: https://nimbus.guide/light-client-data.html "https://nimbus.guide/light-client-data.html"


## Sketch Public API
Below is a concrete sketch that’s meant to “snap into” MFM’s contract:

* **State code stays pure** (no ambient IO)
* All network calls go through **`IoProvider.call()`**
* In **Live**, the pool does the messy stuff (probe / hedge / failover / quorum) and returns **one canonical response**
* The engine records that response under the provided **`FactKey`**
* In **Replay**, the same call returns the recorded response (pool handler is never invoked)

I’m going to sketch this as a new collector crate:

`crates/collectors/evm-rpc-pool/` (Cargo: `mfm-collectors-evm-rpc-pool`)

---

## 1) What the pool “is” in MFM terms

### It should be an IO namespace handler, not a wrapper that calls `io.call()` multiple times

If the pool itself called `io.call()` multiple times (for hedging), you’d risk:

* recording multiple facts for one logical call
* non-deterministic selection unless you also record the selection

Instead, make the pool a **single** IO call:

* `IoCall.namespace = "evm.rpc_pool"`
* `IoCall.fact_key = Some(base_key)` for any call that must be replayable
* the pool handler internally fans out to multiple upstream RPCs, chooses the best answer, returns **one** response JSON
* LiveIo records it
* ReplayIo returns it

This keeps fact semantics simple: **1 logical call → 1 fact key → 1 recorded payload**.

---

## 2) Public “state-facing” API (typed wrapper)

State code should never build raw JSON by hand; give it a typed facade that:

* builds an `IoCall` for `"evm.rpc_pool"`
* generates safe fact keys
* parses JSON-RPC results into Rust types

### `client.rs`

```rust
// crates/collectors/evm-rpc-pool/src/client.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use mfm_machine::errors::IoError;
use mfm_machine::ids::FactKey;
use mfm_machine::io::{IoCall, IoProvider, IoResult};

use crate::facts::FactKeyBuilder;
use crate::types::{EvmChain, PoolPolicy, RpcEnvelope, RpcRequest};

pub struct EvmRpcPoolClient {
    pub chain: EvmChain,
}

impl EvmRpcPoolClient {
    pub fn new(chain: EvmChain) -> Self {
        Self { chain }
    }

    /// Low-level escape hatch (still typed at the pool-request level).
    pub async fn call_raw(
        &self,
        io: &mut dyn IoProvider,
        fact_key: FactKey,
        req: RpcRequest,
        policy: PoolPolicy,
    ) -> Result<RpcEnvelope, IoError> {
        let call = IoCall {
            namespace: "evm.rpc_pool".to_string(),
            request: serde_json::to_value(crate::types::PoolCall {
                chain: self.chain,
                req,
                policy,
            }).expect("canonical-JSON compatible"),
            fact_key: Some(fact_key),
        };

        let IoResult { response, .. } = io.call(call).await?;

        // Response is a pool envelope (meta + jsonrpc response)
        let env: RpcEnvelope = serde_json::from_value(response)
            .map_err(|e| crate::errors::io_parse_error("evm_rpc_pool_bad_response", e))?;

        Ok(env)
    }

    /// Convenience typed call pattern:
    /// - builds a stable fact key from (state-scoped prefix + method + params hash + ordinal)
    /// - returns decoded `T`
    pub async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        io: &mut dyn IoProvider,
        facts: &mut FactKeyBuilder,
        req: RpcRequest,
        policy: PoolPolicy,
    ) -> Result<T, IoError> {
        let fk = facts.key_for_rpc(&req);

        let env = self.call_raw(io, fk, req, policy).await?;

        // Extract jsonrpc result and decode T
        let result = env.response.result
            .ok_or_else(|| crate::errors::io_protocol_error("evm_rpc_missing_result"))?;

        let out: T = serde_json::from_value(result)
            .map_err(|e| crate::errors::io_parse_error("evm_rpc_result_decode_failed", e))?;

        Ok(out)
    }

    // Example sugar: eth_chainId
    pub async fn chain_id(
        &self,
        io: &mut dyn IoProvider,
        facts: &mut FactKeyBuilder,
    ) -> Result<String, IoError> {
        self.call(
            io,
            facts,
            RpcRequest::new("eth_chainId", serde_json::json!([])),
            PoolPolicy::default_light(),
        ).await
    }
}
```

This is what states use.

---

## 3) Fact keys: stable, deterministic, and robust to retries

You want keys that are:

* deterministic
* do not depend on wall clock
* resilient if you add/remove calls (better than “just ordinal”)
* don’t contain secrets

### `facts.rs`

```rust
// crates/collectors/evm-rpc-pool/src/facts.rs

use serde_json::Value;
use sha2::{Digest, Sha256};

use mfm_machine::ids::FactKey;

use crate::types::RpcRequest;

/// Builds per-state-scoped fact keys.
///
/// IMPORTANT: prefix should be stable and safe.
/// Good default: the planned StateId string (op_path + state_local_id),
/// which ops can embed into the state struct at expand() time.
pub struct FactKeyBuilder {
    prefix: String,
    ordinal: u32,
}

impl FactKeyBuilder {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self { prefix: prefix.into(), ordinal: 0 }
    }

    pub fn key_for_rpc(&mut self, req: &RpcRequest) -> FactKey {
        self.ordinal += 1;

        // Canonicalize a stable request identity.
        // NOTE: You want canonical JSON semantics here; if your codebase already
        // has a canonical JSON serializer, use that instead of serde_json::to_vec.
        let req_id = serde_json::json!({
            "method": req.method,
            "params": req.params,
        });

        let bytes = serde_json::to_vec(&req_id).expect("json");
        let hash = Sha256::digest(&bytes);
        let h = hex::encode(hash);

        // FactKey format is not constrained like StateId, so we can use dots safely.
        // Keep it structured; don’t put URLs or headers here.
        FactKey(format!(
            "{}.evm.jsonrpc.{}.{}.{}",
            self.prefix,
            sanitize_method(req.method),
            h,
            self.ordinal
        ))
    }
}

fn sanitize_method(m: &str) -> String {
    // Convert "eth_getBalance" to "eth_getbalance" or similar stable safe format.
    // Keep it simple; this is just for readability in debugging.
    m.to_ascii_lowercase()
}
```

**Key point:** the state should carry a stable `prefix`. Since ops create states during `expand()`, they can pass something like:

* `prefix = state_id_string` (the planner knows the intended ID format: `<op_path>.<state_local_id>` even if the engine assigns the full StateId)
* or `prefix = "<op_path>.<state_local_id>"` as a close equivalent

You don’t need *actual* run_id/attempt in the fact key; retries reuse the same key, which is often what you want (single-assignment fact binding within a run).

---

## 4) IO request/response schema for `"evm.rpc_pool"`

This is the **payload** inside `IoCall.request` and `IoResult.response`.

### `types.rs`

```rust
// crates/collectors/evm-rpc-pool/src/types.rs

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvmChain {
    Mainnet,
    // Future: holesky, sepolia, custom chain id...
    // Custom(u64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcRequest {
    pub method: String,
    pub params: Value,
}

impl RpcRequest {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self { method: method.into(), params }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolCall {
    pub chain: EvmChain,
    pub req: RpcRequest,
    pub policy: PoolPolicy,
}

/// How aggressive the pool should be for this call.
///
/// Keep this small; you can add nuance later.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolPolicy {
    pub class: CallClass,
    pub routing: RoutingPolicy,
    pub quorum: Option<QuorumPolicy>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CallClass {
    Light,   // chainId, blockNumber, getBalance, getCode...
    Heavy,   // getLogs, trace_*, large responses...
    TxSend,  // eth_sendRawTransaction / eth_sendTransaction
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RoutingPolicy {
    PreferLocalThenRemote,
    RemoteOnly,
    LocalOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuorumPolicy {
    pub n: u8,         // number of sources to query
    pub min_agree: u8, // minimum agreeing responses
}

impl PoolPolicy {
    pub fn default_light() -> Self {
        Self {
            class: CallClass::Light,
            routing: RoutingPolicy::PreferLocalThenRemote,
            quorum: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcEnvelope {
    pub meta: RpcMeta,
    pub response: JsonRpcResponse,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcMeta {
    /// A stable non-secret identifier like "helios_local", "drpc_public", "cloudflare_trial".
    pub selected_source: String,

    /// For transparency/debugging (safe, no URLs).
    pub attempted_sources: Vec<String>,

    /// Optional diagnostic fields (do not include request URLs, headers, raw error bodies).
    pub strategy: String,        // "local", "failover", "hedged", "quorum"
    pub elapsed_ms: Option<u64>, // ok to store; it’s a fact anyway
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>, // consider dropping or hard-limiting for no-secrets + size
}
```

The envelope gives you a place to put **safe metadata** (source IDs, strategy) without leaking URLs.

---

## 5) The pool runtime handler (Live mode)

This is the part that actually:

* maintains endpoint health
* probes capabilities
* performs hedged requests / failover / quorum
* sanitizes errors

### The handler’s job

Given `PoolCall { chain, req, policy }`:

1. Build a candidate list of sources (local helios, local reth, remote pool)
2. Apply routing policy
3. Execute with strategy:

   * Light calls: can hedge/failover aggressively
   * Heavy calls: avoid hedging (costly); prefer sequential failover
   * TxSend: never quorum; typically “prefer local fullnode” or “prefer single best”
4. Return **one** `RpcEnvelope`

### `handler.rs` sketch

```rust
// crates/collectors/evm-rpc-pool/src/handler.rs

use async_trait::async_trait;
use serde_json::Value;
use tokio::time::{timeout, Duration, Instant};

use mfm_machine::errors::IoError;
use mfm_machine::io::IoCall;

use crate::types::{PoolCall, RpcEnvelope, RpcMeta};

/// This trait likely already exists somewhere in your runtime.
/// Sketching it for clarity.
#[async_trait]
pub trait IoNamespaceHandler: Send + Sync {
    fn namespace(&self) -> &'static str;
    async fn handle(&self, call: IoCall) -> Result<Value, IoError>;
}

pub struct EvmRpcPoolHandler {
    sources: Vec<RpcSource>, // resolved at runtime; URLs not persisted
    cfg: PoolRuntimeConfig,
}

pub struct PoolRuntimeConfig {
    pub light_timeout: Duration,
    pub heavy_timeout: Duration,
    pub hedge_delay: Duration, // e.g. 150ms
    pub max_parallel: usize,   // e.g. 2 for light calls
}

/// A runtime-resolved endpoint. IMPORTANT: url/api-key stay in-memory only.
pub struct RpcSource {
    pub id: String,               // "helios_local", "drpc_public", ...
    pub kind: SourceKind,         // Local/Remote/...
    pub client: Box<dyn JsonRpcTransport>,
    pub capabilities: Capabilities,
}

pub enum SourceKind { Local, Remote }

pub struct Capabilities {
    pub supports_get_proof: bool,
    pub supports_get_logs: bool,
    // ...
}

/// Transport used by handler internally. Could be reqwest-based.
#[async_trait]
pub trait JsonRpcTransport: Send + Sync {
    async fn send(&self, method: &str, params: &Value) -> Result<crate::types::JsonRpcResponse, TransportError>;
}

pub struct TransportError {
    pub retryable: bool,
    pub category: String, // "timeout", "rate_limited", "http", ...
    pub safe_message: String,
}

#[async_trait]
impl IoNamespaceHandler for EvmRpcPoolHandler {
    fn namespace(&self) -> &'static str { "evm.rpc_pool" }

    async fn handle(&self, call: IoCall) -> Result<Value, IoError> {
        let pool_call: PoolCall = serde_json::from_value(call.request)
            .map_err(|e| crate::errors::io_parse_error("evm_rpc_pool_bad_request", e))?;

        let start = Instant::now();
        let (resp, meta) = self.execute(pool_call).await?;
        let env = RpcEnvelope {
            meta: RpcMeta {
                elapsed_ms: Some(start.elapsed().as_millis() as u64),
                ..meta
            },
            response: resp,
        };

        Ok(serde_json::to_value(env).expect("json"))
    }
}
```

### Execution strategy sketch

You can implement a small decision matrix:

* `PreferLocalThenRemote`:

  * if any healthy local source exists → try local first
  * else → remote pool

* `Light` calls:

  * hedged: start best candidate immediately; after `hedge_delay`, start second candidate; take first success
  * on failure, optionally try remaining sequentially

* `Heavy` calls:

  * no hedging (expensive)
  * sequential failover: best → next → next

* `Quorum` (optional, carefully scoped):

  * query N sources (in parallel)
  * normalize responses
  * require min_agree equality (strict or method-specific comparator)
  * if no quorum, return structured error

I’d start with **failover + hedging for Light only**, and add quorum later only for a narrow list of “easy to compare” methods.

---

## 6) Configuration: endpoint IDs, no secrets

The pool config should separate:

* **Stable, persistable configuration** (safe to appear in manifests)
* **Runtime-resolved secrets** (URLs with keys)

### Persistable (safe) config shape

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcPoolConfig {
    pub sources: Vec<RpcSourceRef>, // only IDs + kinds
    pub ordering: Vec<String>,      // preferred order of ids
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcSourceRef {
    pub id: String,         // "helios_local" / "drpc_public" / "user_primary"
    pub kind: SourceKindRef // LocalHelios | LocalReth | Remote
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SourceKindRef {
    LocalHelios,
    LocalReth,
    Remote,
}
```

### Runtime URL resolution (NOT persisted)

* LocalHelios → derived from service discovery (`127.0.0.1:<heliosPort>`)
* Remote IDs → resolved from env:

Examples (just naming ideas):

```bash
MFM_EVM_RPC_URL__DRPC_PUBLIC="https://eth.drpc.org"
MFM_EVM_RPC_URL__CLOUDFLARE_TRIAL="https://web3-trial.cloudflare-eth.com/v1/mainnet"
MFM_EVM_RPC_URL__USER_PRIMARY="https://.../?api_key=SECRET"
```

The handler only ever stores `id` in returned meta/errors/events — never the URL.

---

## 7) Error model (safe + structured)

Keep error details stable and non-secret.

### `errors.rs`

```rust
pub fn io_protocol_error(code: &str) -> mfm_machine::errors::IoError {
    use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
    use mfm_machine::ids::ErrorCode;

    IoError::Other(ErrorInfo {
        code: ErrorCode(code.to_string()),
        category: ErrorCategory::Rpc,
        retryable: false,
        message: "RPC protocol error".to_string(),
        details: None,
    })
}
```

For pool exhaustion failures, include only:

* attempted source IDs
* categories (“timeout”, “rate_limited”)
* **not** raw upstream error bodies (those can contain arbitrary data)

---

## 8) Example usage in a state

In op expansion, create the state with a stable prefix:

* `prefix = format!("{}", planned_state_id)` (or op_path + state_local_id)

Then:

```rust
use mfm_collectors_evm_rpc_pool::{EvmRpcPoolClient, FactKeyBuilder, types::EvmChain};
use mfm_machine::meta::{StateMeta, SideEffectKind, Idempotency};
use mfm_machine::meta::standard_tags as tags;

pub struct FetchBalanceState {
    fact_prefix: String,
    address: String,
}

#[async_trait::async_trait]
impl mfm_machine::state::State for FetchBalanceState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: vec![tags::FETCH_DATA.into()],
            depends_on: vec![],
            depends_on_strategy: mfm_machine::meta::DependencyStrategy::LatestSuccessful,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        _ctx: &mut dyn mfm_machine::context::DynContext,
        io: &mut dyn mfm_machine::io::IoProvider,
        _rec: &mut dyn mfm_machine::recorder::EventRecorder,
    ) -> Result<mfm_machine::state::StateOutcome, mfm_machine::errors::StateError> {
        let client = EvmRpcPoolClient::new(EvmChain::Mainnet);
        let mut facts = FactKeyBuilder::new(&self.fact_prefix);

        // Example JSON-RPC call:
        let bal_hex: String = client.call(
            io,
            &mut facts,
            mfm_collectors_evm_rpc_pool::types::RpcRequest::new(
                "eth_getBalance",
                serde_json::json!([self.address, "latest"]),
            ),
            mfm_collectors_evm_rpc_pool::types::PoolPolicy::default_light(),
        ).await.map_err(|e| crate::map_io_to_state_error(e))?;

        // ... write to context, etc.

        Ok(mfm_machine::state::StateOutcome { snapshot: mfm_machine::state::SnapshotPolicy::OnSuccess })
    }
}
```

---

## 9) Minimal roadmap (what I would implement first)

To keep this from ballooning:

### Milestone A (high value, low risk)

* Pool routing:

  * prefer local helios if healthy
  * else remote failover list
* Strategies:

  * **Light**: hedged (2 sources max)
  * **Heavy**: sequential failover only
* Capability probes:

  * `eth_chainId`
  * `eth_blockNumber`
  * optional: per-source “max response size” guardrails for heavy methods

### Milestone B

* Method-aware policies:

  * enforce “no hedging” for `eth_getLogs`
  * add chunking helper in the typed client (`get_logs_chunked(...)`) so state code doesn’t reimplement it

### Milestone C (optional, narrow)

* Quorum for a tiny allowlist of methods where equality is straightforward

  * (I would not do quorum for `eth_getLogs` early; it’s messy and expensive.)

---

## 10) One skeptical note (worth calling out)

A pool can improve **availability** and sometimes **correctness**, but it can quietly harm **privacy** (more upstreams see your requests). That’s one reason I like:

* **hedging only for light calls**
* **no hedging for heavy / sensitive calls**
* prefer local Helios/fullnode first whenever available

---

If you tell me (1) what your existing `collectors/*` interface looks like (i.e., how IO namespaces are registered into `LiveIo` today), I can adapt this sketch into the exact trait/module pattern you already use so it’s not “invented in a vacuum.”

