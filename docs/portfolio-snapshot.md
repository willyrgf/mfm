# Portfolio Snapshot

MFM exposes one portfolio workflow:

```text
mfm.portfolio/snapshot@1
```

Start it with one stable target:

```sh
mfm_cli run start mfm.portfolio/snapshot@1 acme/primary
```

Import a `PortfolioConfig` through setup first. Its intrinsic `portfolio_id` becomes the target,
so `portfolio_id = "acme/primary"` is selected with `acme/primary` on CLI or with
`{"entry_point":"mfm.portfolio/snapshot@1","target":"acme/primary"}` over REST. Only that exact
versioned snapshot objective is published.

For a runnable token-only setup, import
[`examples/setup/portfolio-erc20.toml`](../examples/setup/portfolio-erc20.toml) and use the EVM
route in [`examples/configs/runtime-ethereum-mainnet.toml`](../examples/configs/runtime-ethereum-mainnet.toml)
with `MFM_ETHEREUM_MAINNET_RPC_URL` set. The setup deliberately has an ERC-20 contract address but
no authored token decimals, endpoint, runtime binding, or read bound; EVM collection observes
decimals and balances at the retained anchor.

At admission, the app loads the target's current configuration, verifies and normalizes it, records
the target/schema/digest evidence in `RunAdmitted`, and gives the concrete `PortfolioConfig` to the
snapshot operation. Admission also enforces the configured network, wallet, symbol,
wallet-to-symbol, and per-EVM-network source limits before graph expansion.

The operation derives only explicit wallet-to-symbol demand. Bitcoin collection resolves one
shared anchor per demanded network and emits checked network receipts. Each EVM network uses one
external-read state and one atomic publication state. One checked session resolves latest once,
reads deduplicated token metadata and every native/ERC-20 balance at the exact hash, and rechecks
that hash by block number. The bounded concurrent scheduler issues each chunk in certified plan
order and preserves that order independently of response completion order.

The publication attempt records the complete `evm.balance_snapshot` fact batch and a checked
`EvmBalanceCollectionReceipt` together. That receipt contains the network/chain, exact anchor,
sorted sources, and verified fact content identities, but no duplicate balance response material.
The typed Bitcoin and EVM receipt vectors flow directly into `SelectHoldingsState`; their input
edges are the managed-write completion barrier. Selection issues all family queries over one store
snapshot, rehydrates every candidate response, rederives fact identity, and admits only the exact
receipt-authorized content. Byte-identical append occurrences are equivalent; same-subject facts
with different response content are filtered before ordering. Assembly consumes only these
store-reread observations, including for token-only and all-EVM portfolios.

Collection plans, evidence, receipts, and fact subjects reuse the configured
`NormalizedEvmAddress` plus `HoldingSourceConfig` values directly. Session evidence and the shared
checked `EvmBlockAnchor` come from the EVM capability contract; the block number retains the full
U256 range as a canonical decimal string in persisted values.

A wallet with no configured symbols is retained with empty observations and zero quote totals, but
creates no collection work or network pin; the aggregate remains valid only when another explicit
wallet-to-symbol edge exists.

Every persisted `WalletSnapshot` retains the checked `WalletSubject` algebra directly rather than a
parallel address string. EVM execution pins use the shared checked `EvmBlockAnchor` nested under a
non-zero chain id; transaction receipts, logs, validation evidence, collection evidence, and public
portfolio pins reuse the same number/hash value.

The root returns `PortfolioPublicOutputs` with exactly `snapshot` and `report`. It preserves zero
holdings, exposes direct quote totals, and does not expose receipt entries, source keys, fact
identities, artifact references, provider evidence, scan bounds, or runtime routes.
Both public values emit `schema_version: 1`; snapshot and report version selection is not a
request or certified-state policy.

After admission current configuration is not run authority. Resume, replay, status, stream
inspection, and public-output rendering use the certified spec and retained evidence. Replay
recomputes EVM facts and receipts, verifies that selection consumed the exact family vectors, and
replays selection from retained query and response evidence. Live capability routes remain
process-local runtime configuration. Evidence-only replay does not load them; a live resume loads
them only when verified unfinished external nodes still require a live capability.
