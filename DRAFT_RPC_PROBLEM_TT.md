# DRAFT RPC Problem Proposal (Revised)

Revision status: revised for current MFM architecture (2026-02-19).

Key revision highlights:

* Keep `namespace = "evm"` and evolve existing transports instead of introducing `evm.rpc_pool`.
* Reuse canonical fact-key derivation (`artifact_id_for_json`) and existing no-secrets safeguards.
* Prioritize Milestone A (`failover + light hedging`) and defer quorum to a narrow, later phase.

---

# PROMPT
This is all context for our research. I'm using helios as the fastest way I found to instance a minimal ethereum node that start syncing as fast as possible so users can query their own local ethereum RPC. With this all in mind. Are there better options? If so, how would it fit this project? If helios is our best hope, how to make it sync as fast as possible? Since this is a opensource project the ideal scenario it should use open on internet, safe and reliable RPCs. What are our best options? If we can't find open, safe and reliable RPCs, what would be the next best option? parallel RPC execution and use the best information possible (in this case, I need to plan a new layer to handle it)? other best next option?

# ANSWER
## Decision Summary

* Keep **Helios** as the default local RPC path for fast startup and low resource usage.
* Build reliability by extending the existing **`namespace="evm"`** collector/transport path (not a new `evm.rpc_pool` namespace).
* Use current canonical fact-key + replay machinery (`artifact_id_for_json`-based derivation); do not add parallel hashing logic.
* Implement in phases: ship **Milestone A** first (failover + light hedging), then add method-aware policies; defer quorum to a narrow optional phase.
* Keep URLs/auth strictly runtime-only; persist only safe source IDs and sanitized diagnostics.

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

* Reuse your existing `service::reth` and `service::helios` modules; add optional `service::nimbus` only if you want user-run consensus as a first-class path.
* Expose the local node through a shared RPC source-discovery registry (`local_reth` / `helios_local`) instead of per-command URL wiring.
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
* `rpc_policy.strategy = failover | hedged_light` (add quorum later, only for narrow allowlists)

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

If you want, I can sketch (at the level of Rust modules + traits + fact-key strategy) how to extend the existing `collectors/evm` + `collectors/evm-jsonrpc-http` path so it plugs cleanly into your `IoProvider` + replay model, including how to record “winner endpoint” deterministically without ever persisting URLs or keys.

[1]: https://github.com/a16z/helios "https://github.com/a16z/helios"
[2]: https://ethereum.org/developers/docs/nodes-and-clients/light-clients/ "https://ethereum.org/developers/docs/nodes-and-clients/light-clients/"
[3]: https://reth.rs/cli/reth/download/ "https://reth.rs/cli/reth/download/"
[4]: https://www.merkle.io/blog/reth-snapshots-faster-syncing-ethereum "https://www.merkle.io/blog/reth-snapshots-faster-syncing-ethereum"
[5]: https://raw.githubusercontent.com/a16z/helios/master/config.md "https://raw.githubusercontent.com/a16z/helios/master/config.md"
[6]: https://developers.cloudflare.com/web3/how-to/use-ethereum-gateway/ "https://developers.cloudflare.com/web3/how-to/use-ethereum-gateway/"
[7]: https://drpc.org/docs/ethereum-api "https://drpc.org/docs/ethereum-api"
[8]: https://drpc.org/docs/tor "https://drpc.org/docs/tor"
[9]: https://nimbus.guide/light-client-data.html "https://nimbus.guide/light-client-data.html"


## Sketch Public API (Revised For Current MFM)
Below is the same idea, but adjusted to match the repository as it exists today.

### Core alignment points

* Keep **state code pure** and all network access behind `IoProvider.call()`.
* Keep **`namespace = "evm"`** (do not introduce `evm.rpc_pool` yet).
* Reuse existing canonical fact-key derivation (`artifact_id_for_json` path) rather than adding a parallel `FactKeyBuilder`.
* Keep URLs/auth runtime-only; persisted surfaces store only safe source identifiers.

---

## 1) Keep the existing namespace and evolve it

MFM already has:

* `mfm_collectors_evm` typed client wrappers over `namespace="evm"`
* `mfm_collectors_evm_jsonrpc_http` live transport implementation
* router wiring by namespace group (`evm` route in app runtime)

So the lowest-risk path is:

