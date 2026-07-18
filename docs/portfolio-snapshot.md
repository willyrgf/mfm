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
shared anchor per demanded network, proves an exact Bitcoin receipt, and selects only facts whose
content and anchor match that receipt. Each EVM network instead uses one external-read state and one
atomic publication state. One checked session resolves latest once, reads deduplicated token
metadata and every native/ERC-20 balance at the exact hash, and rechecks that hash by block number.
The publication attempt records the complete `portfolio.evm_balance_snapshot` fact batch and a
direct network snapshot together. Assembly consumes that snapshot directly; token-only runs make no
fact-index request. Collection plans, evidence, snapshots, and fact subjects reuse the configured
`NormalizedEvmAddress` plus `HoldingSourceConfig` values directly. Session evidence and block
anchors are the shared EVM capability values, and block numbers retain their full U256 range as
canonical decimal strings.

A wallet with no configured symbols is retained with empty observations and zero quote totals, but
creates no collection work or network pin; the aggregate remains valid only when another explicit
wallet-to-symbol edge exists.

The root returns `PortfolioPublicOutputs` with exactly `snapshot` and `report`. It preserves zero
holdings, exposes direct quote totals, and does not expose receipt entries, source keys, fact
identities, artifact references, provider evidence, scan bounds, or runtime routes.
Both public values emit `schema_version: 1`; snapshot and report version selection is not a
request or certified-state policy.

After admission current configuration is not run authority. Resume, replay, status, stream inspection, and
public-output rendering use the certified spec and retained evidence. Live capability routes remain
process-local runtime configuration. Evidence-only replay does not load them; a live resume loads
them only when verified unfinished external nodes still require a live capability.
