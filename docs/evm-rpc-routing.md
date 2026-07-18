# EVM Runtime Sessions

Status: typed transport runbook for EVM-backed reads, transactions, and signing resources.

RPC endpoints, authorization, signer bindings, and keystore paths are live runtime inputs. They are
not semantic run authority and must not be persisted in manifests, events, artifacts, public
outputs, fixtures, or replay inputs.

## Direct routes

Live CLI start accepts `--runtime-config <PATH>`. Resume needs it only while verified history still
has a pending EVM live node. CLI and REST use `MFM_RUNTIME_CONFIG_FILE` when no explicit path is
provided. Evidence-only commands do not load this file.

Each semantic EVM network has exactly one direct route:

```toml
[evm.routes.reth-dev]
source_ref = "reth-local"
rpc_url = "http://127.0.0.1:8545"

[evm.routes.ethereum-mainnet]
source_ref = "mainnet-primary"
rpc_url_file = "/run/mfm/mainnet-rpc-url"
auth_header_file = "/run/mfm/mainnet-auth-header"

[keystores.default]
keystore_path = "/run/mfm/deployer.keystore"
unlock_file = "/run/mfm/deployer.password"

[signers.deployer]
provider = "keystore"
keystore_ref = "default"
entry_id = "00000000-0000-0000-0000-000000000000"
```

JSON with the same shape is accepted. Expected chain id comes from certified workflow semantics;
runtime routes cannot override it. There are no source registries, policy ids, ordered candidates,
or fallback rotation.

Live reads load only the requested route. Transaction assembly loads that route plus the exact
referenced signer and keystore. Unrelated malformed EVM routes or signer entries do not block the
selected resource, while malformed selected material fails closed.

## Source-bound sessions

The EVM state-facing capability surface has two coherent authorities:

- `EvmReadCapability` / `EvmReadSession` for block, balance, code, and call reads;
- `EvmTransactionCapability` / `EvmTransactionSession` for pending nonce, fee inputs, estimation,
  submission, transaction observation, receipt observation, and confirmation blocks.

App assembly derives an `EvmNetworkBinding` from certified `network_id` and non-zero chain id, loads
its direct route, and asynchronously binds `EvmJsonRpcSession`. Binding constructs a bounded HTTP
client and calls `eth_chainId` once. A mismatch fails before a session is returned. The resulting
session is fixed to one endpoint and one redacted `source_ref` for the whole attempt; methods do not
reselect, reprobe, or fail over.

The bind-time `EvmSessionEvidence` contains only semantic network id, verified chain id, source ref,
and the certified session implementation id. Endpoints and credentials never enter capability
requests or evidence.

The HTTP transport has a 10-second connection timeout, a 30-second request timeout, and a one-MiB
response limit. It requires JSON-RPC version `2.0`, exact response id `1`, and exactly one of
`result` or `error`. Quantities, hashes, addresses, bytes, transactions, receipts, and complete logs
are decoded into checked Alloy-backed types. Submission succeeds only when the provider hash equals
the local hash of the submitted bytes.

## Exact anchors

Portfolio reads first resolve a number/hash anchor. Balance and ERC-20 calls use the anchor hash as
an EIP-1898 selector with `requireCanonical: true`. After the anchored reads, the same session reads
the anchor by number and requires the returned hash to equal the original hash. Asking for the old
hash again is not a canonicality check and is forbidden.

The current collector graph retains destination, exact calldata, raw return bytes, the final
number/hash result, and one session evidence value. Replay reconstructs those typed values and runs
the same deterministic state reducers without runtime config or network access.

## Signing

`keystore tx-sign` selects one exact `[signers]` entry and its referenced `[keystores]` profile. It
does not require an EVM route. The app calls the same canonical Alloy EIP-1559 signing service used
by reusable mutation code. The deterministic RFC 6979 recoverable low-s profile is explicit, the
provider is called once, and the expected sender and local transaction hash are verified.

Keystore paths, unlock files, passwords, private keys, mnemonics, signatures, signed envelopes, and
raw transactions remain runtime-only. The explicit `tx-sign --out` file is the sole user-selected
bearer boundary and is written mode 0600; stdout and stderr expose only redacted metadata.

## Failure and replay rules

No runtime file, no `evm` family, or no selected route is a missing provider configuration. An
unreadable, malformed, or invalid selected route is invalid provider configuration. Diagnostics may
carry the certified network, expected chain id, source ref, closed operation id, and reviewed
numeric codes, but never endpoints, authorization, provider messages, response bodies, or paths.

Replay uses the certified spec, append-only stream, retained typed artifacts, and replay verifiers.
It must not open an RPC connection, resolve a current route, or construct a signer.

For the complete transaction state, lane, preparation, recovery, receipt, and finality contract,
see [EVM Transactions](evm-transactions.md).

Contributor ownership:

- `mfm-runtime-config` parses and selectively resolves routes and signers;
- app assembly selects runtime resources and binds sessions;
- `mfm-transports-evm` owns bounded JSON-RPC and typed protocol decoding;
- `mfm-adapters-evm` sequences state intent over a bound session and records redacted evidence;
- states own deterministic validation and reduction; binaries only pass paths and render results.