* extend the `evm` request envelope to optionally include a **routing hint by source ID**
* keep all existing `EvmIoClient` call sites intact
* implement pooling/failover inside the `evm` live transport factory

That avoids broad cross-crate churn.

---

## 2) Fact keys: reuse canonical hashing already in-tree

Do **not** add ad-hoc request hashing (`serde_json::to_vec`, custom SHA path, ordinal mix).
Use the existing derivation path from `mfm_collectors_evm`:

* request payload -> canonical JSON hash (`artifact_id_for_json`)
* `FactKey` bound to state ID + request hash
* same request + same state yields stable deterministic key

This preserves current replay semantics and secret scanning behavior.

---

## 3) Remove per-request URL routing from new code paths

Current transport supports `rpc_url` in request payload. That was useful for bootstrap, but it is not the right long-term shape for a pool and can force URL-like values into request payloads.

Recommended target:

* request payload includes `method`, `params`, and optional `route.source_id`
* runtime resolves `source_id -> URL/auth` from env/service discovery
* persisted facts/events/errors include source IDs only (`helios_local`, `drpc_public`, etc.)
* remove `rpc_url` payload routing from the EVM read path as a breaking change

---

## 4) Runtime strategy (Milestone A first)

Start with high-value behavior only:

* routing preference: local Helios -> local full node -> remote pool
* **Light** calls: hedged (max 2 concurrent) with short hedge delay
* **Heavy** calls (especially `eth_getLogs`): sequential failover only
* no quorum in Milestone A

This gives immediate reliability gains while keeping determinism and complexity under control.

---

## 5) Pool configuration shape

Persistable config should carry only non-secret references:

```rust
pub struct RpcPoolConfig {
    pub sources: Vec<RpcSourceRef>, // IDs + type only
    pub preferred_order: Vec<String>,
}

pub struct RpcSourceRef {
    pub id: String, // "helios_local", "drpc_public", "cloudflare_trial", "user_primary"
    pub kind: SourceKindRef,
}
```

Runtime-only resolution:

* local IDs -> discovered local service endpoints
* remote IDs -> env-provided URLs/auth (never persisted, never logged)

---

## 6) State-facing usage

No new state-facing client is required for Milestone A.
States keep using `EvmIoClient` and `JsonRpcCall`; transport decides source selection and failover.

That keeps ops and state code thin and stable.

---

## 7) Minimal roadmap

### Milestone A (ship first)

* Extend `evm` live transport for source probing/scoring and failover.
* Add light-call hedging (2-source max).
* Add capability probe checks (`eth_chainId`, `eth_blockNumber`, `eth_getProof` where required by Helios path).
* Add tests for:
  * replay determinism (single logical call -> single fact payload)
  * no-secrets behavior in errors/facts
  * fallback behavior under timeout/429.

### Milestone B

* Method-aware policies (`eth_getLogs` chunking and no-hedge enforcement).
* Better endpoint health decay/recovery scoring.

### Milestone C (optional, narrow)

* Add quorum only for a small allowlist of trivially comparable methods.
* Keep quorum off for heavy methods and `eth_getLogs`.

---

## 8) Skeptical note

Pooling improves availability and can improve correctness checks, but it can reduce privacy because more upstreams may see traffic.

Practical guardrails:

* hedge only light reads
* avoid hedging heavy/sensitive methods
* prefer local Helios/fullnode first whenever healthy

---

# Deep Dive Expansion (All Proposal Parts)

This section expands each proposal item above into concrete, implementation-level guidance that fits the current MFM tree (`crates/collectors/evm`, `crates/collectors/evm-jsonrpc-http`, `crates/app`, and Nixfied Helios service wiring).

## Part 0) Ground Rules and Success Criteria

### 0.1 Architecture constraints that this plan must preserve

* Keep business execution logic out of `bin/*`; transport/routing behavior lives in collectors/runtime wiring.
* Keep `namespace = "evm"` as the sole IO surface for EVM JSON-RPC.
* Preserve deterministic replay by continuing to bind one logical request to one `FactKey -> payload_id`.
* Preserve no-secrets surfaces: URLs with keys and authorization headers remain runtime-only.
* Preserve canonical JSON hashing via existing `artifact_id_for_json` derivation paths.

### 0.2 Primary outcome metrics

