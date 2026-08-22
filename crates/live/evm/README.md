# mfm-evm-live

Direct provider registration for the three surviving EVM Read capabilities. Each callback captures
one domain-owned `EvmPhysicalTarget` and opaque provider handle, checks exact intent chain/route
before IO, bounds request encoding, and returns typed evidence or `AdapterError`.

`EvmAdapterLocator` is one bounded private HTTP(S) URL. It implements neither `Debug`, `Display`,
nor serialization. Non-HTTP schemes, fragments, and control characters are rejected.

`JsonRpcEvmProvider` is the production provider behind that handle. It owns one endpoint URL, one
10-second per-request deadline, a 512 KiB response bound, and the six frozen-wire calls the domain's
Read subjects require. Its stock Reqwest client uses no ambient proxy, redirect, referer propagation,
or automatic retry. HTTPS uses Reqwest's compiled WebPKI roots; local managed tests connect directly
to Reth over HTTP. MFM owns no TLS-root configuration or transport-security abstraction. The provider
decodes request bytes with the domain's checked `EvmReadIntent` deserializer and declares no serde
mirror of that wire.

The per-operation observation contract every `EvmProvider` owes the domain is on the `EvmProvider`
trait rustdoc. Its one trap: `confirm-balance-anchor` re-observes the committed block its intent
names and never the head. The domain compares that result to the anchor it pinned before the
balance reads, so a head read would report ordinary chain progression as a reorg and fail every
collection on a chain that produces blocks.

Transport, status, bound, and undecodable-ingress failures are `Unavailable`; a JSON-RPC error
object and an `eth_call` result of exactly `"0x"` are definite `SafeFailure`; an undecodable or
operation-mismatched intent and a malformed address are `Internal` before any IO. The provider never
produces `IntegrityBlocked`: only authenticated external evidence can durably represent an integrity
block. The RPC URL is a credential-bearing handle, so the provider implements no `Debug` and no
error, log, or value names it.

This is the ONLY crate allowed to depend on `alloy-*`. Custody is option-invariant and the frozen
wire codecs (U256/hex, ABI, RLP, keccak) live in alloy's stable pure-Rust core; the provider half of
alloy churns and would drag a TLS/cmake build into the pinned Nix sandbox. `alloy-provider`,
`alloy-network`, `alloy-rpc-types`, and `alloy-signer` are excluded. Verify with
`cargo tree -e features -p mfm-evm-live`.

The crate owns no State registration, planner, binding wrapper, live assembly contribution, call ID,
response echo, signer, nonce, broadcast, or transaction-submission path.

Capability injection is not live registration: it is deterministic domain-owned Program topology
applied before Runtime sees the Program. This crate never invokes Operation or injection hooks.
