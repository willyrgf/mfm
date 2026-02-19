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

Keep `rpc_url` request override only as a temporary compatibility path, then phase it out.

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