* Startup UX: time from `svc::helios::start` to first successful `eth_blockNumber`.
* Availability: percentage of successful EVM read calls under injected timeout/429 faults.
* Replay correctness: same run replay returns the recorded payloads without network calls.
* Secret safety: no URL credentials/auth headers in events, artifacts, errors, or logs.

---

## Part 1) Decision Summary Deep Dive

### 1.1 Keep Helios as default local RPC

Why this is still the right default for MFM:

* Existing Nixfied integration already starts/stops Helios and uses it in parity/mainnet workflows.
* Fast bootstrap is a product-level requirement; Helios is optimized for exactly that.
* It keeps local resource requirements low compared to full-node defaults.

How to operationalize:

* Default to local Helios first in source ordering when healthy.
* Keep Helios `data_dir` persistent so restarts avoid repeated cold bootstrap.
* Treat execution/consensus upstreams as unreliable dependencies and add pool logic around them.

### 1.2 Extend existing `evm` collector path instead of new namespace

Why:

* Current states/ops already depend on `EvmIoClient` and `JsonRpcCall`.
* Introducing `evm.rpc_pool` would add broad routing and migration churn without early value.
* Existing `RouterLiveIoTransportFactory` already routes by namespace, so behavior can be upgraded in-place.

Implementation principle:

* Keep state-facing API stable.
* Concentrate failover/hedging/probing inside `crates/collectors/evm-jsonrpc-http`.

### 1.3 Reuse canonical fact-key derivation

Current in-tree behavior already does what we need:

* `crates/collectors/evm/src/lib.rs` derives `FactKey` from canonical JSON hash of `JsonRpcCall`.
* Secrets/floats are rejected by canonical hash path.

Required discipline:

* Do not add parallel hash builders for EVM requests.
* Any transport-internal routing state must not alter `JsonRpcCall` hashing unless explicitly intended.

### 1.4 Ship Milestone A first; defer quorum

Why this sequencing is important:

* Availability gains mostly come from failover and light hedging.
* Quorum adds complexity, latency, and comparison semantics risk.
* Quorum is only defensible for narrow, trivially comparable read methods.

Milestone A scope (strict):

* failover for retryable transport failures
* hedge only lightweight idempotent reads
* capability probes and endpoint scoring
* no quorum

### 1.5 Keep URLs/auth runtime-only

Threat surfaces:

* request payload overrides (`rpc_url`)
* error details payloads
* structured logs and diagnostics
* run artifacts/events

Controls:

* persist source identifiers only (`helios_local`, `drpc_public`, etc.)
* sanitize diagnostics to host/class only
* never persist raw URL or header values

---

## Part 2) Option Analysis Deep Dive

### 2.1 Option A: Full node mode (max independence)

Use case:

* users needing broad method support, heavy log scans, and stronger privacy

Fit in MFM:

* continue using same `namespace = "evm"` IO calls
* local full node becomes a preferred source entry in the same pool

Tradeoff:

* slower bootstrap and heavier resource footprint
* best for advanced/operator profile

### 2.2 Option B: Helios + robust upstream pool (default)

Use case:

* fastest path to a local endpoint with practical resilience

Fit in MFM:

* no state API changes
* implement routing/probing/failover in live transport

Tradeoff:

* upstream dependency and metadata privacy leakage remain

### 2.3 Option C: Portal/P2P light data (future)

Use case:

* long-term decentralization of light-client data retrieval

Fit in MFM:

* future collector backend under same `evm` boundary

Tradeoff:

* ecosystem maturity and coverage constraints today

---

## Part 3) Architecture Fit Deep Dive

### 3.1 End-to-end call path today

1. state uses `EvmIoClient::call(JsonRpcCall)`
2. `FactKey` derived from canonical JSON request hash
3. call routed to `namespace = "evm"` live transport factory
4. HTTP transport executes JSON-RPC
5. response recorded by live IO for replay

### 3.2 Where new logic belongs

Allowed:

* source pool model and probing in `crates/collectors/evm-jsonrpc-http`
* env/config parsing and transport wiring in `crates/app`
* service lifecycle and defaults in `nixfied/project/*`

Not allowed:

* embedding domain workflow logic in CLI/API transport layers
* adding ambient IO to state logic

### 3.3 Determinism model with hedging

Live mode:

* transport may issue multiple upstream requests for one logical IO call
* returns first acceptable success according to policy

