# Problem: Helios Is Not Reliable On Current Open Mainnet RPCs

Date: 2026-03-14

## Context

Primary repro:

```sh
env NIX_ENV=0 HELIOS_NETWORK=mainnet SERVICE_REUSE_POLICY=same-slot \
  nix run .#mfm::portfolio::snapshot -- /tmp/portfolio-request.json
```

This repo now uses framework-managed Postgres and Helios for that path. The local wiring is no longer the main blocker.

## What Works

- The canonical request file is valid.
- The snapshot wrapper now reaches:
  - Postgres reuse/start
  - database setup
  - Helios checkpoint derivation
  - Helios process launch
- Helios package/build issues were reduced:
  - source build via `cargoLock`
  - `0001-disable-reqwest-hickory-dns.patch`
  - `0002-limit-light-client-updates-request.patch`

## Actual Problem

Anonymous public mainnet RPCs are not reliable enough for Helios readiness in this workflow.

Observed failures:

- Consensus bootstrap source `https://www.lightclientdata.org` frequently returned `503`.
- Lodestar fallback `https://lodestar-mainnet.chainsafe.io` was good enough for checkpoint derivation and direct bootstrap curls, but Helios bootstrap still failed transiently in some runs.
- Execution RPCs answered JSON-RPC requests, but Helios often stalled in an "almost synced" state and never returned a usable `eth_blockNumber`.

Typical runtime symptoms:

- `{"code":1,"message":"out of sync: ... seconds behind"}`
- `eth_syncing` stuck at:
  - `currentBlock = 0`
  - or a block that stayed a few minutes behind head
- Helios log:
  - `inconsistent block history detected: clearing cache`

## Concrete Findings

### Consensus side

- `lightclientdata` was unreliable during testing.
- Lodestar was the best available public fallback for:
  - `/eth/v1/beacon/headers/finalized`
  - `/eth/v1/beacon/light_client/bootstrap/<checkpoint>`

### Execution side

Tested public execution RPCs:

- `https://eth.drpc.org`
- `https://ethereum-rpc.publicnode.com`
- `https://eth.llamarpc.com`
- `https://rpc.flashbots.net`

Results:

- `eth.drpc.org`
  - worst behavior under Helios
  - Helios stayed stuck and repeatedly logged block-cache inconsistency warnings
- `ethereum-rpc.publicnode.com`
  - best result of the open RPCs tested
  - a fresh-data-dir Helios smoke test on port `8558` did return a real `eth_blockNumber`
  - but the full snapshot path still drifted into `out of sync`
- `eth.llamarpc.com`
  - Helios stabilized around a head roughly five minutes old
- `rpc.flashbots.net`
  - similar stale-head behavior

## Important Conclusion

The remaining blocker is not the portfolio snapshot contract and not the basic Nix/service wiring.

The blocker is operational:

- public consensus endpoints are flaky
- public execution endpoints are inconsistent enough to break Helios readiness or keep it behind head

This means the current "free public RPC" setup is not a dependable base for deterministic local portfolio snapshots.

## What A Definitive Solution Must Achieve

Any real fix should provide:

- a stable mainnet execution RPC for Helios
- a stable mainnet consensus/light-client source
- predictable readiness behavior for `mfm::portfolio::snapshot`
- low enough variance that `eth_blockNumber` becomes usable without repeated manual retries

## Useful Grep Strings

- `out of sync:`
- `inconsistent block history detected: clearing cache`
- `could not fetch bootstrap`
- `No partialUpdate available for period`
- `The requested URL returned error: 503`
