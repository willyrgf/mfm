# Typed Bitcoin Runtime Config

Status: typed transport runbook for Bitcoin-backed portfolio workflows.

Bitcoin RPC endpoints, Basic-auth values, and scan deadlines are process-local live inputs. They
are not semantic run authority and must not be persisted in specs, events, artifacts, public
outputs, fixtures, or replay inputs.

Normative architecture references:

- `docs/design.md`
- `docs/architecture.md`
- `docs/persisted-public-surfaces.md`

## Runtime Config File

Live CLI start accepts `--runtime-config <PATH>`. A resume needs it only while verified history has
a pending Bitcoin live-source node. The REST server accepts the same explicit flag. There is no
environment-selected config path. Read-only commands, replay, and REST startup do not load this
file.

Example TOML:

```toml
[bitcoin.routes.public-bitcoin-core]
scan_timeout_seconds = 30
rpc_url = { env = "MFM_BITCOIN_RPC_URL" }
rpc_user = { env = "MFM_BITCOIN_RPC_USER" }
rpc_password = { env = "MFM_BITCOIN_RPC_PASSWORD" }
```

The route key is the semantic `BitcoinSourceIdentity`, not an endpoint name, URL, credential id, or
routing policy. `scan_timeout_seconds` is required and must be in `1..=86_400`. Basic-auth user and
password are either both absent or both present.

Each value source is exactly one of `{ direct = "..." }`, `{ env = "NAME" }`,
`{ file = "/path" }`, or `{ file_env = "NAME" }`. Passwords cannot use `direct`; their source must
be indirect even in an unselected route.

## Source-Bound Aggregate Reads

App assembly creates one private routed session set. It selects an endpoint-bound
`BitcoinRpcSession` by the certified semantic source identity, validates its exact
`BitcoinSourceBinding` without network IO, and registers that routed set once for
`BitcoinBalanceCollectionReadCapability`.

`rust-bitcoin` is the primitive authority for canonical address parsing, network compatibility,
script derivation, block/transaction hashes, outpoints, and amounts. Supported Bitcoin Core chain
tags are `main`, `test`, `testnet4`, `signet`, and `regtest`. Test-family address encodings may be
shared; the checked `getblockchaininfo.chain` value establishes the actual chain.

Each `BitcoinBalanceCollectionRequest` carries one exact semantic binding and between 1 and 1,024
canonical addresses in strict UTF-8 order. A successful attempt performs exactly:

1. `getblockchaininfo`, requiring the configured chain and `initialblockdownload == false`;
2. one `scantxoutset start` containing every `addr(address)` descriptor; and
3. `getblockhash(scan.height)`, requiring the hash to equal the scan anchor.

Tip advancement is valid. A changed hash at the scan height is a reorganization and fails the
attempt. MFM issues no additional RPC or scan-control requests; it has no process-local scan
coordinator and no hidden transport retry.

The selected deadline is the overall `scantxoutset start` timeout. Timeout or cancellation drops
MFM's HTTP request but does not abort Bitcoin Core's global scan. A later full-attempt retry may
receive the exact retriable scan-busy error until Core finishes.

## Strict Reduction

The transport accepts the forward-additive Bitcoin Core 28 response shape but strictly validates
every consumed field. It requires JSON-RPC 2.0, exact request ids, exactly one result/error arm,
bounded bodies, unique object members at every nesting level, mandatory scan fields, checked hashes
and outpoints, bounded scripts/descriptors, known scriptPubKeys, and UTXO heights no greater than
the scan height.

Raw JSON amount tokens are converted directly to satoshis with at most eight fractional digits and
no sign, exponent, float intermediary, overflow, or value above Bitcoin `MAX_MONEY`. Address
balances and the total are checked sums; `total_amount` must equal the observed sum. Addresses with
no UTXO remain explicit zero balances. Provider bodies/messages, endpoints, and credentials are
discarded at the transport boundary.

`CollectBitcoinBalancesState` reduces that one source-bound observation to an ordered non-empty
`bitcoin.balance_snapshot` fact batch and a minimal `BitcoinBalanceCollectionReceipt`. Runtime
settles the facts, receipt, retained evidence, and attempt completion in one atomic append.

## Ingress And Replay

No runtime file, no Bitcoin family, or no selected semantic route yields `RuntimeConfigRequired`
with a closed missing/route diagnostic. An unreadable, malformed, or semantically invalid supplied
file yields `RuntimeConfigInvalid`; it is never presented as a missing route. Admission aggregates
and deduplicates missing Bitcoin and EVM routes before `RunAdmitted`. Resume repeats that check only
for nonterminal live-source nodes.

Replay uses the certified spec, append-only run stream, and retained typed artifacts. It reruns the
same reducer and verifies the exact ordered fact batch and receipt. It must not open an RPC
connection, consult runtime config, or repeat live IO.