Replay mode:

* same `FactKey` resolves to recorded payload id
* no network calls, no re-hedging

Determinism claim:

* run replay remains deterministic even if live winner endpoint was nondeterministic.

---

## Part 4) Helios Fast-Start Deep Dive

### 4.1 Startup-critical knobs

* checkpoint freshness and availability
* consensus endpoint responsiveness
* execution endpoint `eth_getProof` support
* persistent local data directory

### 4.2 Concrete MFM defaults/tuning targets

* prefer persistent Helios service scope for developer workflows where practical
* make explicit execution/consensus defaults visible and overrideable in Nixfied apps
* keep readiness probe gated on `eth_blockNumber` success
* keep a checkpoint strategy that avoids frequent recomputation in happy path

### 4.3 Probe contract before selecting an endpoint

Minimum probe set:

* `eth_chainId` returns expected chain id
* `eth_blockNumber` returns valid hex quantity
* `eth_getProof` probe for sources that must back Helios execution path

Failure policy:

* endpoint failing required probes is excluded from active candidate set
* emit safe diagnostics by source id and error class only

---

## Part 5) Open/Safe/Reliable Triangle Deep Dive

### 5.1 Explicit tradeoff matrix

| Mode | Open | Safety (verification) | Reliability | Privacy |
|---|---|---|---|---|
| Helios + public endpoints | High | Medium/High | Medium | Medium/Low |
| Helios + paid provider(s) | Low/Medium | Medium/High | High | Medium/Low |
| Full node local | Medium | High | High | High |

Interpretation:

* one default cannot maximize all axes
* product should expose mode selection, not hide tradeoffs

### 5.2 Recommended default policy

* default mode: Helios + curated public pool
* reliability mode: bring-your-own provider via env
* independence mode: full local node source first

---

## Part 6) Tiered Strategy Deep Dive

### 6.1 Tier 1 (curated public pool)

Inputs:

* static source IDs in config
* runtime URL resolution from env or service discovery

Behavior:

* startup probes + health score initialization
* primary selection by source class and score
* failover on retryable transport categories

### 6.2 Tier 2 (hedged reads + scoring)

Method classes:

* `read_light`: `eth_chainId`, `eth_blockNumber`, `eth_call`, `eth_getBalance`, etc.
* `read_heavy`: `eth_getLogs`, very large `eth_call`, tracing/debug endpoints
* `write_or_side_effect`: `eth_sendRawTransaction`, admin/debug mutating methods

Policy:

* hedge only `read_light`
* no hedge for heavy/write classes
* never execute write methods against multiple sources

Hedge algorithm (target behavior):

```rust
// Pseudocode for one logical IO call
match classify(method) {
    ReadLight if strategy == HedgedLight => {
        start(primary);
        wait(hedge_delay_ms);
        if !primary_completed {
            start(secondary);
        }
        return first_success_or_best_error();
    }
    _ => {
        for source in ordered_sources {
            if let Ok(resp) = call_once(source).await {
                return Ok(resp);
            }
            record_failure(source);
        }
        return Err(aggregate_error);
    }
}
```

### 6.3 Tier 3 (BYO provider)

Requirements:

* URLs and auth only from environment/runtime
* no persisted provider secrets
* stable source id still used in diagnostics/events

### 6.4 Tier 4 (full-node-first mode)

Behavior:

* source order prioritizes local full node, then Helios, then remote pool
* read/write split remains in effect

---

## Part 7) Configuration Model Deep Dive

### 7.1 Persisted model (safe only)

Persisted values should include:

* source IDs
* source kinds (`local_helios`, `local_fullnode`, `remote_public`, `remote_user`)
* preferred order and strategy selection
* method policy toggles (hedge on/off per class)

Persisted values should not include:

* full URLs
* bearer tokens/API keys
* headers

### 7.2 Runtime resolution model

Resolution examples:

* `helios_local` -> `http://127.0.0.1:$HELIOSRPC_PORT`
* `local_reth` -> `http://127.0.0.1:$RETHHTTP_PORT`
* `drpc_public` -> env/default URL at runtime
* `user_primary` -> env-provided URL/auth

### 7.3 Breaking-change policy

* source-id routing is mandatory for `namespace="evm"` reads
* remove per-request `rpc_url` overrides from EVM read transport
* remove single-source fallback config paths that bypass source discovery

