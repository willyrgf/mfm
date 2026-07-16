# Portfolio Snapshot

MFM exposes one portfolio workflow:

```text
mfm.portfolio/snapshot@1
```

Start it with one exact catalog reference:

```json
{
  "portfolio": {
    "name": "acme/primary",
    "digest": "content:sha256-jcs-v1:..."
  }
}
```

Import a `PortfolioConfig` through setup first, then use the returned `name` and `digest` with
`mfm_cli run start --entry-point mfm.portfolio/snapshot@1 --request request.json` or the matching
REST start request. There is no standalone collector, report-only, collect/reuse, latest, alias, or
contract-workflow entry point.

For a runnable token-only setup, import
[`examples/setup/portfolio-erc20.toml`](../examples/setup/portfolio-erc20.toml) and use the EVM
route in [`examples/configs/runtime-ethereum-mainnet.toml`](../examples/configs/runtime-ethereum-mainnet.toml)
with `MFM_ETHEREUM_MAINNET_RPC_URL` set. The setup deliberately has an ERC-20 contract address but
no authored token decimals, endpoint, source policy, or read bound; the collector observes decimals
and balances at the retained anchor.

At admission, the app verifies and normalizes the exact catalog value, records its identity in
`RunAdmitted`, and gives the concrete `PortfolioConfig` to the snapshot operation. The operation
derives only explicit wallet-to-symbol demand, resolves one shared anchor per demanded network,
collects BTC native, EVM native, and ERC-20 sources as needed, proves an exact collection receipt,
and selects facts only when their content and anchor match that receipt.
A wallet with no configured symbols is retained with empty observations and zero quote totals, but
creates no collector work or network pin; the aggregate remains valid only when another explicit
wallet-to-symbol edge exists.

The root returns `PortfolioPublicOutputs` with exactly `snapshot` and `report`. It preserves zero
holdings, exposes direct quote totals, and does not expose receipt entries, source keys, fact
identities, artifact references, provider evidence, scan bounds, or runtime routes.
Both public values emit `schema_version: 1`; snapshot and report version selection is not a
request or certified-state policy.

After admission the catalog is not run authority. Resume, replay, status, stream inspection, and
public-output rendering use the certified spec and retained evidence. Live capability routes remain
process-local runtime configuration. Evidence-only replay does not load them; a live resume loads
them only when verified unfinished external nodes still require a live capability.
