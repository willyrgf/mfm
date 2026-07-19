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

Selected RPC URLs must use `http` or `https`, must not contain userinfo, and selected authorization
values must parse as one HTTP header value. Selective route resolution enforces all three rules on
the blocking config worker before admission, before any transport or signer is constructed.
Missing selected routes surface `RuntimeConfigRequired`; unreadable or malformed selected routes,
including unsupported schemes and invalid authorization values, surface `RuntimeConfigInvalid`.
Both retain only closed semantic EVM diagnostics and leave the run stream empty.

## Source-bound sessions

The EVM state-facing capability surface has two coherent authorities:

- `EvmReadCapability` / `EvmReadSession` for block, balance, code, and call reads;
- `EvmTransactionCapability` / `EvmTransactionSession` for pending nonce, fee inputs, estimation,
  submission, transaction observation, receipt observation, and confirmation blocks.

App assembly derives an `EvmNetworkBinding` from certified `network_id` and non-zero chain id. One
shared asynchronous route validator serves balance collection and exact-anchor contract validation;
it performs selective runtime-config file loading and parsing on a blocking worker before admission.
Only after validation may execution asynchronously bind `EvmJsonRpcSession` through one
process-shared `EvmJsonRpcTransport`. Binding calls `eth_chainId` once. A mismatch fails before a
session is returned. The resulting session is fixed to one endpoint and one redacted `source_ref`
for the whole attempt; methods do not reselect, reprobe, or fail over.

The bind-time `EvmSessionEvidence` contains only semantic network id, verified chain id, source ref,
and the certified session implementation id. The source ref is audit provenance for the route used
by that attempt, not semantic policy: it may change across attempts or resume, and replay never
resolves it against current runtime routing. Endpoints and credentials never enter capability
requests or evidence.

The HTTP transport has one shared connection pool, a global bound of 64 in-flight exchanges, a
process-local bound of 16 in-flight exchanges for each `source_ref`, a 10-second connection timeout,
a 30-second request timeout, and a one-MiB outer response limit. Deployed-code and contract-call
responses instead derive a smaller body limit from the 128-KiB code bound or the call's explicit
decoded-result bound plus a fixed JSON-RPC envelope allowance. Both content length and streamed
chunks are checked against that method limit before JSON decoding. Redirect following and
reqwest's implicit retry policy are disabled. Every capability call therefore owns exactly one
exchange with its fixed endpoint; every later transaction broadcast is an explicit adapter
recovery decision. The transport requires JSON-RPC version `2.0`, exact response id `1`, and exactly
one of `result` or `error`. Quantities, hashes, addresses, bytes, transactions, receipts, and
complete logs are decoded into checked Alloy-backed types. Submission succeeds only when the
provider hash equals the local hash of the submitted bytes.

## Exact anchors

Portfolio reads first resolve a number/hash anchor. Balance and ERC-20 calls use the anchor hash as
an EIP-1898 selector with `requireCanonical: true`. After the anchored reads, the same session reads
the anchor by number and requires the returned hash to equal the original hash. Asking for the old
hash again is not a canonicality check and is forbidden.

For each `EvmBalanceCollectionOperation` call, `CollectEvmBalancesState` resolves latest once,
deduplicates ERC-20 metadata by contract, reads every sorted native/token source at that exact hash,
and performs one final number-to-hash check. Metadata and balance reads have a hard concurrency
limit of 16. Its single aggregate evidence value retains the checked session, requests/results,
and final canonicality observation. The state-owned reducer enforces exact order and coverage for
both live execution and replay.

`RecordEvmBalanceFactsState` then records one `evm.balance_snapshot` fact per source and a checked
`EvmBalanceCollectionReceipt` in one atomic managed-write attempt. The receipt carries the exact
anchor, sorted sources, and verified content identities without copying balance response material.
The live runner, atomic publication, and evidence-only replay live in `mfm-adapters-evm`.
`PortfolioReportOperation` receives the typed family receipt vectors and passes the same structured
binding to selection, which queries same-run facts through the shared BTC/EVM fact-index snapshot,
rehydrates response artifacts, and rederives exact content identity before assembly. All-EVM
portfolios use this same store path. EVM replay reconstructs collection and publication in the EVM
adapter; portfolio replay reconstructs only receipt-pinned selection and report projection. Neither
requires runtime config or network access.

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

Read-only and pre-mutation availability/resource failures block the attempt so process-local routing
can be repaired. HTTP 408, 425, 429, 500, 502, 503, 504, and 507 are availability/resource failures;
other non-success HTTP statuses are deterministic rejections. JSON-RPC internal error `-32603` and
Ethereum resource errors `-32001`, `-32002`, and `-32005` are availability/resource failures; other
numeric JSON-RPC errors are deterministic rejections. A missing or wrongly typed numeric field is
a response-contract violation. Deterministic request and response-contract violations remain
terminal. After transaction submission is durably possible, every provider/session failure blocks:
it cannot prove the transaction failed and cannot authorize rebroadcast. Classification uses typed
capability variants and closed numeric diagnostics only, never provider messages.

Replay uses the certified spec, append-only stream, retained typed artifacts, and replay verifiers.
It must not open an RPC connection, resolve a current route, or construct a signer.

For the complete transaction state, lane, preparation, recovery, receipt, and finality contract,
see [EVM Transactions](evm-transactions.md).

Contributor ownership:

- `mfm-runtime-config` parses and selectively resolves routes and signers;
- app assembly selects runtime resources and binds sessions;
- `mfm-transports-evm` owns bounded JSON-RPC and typed protocol decoding;
- `mfm-adapters-evm` owns reusable balance collection/publication, transaction, validation, and
  evidence-only replay bindings;
- `mfm-adapters-portfolio` owns receipt-pinned fact-index selection and snapshot/report projection
  bindings only;
- portfolio and reusable EVM states own their deterministic validation/reduction semantics;
  binaries only pass paths and render results.

The exact package, state-kind, and app-entry-point inventory is recorded in
[Current EVM Inventory](architecture.md#current-evm-inventory).