---

## Part 8) API and Transport Shape Deep Dive

### 8.1 `crates/collectors/evm` (state-facing)

Near-term plan:

* keep `JsonRpcCall { method, params }` unchanged
* keep `fact_key_for_jsonrpc_call` unchanged

Reason:

* avoids fact-key churn across all existing ops
* avoids mass refactor across `evm-runtime` and ops crates

### 8.2 `crates/collectors/evm-jsonrpc-http` (live transport)

Expected additions:

* internal source registry + health scores
* method classifier
* failover and optional hedging executor
* optional `route_source_id` in transport request envelope (safe source ID only)
* remove `rpc_url` in request envelope for EVM read flow

### 8.3 `crates/app` wiring

Expected additions:

* runtime construction of multi-source EVM transport config
* env parsing for source URLs/auth at startup only
* keep namespace routing unchanged (`"evm"` route)

---

## Part 9) Error Model and No-Secrets Deep Dive

### 9.1 Error code strategy

Keep stable high-level classes:

* request parsing/config errors -> non-retryable
* transport status/timeouts/rate limit -> retryable where appropriate
* JSON-RPC error object -> retryability based on method/error class policy

Additions for pool behavior:

* `evm_source_unhealthy`
* `evm_no_healthy_source`
* `evm_hedge_exhausted`

### 9.2 Diagnostic detail policy

Allowed in details:

* source ID
* HTTP status class
* JSON-RPC error code
* truncated generic message

Disallowed in details:

* full request payloads containing credentials
* full URLs with query params
* authorization header values

### 9.3 Logging contract

* log to stderr only
* include correlation keys (`run_id`, `op_id`, `state_id`, `attempt`, `event_seq` when present)
* never log secret-bearing values

---

## Part 10) Method-Aware Policy Deep Dive

### 10.1 Lightweight read allowlist for hedging

Candidate methods:

* `eth_chainId`
* `eth_blockNumber`
* `eth_getBalance`
* `eth_call` (bounded payload/timeout policy)
* `eth_getTransactionReceipt`
* `eth_getBlockByNumber`

### 10.2 Heavy/no-hedge methods

* `eth_getLogs`
* `trace_*`
* `debug_*`
* methods known to trigger large payloads or long tail latencies

### 10.3 `eth_getLogs` special handling (Milestone B)

Policy outline:

* chunk block ranges by configurable max span
* sequential failover only
* optional per-source cache key by `(from,to,address,topics)`

---

## Part 11) Milestones with Exit Criteria

### 11.1 Milestone A (ship first)

Scope:

* source probing and scoring
* failover for retryable faults
* hedged light reads (2-source max)
* no secret leakage

Exit criteria:

* integration tests demonstrate successful fallback on timeout and 429
* replay determinism tests pass for hedged/failover calls
* no regression in existing `namespace = "evm"` state consumers

### 11.2 Milestone B

Scope:

* method-aware policy engine
* `eth_getLogs` chunking + no-hedge enforcement
* health decay/recovery tuning

Exit criteria:

* bounded behavior under high-volume log queries
* stable pass rates in full CI under fault injection

### 11.3 Milestone C (optional)

Scope:

* quorum only for tiny allowlist of trivially comparable methods

Exit criteria:

* measurable correctness benefit justifies latency/cost
* no regression in replay behavior or secret safety

---

## Part 12) Test Strategy Deep Dive

### 12.1 Unit tests (`crates/collectors/evm-jsonrpc-http`)

* method classifier correctness
* scoring/ranking updates under success/failure sequences
* hedge timing behavior (primary-only success, secondary wins, both fail)
* error sanitization and detail redaction

### 12.2 Integration tests (workspace)

* end-to-end fallback with one failing and one healthy endpoint
* rate-limit path maps to retryable categories and failover works
* replay run uses recorded fact without network access

### 12.3 Security regression tests

* ensure URL auth/query secrets never appear in:
  * event payloads
  * artifact payloads
  * error details
  * CLI/REST outputs

---

## Part 13) Rollout and Operational Deep Dive

### 13.1 Rollout plan

1. implement Milestone A as the default path in this branch
2. run full CI (`nix run .#ci -- --full --summary`) after each major commit slice
3. land Milestone B in the same branch before merge
4. merge only after the full target behavior is complete and green

### 13.2 Runtime telemetry to collect

* selected source ID per request class
* failover counts by error code
* hedge trigger rate and winner distribution
* p50/p95 latency by method class and source

### 13.3 Safe rollback

* single-source fallback mode remains available
* disable hedging via config without code revert
* keep source registry entries minimal (local first) when isolating incidents

---

## Part 14) Open Questions to Resolve Before Coding

* Should source IDs be globally fixed strings or namespaced per env/profile?
* Do we need separate pools for read and write calls from day one?
* Which methods enter the initial hedging allowlist, and how strict are payload-size guards?

These should be finalized before implementing Milestone A to avoid cross-crate churn.

---

## Part 15) Milestone A Implementation Checklist (File-by-File)

This is an execution checklist for delivering Milestone A incrementally with low regression risk.

### 15.1 Slice 0: Lock scope and policy inputs

Files:

* `DRAFT_RPC_PROBLEM_TT.md`
* `docs/redesign.md` (only if contract wording needs updates)

Checklist:

* [ ] Freeze Milestone A scope: failover + light hedging, no quorum.
* [ ] Freeze initial light-method allowlist and explicit no-hedge list.
* [ ] Freeze error-code additions and redaction policy.
* [ ] Freeze breaking changes: no `rpc_url` request override in EVM read transport.

Exit gate:

* [ ] Written decisions merged before transport implementation starts.

### 15.2 Slice 1: Extend transport config for source pools

Files:

* `crates/collectors/evm-jsonrpc-http/src/lib.rs`
* `crates/app/src/lib.rs`

Checklist:

* [ ] Add source-id based config model to transport (`sources`, ordering, strategy, hedge delay).
* [ ] Remove single-source `rpc_url`/`authorization` transport config from EVM read path.
* [ ] Add runtime-only source resolution path in app wiring.
* [ ] Keep namespace routing unchanged (`"evm"` in `make_engine_bundle`).

Acceptance tests:

* [ ] New test: transport creation fails fast when source registry is empty/invalid.
* [ ] New test: config with multiple source IDs builds transport successfully.

### 15.3 Slice 2: Add method classification and routing policy

Files:

* `crates/collectors/evm-jsonrpc-http/src/lib.rs`

Checklist:

* [ ] Introduce method classifier (`read_light`, `read_heavy`, `write_or_side_effect`).
* [ ] Add default policy: hedge only `read_light`, sequential failover for others.
* [ ] Add explicit deny-hedge set (`eth_getLogs`, `trace_*`, `debug_*`, writes).
* [ ] Ensure classification does not alter request payload hashing path.

Acceptance tests:

* [ ] Unit tests for classifier correctness across representative method set.
* [ ] Unit test proving write methods never trigger multi-source dispatch.

### 15.4 Slice 3: Implement failover executor

Files:

* `crates/collectors/evm-jsonrpc-http/src/lib.rs`

Checklist:

* [ ] Implement ordered source attempts for retryable transport failures.
* [ ] Respect retryability categories already used by transport (`timeout`, `429`, transient HTTP).
* [ ] Return first successful response.
* [ ] Return stable aggregate error when all candidates fail.
* [ ] Include safe source IDs in diagnostics; never include full URL/auth.

Acceptance tests:

* [ ] One source hard-fails, second succeeds -> call succeeds.
* [ ] First source returns 429, second succeeds -> call succeeds.
* [ ] All sources fail -> stable pool error code and retryability.

### 15.5 Slice 4: Implement light hedging (max 2 sources)

Files:

* `crates/collectors/evm-jsonrpc-http/src/lib.rs`

Checklist:

* [ ] Add hedged dispatch path for `read_light` only.
* [ ] Start primary, delay `hedge_delay_ms`, optionally start secondary.
* [ ] Return first acceptable success; cancel/ignore loser path safely.
* [ ] Keep hedge fanout bounded to 2 sources.
* [ ] Keep heavy/write methods on failover-only path.

Acceptance tests:

* [ ] Primary slow + secondary fast -> secondary winner returned.
* [ ] Primary fast -> secondary never started.
* [ ] Both fail -> stable aggregate error returned.

### 15.6 Slice 5: Health probing and scoring

Files:

* `crates/collectors/evm-jsonrpc-http/src/lib.rs`
* `crates/app/src/lib.rs`
* (optional config docs) `nixfied/project/dev.nix`

Checklist:

* [ ] Add startup probe hooks (`eth_chainId`, `eth_blockNumber`).
* [ ] Add optional capability probe (`eth_getProof`) for Helios-backing candidates.
* [ ] Add lightweight health score decay/recovery updates per call outcome.
* [ ] Prefer healthy local sources before remote ones when scores are comparable.

Acceptance tests:

* [ ] Unhealthy source is skipped until recovery condition is met.
* [ ] Healthy local source is preferred over equal-score remote candidate.

### 15.7 Slice 6: Secret-safety and diagnostics hardening

Files:

* `crates/collectors/evm-jsonrpc-http/src/lib.rs`
* `crates/app/src/lib.rs`
* `bin/cli/README.md`
* `bin/rest-api/README.md`

Checklist:

* [ ] Redact URL/auth from all error details.
* [ ] Ensure only source IDs and non-sensitive diagnostics are persisted/logged.
* [ ] Document runtime-only secret handling in CLI/REST docs.
* [ ] Preserve existing stable error surfaces where unchanged.

Acceptance tests:

* [ ] Regression tests assert no URL/auth appears in error `details`.
* [ ] Regression tests assert no secret-bearing fields appear in serialized outputs.

### 15.8 Slice 7: Remove per-request `rpc_url` override (breaking)

Files:

* `crates/collectors/evm-jsonrpc-http/src/lib.rs`
* `bin/rest-api/README.md`
* `bin/cli/README.md`

Checklist:

* [ ] Remove `rpc_url` from EVM read request schema in transport deserialization.
* [ ] Update tests and docs to use source-id routing only.
* [ ] Keep secret-redaction rules enforced in all error paths.

Acceptance tests:

* [ ] Legacy per-request override request shape returns stable invalid-request error.
* [ ] Invalid route-source requests return redacted diagnostics.

### 15.9 Slice 8: Integration coverage and replay invariants

Files:

* `tests/integration/tests/rest_api_run_control.rs`
* `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs`
* `tests/integration/tests/parity_portfolio_tracker_reth_snapshot.rs`
* (new recommended file) `tests/integration/tests/evm_rpc_pool_failover.rs`

Checklist:

* [ ] Add integration scenario with injected failing primary and healthy secondary.
* [ ] Add replay scenario proving no network call in replay path after live capture.
* [ ] Add 429 path coverage validating failover.
* [ ] Keep existing portfolio/REST parity tests green.

Acceptance tests:

* [ ] Integration suite passes with Milestone A enabled.
* [ ] No replay regressions (`MissingFactKey` behavior unchanged).

### 15.10 Slice 9: Nixfied and runtime defaults

Files:

* `nixfied/project/conf.nix`
* `nixfied/project/dev.nix`
* `nixfied/project/ci.nix`
* `nixfied/project/ci/scripts/steps/mainnet-portfolio-snapshot-helios.nix`

Checklist:

* [ ] Add optional env/config knobs for source IDs and hedging strategy.
* [ ] Remove reliance on single-source `MFM_EVM_RPC_URL` as the EVM read default path.
* [ ] Ensure `ci -- --full` runs Milestone A path by default.
* [ ] Preserve current Helios smoke and portfolio snapshot workflows.

Acceptance tests:

* [ ] `nix run .#ci -- --full --summary` passes with default settings.
* [ ] full CI includes Helios + failover path coverage.

### 15.11 Slice 10: Final contract/doc sweep

Files:

* `crates/collectors/evm-jsonrpc-http/README.md`
* `bin/cli/README.md`
* `bin/rest-api/README.md`
* `docs/architecture.md` (if boundary wording changes)

Checklist:

* [ ] Document source-id routing model and the breaking removal of per-request URL routing.
* [ ] Document failover/hedging high-level behavior and guardrails.
* [ ] Document operator-facing env vars and defaults.
* [ ] Confirm no doc claims conflict with actual runtime behavior.

Exit gate:

* [ ] Reviewer can follow docs to run Milestone A locally and in CI full mode.

### 15.12 Definition of Done (Milestone A)

* [ ] `namespace = "evm"` remains the only state-facing IO route.
* [ ] Existing `EvmIoClient`/`JsonRpcCall` call sites require no migration.
* [ ] Failover and light hedging work for read calls without write fanout.
* [ ] Replay determinism is preserved.
* [ ] No secret leakage in persisted/logged surfaces.
* [ ] `nix run .#ci -- --full --summary` passes (fail-fast, primary gate).

### 15.13 Suggested commit breakdown

1. Commit-1: Source registry config + classifier + schema break (`rpc_url` removal for read path).
2. Commit-2: Failover executor + unit tests.
3. Commit-3: Light hedging + unit tests.
4. Commit-4: Integration tests + replay/security regressions.
5. Commit-5: Nixfied/docs updates + full-CI gating.

This sequence keeps each commit reviewable while still delivering Milestone A end-to-end in one branch.

---

## Part 16) Reusable RPC Discovery Architecture (System-wide + CI)

This is the key to making RPC URL discovery fully reusable beyond one transport.

### 16.1 Design goal

One library-defined discovery flow should resolve EVM RPC sources for:

* CLI execution
* REST API execution
* integration tests
* CI health/preflight checks
* any future component that needs EVM RPC access

No component should hand-roll URL selection logic.

### 16.2 Recommended module boundary

Introduce a dedicated reusable library layer for discovery, for example:

* `crates/collectors/evm-rpc-discovery/src/lib.rs`

Alternative (if avoiding a new crate):

* `crates/collectors/evm-jsonrpc-http/src/discovery.rs`

Preferred is a dedicated crate so discovery is transport-agnostic and can be reused without pulling HTTP execution internals.

### 16.3 Core types (reusable contract)

```rust
pub struct DiscoveryInput {
    pub chain: String,              // "mainnet", "sepolia", "local"
    pub profile: String,            // "dev", "test", "ci", "prod"
    pub require_helios_compat: bool,
}

pub struct RpcSourceCandidate {
    pub source_id: String,          // safe ID only
    pub kind: SourceKindRef,        // local_helios/local_reth/remote_public/remote_user
    pub endpoint: RuntimeEndpoint,  // URL/auth runtime-only
}

pub struct DiscoveredRpcPool {
    pub ordered_sources: Vec<RpcSourceCandidate>,
    pub strategy: RoutingStrategy,
    pub policy: MethodPolicy,
}

pub struct DiscoveryReport {
    pub selected_ids: Vec<String>,  // persisted/logged safe surface
    pub rejected_ids: Vec<String>,
}
```

Key rule:

* `DiscoveredRpcPool` is runtime-only.
* `DiscoveryReport` is safe for logs/events.

### 16.4 Resolver pipeline

The discovery engine should run a deterministic, composable pipeline:

1. gather candidates from local services (`helios_local`, `local_reth`)
2. gather candidates from env-mapped remote sources (`drpc_public`, `user_primary`, etc.)
3. apply chain/profile filters
4. probe capabilities/health (`eth_chainId`, `eth_blockNumber`, optional `eth_getProof`)
5. score and order candidates
6. return runtime pool + safe report

### 16.5 Reuse points in this repo

* `crates/app/src/lib.rs`: use discovery output when constructing EVM transport factory
* `bin/cli`: no custom URL logic; route through app/discovery
* `bin/rest-api`: same as CLI
* CI: use a thin command/hook backed by the same discovery library before run steps

CI usage pattern:

* add a preflight command that calls discovery and prints only safe report data
* fail fast if no healthy required source is discovered
* then run `nix run .#ci -- --full --summary`

### 16.6 Why this is generalisable

* discovery concerns are separated from transport execution concerns
* source-id abstraction works for local services, public remotes, and BYO providers
* the same resolver can later power non-HTTP transports if needed
* policy and probing logic become testable once, reused everywhere

### 16.7 Testing contract for reuse

Unit tests (discovery crate/module):

* env/service candidate collection
* filter + scoring determinism
* probe failure behavior and rejection reasons

Integration tests:

* app startup path uses discovered pool
* CLI and REST path use identical source ordering under same env
* CI preflight and runtime see the same discovered source set

### 16.8 Commit impact (updated)

When implementing Milestone A with full reuse, include discovery extraction early:

1. Commit-1: introduce discovery layer + source-id-only schema break
2. Commit-2: wire app and transport to discovery output
3. Commit-3+: failover/hedging/probing/tests/docs
