# RFC: portfolio snapshot intent compiler and public operation cutover

Status: accepted for implementation planning

Date: 2026-07-15

## Summary

MFM must expose durable user objectives, not its internal graph decomposition. A user who has
published one complete portfolio configuration should be able to start one portfolio workflow
without separately selecting collectors, authoring derived collector policy, or deciding whether
the portfolio requires Bitcoin native, EVM native, or ERC-20 reads.

This RFC replaces the current public operation model with one entry point:

```text
mfm.portfolio/snapshot@1
```

The request contains one exact catalog reference to `PortfolioConfig`. The portfolio snapshot
operation validates that complete value, deterministically derives every explicitly configured
wallet holding, groups the required source observations by network and resource kind, composes
typed internal collector operations and states, proves exact collection completeness, and then
runs the fact-backed report graph.

ERC-20 collection is part of the first complete implementation. EVM native and ERC-20 reads on the
same network share one hash-bound joint tip. Token `decimals()` is observed once per token at that
tip; `balanceOf(account)` is observed for each required token holding. Native and ERC-20 behavior
remain separate typed operations, states, facts, adapters, and replay contracts.

This cutover also removes every EVM contract entry point and the lifecycle operation crate. The
deploy, configure, and validate states remain reusable executable domain-state primitives, including
their adapters, side-effect recovery, and replay verification. Operation wrappers and continuation
imports are not retained without a current public objective.

This is a destructive pre-production replacement. There are no legacy IDs, aliases, compatibility
decoders, deprecated setup kinds, facade operations, or replay shims.

This RFC records the design decision only. The implementation remains follow-up work.

## Relationship to `RFC_CONFIG.md`

This RFC corrects a public-surface leak in the already implemented `RFC_CONFIG.md` design.

It supersedes `RFC_CONFIG.md` only for:

- the public entry-point inventory;
- the closed setup-value inventory;
- portfolio collector/report composition and request policy;
- the claim that `portfolio_snapshot` must remain report-only;
- independently launchable BTC/EVM collector paths;
- EVM contract operation and lifecycle-entry-config ownership.

It preserves the rest of the configuration contract:

- setup import is the only TOML semantic-authoring surface;
- run-start requests are strict JSON and use exact catalog name-plus-digest references;
- catalog resolution and relational validation finish before certification and `RunAdmitted`;
- certified specs contain concrete typed config, not `CatalogRef<T>`;
- the catalog is never resume, replay, status, stream, or public-output authority;
- runtime routing and signer material remain process-local and never enter semantic config;
- structured hashed material contains no floats;
- secrets do not enter configs, specs, events, facts, artifacts, outputs, errors, or fixtures;
- there is no compatibility, latest-version, fallback, or dual-write path.

`RFC_CONFIG.md`, `docs/design.md`, `docs/architecture.md`, `docs/persisted-public-surfaces.md`,
`docs/evm-rpc-routing.md`, CLI/REST documentation, and portfolio and contract runbooks must be
updated in the implementation change so they do not continue to state the superseded decisions.

## Problem situation

### The public surface mirrors implementation topology

The application currently publishes eight entry points:

- a report-only portfolio snapshot;
- a topology-named collect-then-report workflow;
- standalone BTC and EVM native-balance collectors;
- deploy-only, configure-only, validate-only, and full-lifecycle EVM contract workflows.

These paths expose internal fact production and lifecycle phases as independently launchable user
goals. Callers must understand which low-level operations to start, which values to configure, and
which ordering produces a complete result.

That is the wrong abstraction boundary. An operation is a deterministic typed planning unit. It can
be reusable and Rust-public without being an external entry point. External launchability needs a
stricter rule.

### The composed portfolio config has competing sources of truth

The current composed request resolves one `PortfolioConfig` and also accepts BTC/EVM policy from the
caller. Its completed operation config stores the portfolio plus independently mutable derived BTC
and EVM collector vectors.

The portfolio says which holdings exist. The child vectors say which holdings will be collected.
Nothing in their types makes those two descriptions identical. A direct caller can therefore
construct a valid-looking certified graph that omits a configured holding, contains an unexpected
holding, or uses mismatched network or quantity policy.

Derived values must not become a second authored or externally constructible authority.

### Native-only composition is not the intended product

The current composer is organized around Bitcoin native and EVM native batches. The portfolio model
already advertises ERC-20 and protocol-position readers, but portfolio selection rejects those
holdings because no complete collector, fact, adapter, and report projection exists.

This is a partial public affordance: configuration can describe a portfolio the only public
complete workflow cannot execute. Adding another hard-coded public operation for ERC-20 would
repeat the leak and force callers to understand another internal phase.

### Current readiness proves counts, not intent

The existing readiness node compares Bitcoin and EVM family counts. Matching family counts do not
prove that every explicit wallet-to-symbol holding was collected, that no result was duplicated, or
that the observed resource is the one required by the portfolio.

Readiness must prove exact set equality between configured logical demand and completed source
observations.

### Historical fact selection is wider than a refresh-and-report goal

The report-only path searches full historical candidate sets and chooses the latest common anchor
per network. That is valid for a historical fact-backed report operation, but it is too broad for a
public workflow whose contract is to refresh this portfolio and report that collection.

Without an anchor constraint, a concurrent run can write newer facts after this run collects and
before its report reads the fact index. The report can then select the concurrent run rather than
the collection on which its graph depends.

The collection receipt must constrain report selection to this run's certified network anchors and
exact typed fact-content identities.

### Contract phase operations have no current user objective

The contract lifecycle operation was introduced to compose and exercise deploy, configure, and
validate state implementations. Those states are valuable reusable domain-state primitives. The four public
contract workflows and their operation wrappers are not current product objectives.

Keeping them creates setup kinds, request joins, import/adoption paths, program descriptors,
public-output wrappers, replay branches, tests, and documentation for speculative continuation
workflows. The state implementations can remain executable and well tested without those paths.

## Design principles

### Public entry-point admission rule

An operation is registered as an external entry point only when it:

1. represents a complete durable user objective;
2. owns completeness over all required subwork;
3. has a stable domain name independent of current graph topology;
4. is not primarily a fact producer, lifecycle phase, import mechanism, or continuation helper for
   another objective.

Graph size is irrelevant. A small graph can be a complete goal, while a large graph can still be an
internal implementation detail.

### Configuration discovery means deterministic derivation

The snapshot operation discovers work only from the one exact, resolved `PortfolioConfig` selected
by the request. It does not:

- scan other catalog rows;
- select a latest value;
- infer holdings from runtime routes;
- use live observations to change same-run topology;
- collect every token on a network for every account;
- infer native collection merely because a wallet is on an EVM network.

All topology decisions are deterministic planning decisions made before certification.

### Logical holdings and source observations are different types

A logical holding is a portfolio requirement such as “wallet A's USDC holding.” A source
observation is a protocol read such as `balanceOf(A)` on one token contract.

The portfolio compiler owns the mapping between those concepts. Source-near collector operations,
states, and facts do not carry portfolio `wallet_id` or `symbol_id`.

### Generality remains closed and typed

The design uses family-specific typed operations and exhaustive enum matching. It does not add:

- a universal cross-chain collector;
- a dynamic plugin registry;
- operation kind/version strings inside semantic requirements;
- trait-object dispatch selected from untrusted config;
- a generic JSON fact payload;
- a public service-locator or builder registry.

Adding a future holding source means implementing its semantic config, validation, state/capability
contract, fact, replay verification, collection operation, receipt projection, and report
projection as one complete change.

## Goals

- Provide one public portfolio objective from one exact aggregate config reference.
- Derive all and only the holdings explicitly configured by wallet-to-symbol references.
- Support Bitcoin native, EVM native, and ERC-20 holdings in the first implementation.
- Support EVM native-only, ERC-20-only, and mixed native/ERC-20 portfolios.
- Resolve one joint tip per network and share it across all resource reads on that network.
- Keep resource-specific operations and states reusable without making them entry points.
- Prove exact logical coverage before report execution.
- Constrain report facts to the collection receipt's anchors and exact fact-content identities.
- Preserve source-near facts, typed capabilities, certification, replay, and append-only evidence.
- Remove obsolete root-launch/public-output helpers and the contract lifecycle/phase operation
  wrappers that do not correspond to current user objectives.
- Preserve deploy, configure, and validate contract states as executable, test-backed domain-state
  primitives.
- Keep snapshot/report output direct by removing unsupported role buckets and inert time fields.
- Reduce duplicated config, request shapes, descriptors, setup kinds, and dispatch branches.

## Non-goals

- A standalone report-from-history or reuse-existing-facts mode.
- Standalone externally launchable collectors.
- Protocol positions, staking positions, Aave positions, or arbitrary contract readers.
- Discovering every asset owned by an address without explicit portfolio configuration.
- Supporting ERC-20 contracts without a valid `decimals()` and `balanceOf(address)` surface.
- A generic multi-chain collector abstraction.
- Public contract deployment, configuration, validation, lifecycle, or adoption workflows.
- Importing or adopting deployed/configured contracts from another run or external address.
- Backward compatibility with current entry-point IDs, setup values, schemas, or certified runs.
- An intentional empty snapshot or partial-success report.

## Public surface after cutover

The app entry-point registry contains exactly one entry point:

```text
mfm.portfolio/snapshot@1
```

The destructive mapping is:

| Current entry point | Decision |
|---|---|
| `mfm.portfolio/portfolio_snapshot@1` | Delete; semantics are replaced by `mfm.portfolio/snapshot@1`. |
| `mfm.portfolio/collect_then_report@1` | Delete; topology name is replaced by the portfolio objective. |
| `mfm.bitcoin/btc_address_balance@1` | Delete; retain collection as internal typed operations/states. |
| `mfm.evm/evm_native_balance@1` | Delete; retain collection as internal typed operations/states. |
| `mfm.evm.contract/deploy@1` | Delete without replacement. |
| `mfm.evm.contract/configure@1` | Delete without replacement. |
| `mfm.evm.contract/validate@1` | Delete without replacement. |
| `mfm.evm.contract/lifecycle@1` | Delete without replacement. |

There is no `@2` mirror and no alias from an old ID. Discovery and start validation reject every
deleted ID.

## Public request and setup contract

The strict JSON request contains one field:

```json
{
  "portfolio": {
    "name": "acme/primary",
    "digest": "content:sha256-jcs-v1:..."
  }
}
```

The app resolves exactly one `CatalogRef<PortfolioConfig>`, verifies its schema, canonical bytes,
and digest, revalidates it, and passes the concrete normalized value to
`PortfolioSnapshotOperation`.

The request contains no:

- BTC or EVM collector config;
- coverage selection;
- maximum source-read count;
- EVM native decimals;
- child operation list;
- collect/reuse mode;
- report-only mode;
- runtime route or endpoint.

Within this RFC's scope, setup publication exposes `PortfolioConfig`. It no longer exposes authored
BTC/EVM collector configs, contract context/action configs, or contract import/adoption specs as
independently selectable setup values. State config types remain owned by their semantic state/model
crates and can be nested by a future real operation without becoming catalog-public now.

Runtime config remains separate. It maps certified semantic EVM network IDs and Bitcoin source
identities to process-local providers and credentials.

## Portfolio semantic model

### Direct holding-source configuration

The current independently writable symbol `kind`, `role`, `protocol`, `balance_reader`, and
`underlying_symbol_id` fields create invalid combinations and advertise unsupported paths. Replace
them with a direct source algebra:

```rust
pub struct SymbolConfig {
    pub symbol_id: SymbolId,
    pub display_symbol: Option<String>,
    pub network_id: NetworkId,
    pub source: HoldingSourceConfig,
    pub valuation: SymbolValuationConfig,
    pub metadata: PublicMetadata,
}

pub enum HoldingSourceConfig {
    Native,
    Erc20 {
        contract_address: NormalizedEvmAddress,
    },
}
```

Snapshot observations project the same closed source directly: `Native` or `Erc20` with its contract
address. They do not carry a parallel `kind`, `role`, `protocol`, `balance_reader`, or
`underlying_symbol_id`. Native/ERC-20 branching is exhaustive matching on that one source value.
Protocol/staked semantics return only with a source type that can execute and report them end to
end.

### Network-owned native semantics

Bitcoin native quantity scale remains fixed by Bitcoin semantics. EVM JSON-RPC returns native
balances in the network's smallest unit, but does not expose a standard native-currency decimal
method. EVM native-currency scale therefore belongs in semantic `NetworkConfig::Evm`, not in a
run-local collector policy.

The EVM network variant contains the scale directly:

```rust
Evm {
    network_id: NetworkId,
    chain_id: NonZeroU64,
    native_decimals: u8,
    metadata: PublicMetadata,
}
```

ERC-20 decimals do not belong in authored symbol config. They are observed from the token at the
same anchor as its balance.

### Closed supported matrix

Portfolio validation accepts exactly:

| Network family | Holding source | Supported |
|---|---|---|
| Bitcoin | `Native` | Yes |
| Bitcoin | `Erc20` | No; reject configuration. |
| EVM | `Native` | Yes |
| EVM | `Erc20` | Yes |

There are no protocol, staking, or Aave source variants in the accepted model.

### Aggregate validation

`ValidatedPortfolioConfig` proves, normalizes, and indexes the complete aggregate before graph
expansion. In addition to the existing identifier, network, wallet, symbol, quote, and secret-field
rules, validation must prove:

- every wallet references an existing network;
- every wallet subject family matches its network family;
- every referenced symbol exists and belongs to the wallet's network;
- every `(network family, holding source)` pair is supported;
- the logical holding set is non-empty;
- a token contract address is normalized and non-zero;
- no two logical holdings map to the same source balance and would therefore be double-counted;
- no duplicate semantic wallet subject exists on one network;
- quote routes remain complete and float-free;
- normalized network, wallet, symbol, and nested collection ordering is deterministic.

Rejecting source aliases is deliberate. If two wallet IDs name the same network/address or two
symbol aliases cause the same address/token balance to appear twice, summing both would overstate
the portfolio. A future explicit view/alias concept must define non-additive semantics rather than
reuse a balance source implicitly.

## Deterministic holding-demand compiler

### Logical demand

The compiler derives one requirement for each explicit wallet-to-symbol reference:

```text
D = {
  (wallet_id, symbol_id, network_id)
  for each symbol_id in wallet.symbol_ids
}
```

An EVM wallet with only ERC-20 symbol references is a valid token-only wallet. No native requirement
is added automatically. A configured symbol that no wallet references creates no implicit
accounts-by-token product and no collection demand. Unreferenced symbols may remain as semantic or
valuation definitions; only explicit wallet-to-symbol edges are holdings. A portfolio whose
complete edge set is empty fails aggregate config validation. Run-start revalidation surfaces the
same error before graph expansion; the compiler never receives an empty validated manifest.

Each logical requirement receives a deterministic `HoldingRequirementKey` and exactly one typed
source key:

```rust
pub enum HoldingSourceKey {
    BitcoinNative {
        network_id: NetworkId,
        source_identity: BitcoinSourceIdentityId,
        address: BitcoinAddress,
    },
    EvmNative {
        network_id: NetworkId,
        chain_id: NonZeroU64,
        account: NormalizedEvmAddress,
    },
    Erc20 {
        network_id: NetworkId,
        chain_id: NonZeroU64,
        contract_address: NormalizedEvmAddress,
        account: NormalizedEvmAddress,
    },
}
```

Operation kind/version strings, runtime routes, wallet IDs, symbol IDs, and report result IDs do not
enter the source key.

### Logical coverage versus physical batching

Readiness is proven over logical `D`. Physical work is grouped by source semantics:

- one joint tip per network;
- one Bitcoin native read per required Bitcoin source key;
- one EVM native read per required EVM-native source key;
- one ERC-20 `decimals()` read per unique `(network, chain_id, contract_address)`;
- one ERC-20 `balanceOf(account)` read per required ERC-20 source key.

Batching and token-metadata reuse must not erase a logical requirement. Source operation outputs
remain source-near. The parent receipt assembler owns the certified mapping back to logical
requirements.

### Deterministic ordering

The compiler uses normalized config plus sorted maps/sets. Network child scopes, resource child
scopes, token metadata nodes, balance nodes, state keys, expected receipt entries, and public-output
bindings have stable ordering and naming derived from normalized keys, never hash-map iteration or
live results.

The same exact `PortfolioConfig` and operation version must produce the same typed graph and config
artifacts.

## Internal operation graph

The conceptual graph is:

```text
PortfolioSnapshotOperation(Config = PortfolioConfig)
  -> derive exact logical holding manifest
  -> for each required Bitcoin network
       BitcoinNetworkCollectionOperation
         -> ResolveBtcJointTipState once
         -> BitcoinNativeBalancesAtAnchorOperation
              -> observe + record source-near facts
         -> source-near Bitcoin network receipt
  -> for each required EVM network
       EvmNetworkCollectionOperation
         -> ResolveEvmJointTipState once
         -> EvmNativeBalancesAtAnchorOperation       [only when native is required]
              -> observe + record source-near facts
         -> EvmErc20BalancesAtAnchorOperation        [only when ERC-20 is required]
              -> observe token metadata + balances + record source-near facts
         -> source-near EVM network receipt
  -> AssemblePortfolioCollectionReceiptState
       -> exact expected/actual requirement equality
       -> one exact anchor per network
  -> internal PortfolioReportOperation
       -> ResolveSubjects
       -> SelectHoldings constrained by receipt anchors
       -> ResolveValuations
       -> AssembleSnapshot
       -> ProjectReport
```

The internal operation names are descriptive targets; implementation may retain an existing crate
when it already has the correct semantic boundary or rename/merge wrappers when deletion is simpler.
The required boundaries are:

- the public root owns the complete portfolio objective;
- a family/network operation owns joint-tip sharing;
- resource operations own reusable typed subgraphs;
- states own observation, normalization, fact writing, and pure assembly semantics;
- adapters bind state intent to capabilities and evidence;
- transports remain generic protocol implementations.

No internal collector operation has a standalone root draft/launch helper or public-output wrapper.
The existing BTC chain-head checkpoint/cycle remains internal Control machinery when its checkpoint
consumer is retained; it is not a holding collector and is not absorbed into the snapshot surface.

The fan-in uses the program framework's existing certified vector-of-handles input binding; it does
not require a variadic state type or erased handles. Each Bitcoin network child returns one
`BitcoinNetworkCollectionReceipt`. Each EVM network child uses a pure
`AssembleEvmNetworkCollectionReceiptState` with separate typed vectors containing zero-or-one native
batch receipt and zero-or-one ERC-20 batch receipt. It produces one
`EvmNetworkCollectionReceipt` only after proving that the configured resource set is exact and
non-empty. The portfolio receipt state has this static typed input shape:

```text
PortfolioCollectionReceiptInput {
  bitcoin_networks: Vec<BitcoinNetworkCollectionReceipt>,
  evm_networks: Vec<EvmNetworkCollectionReceipt>,
}
```

The operation binds configuration-sized, deterministically sorted vectors of typed child handles to
those fields. Their binding shape and order are part of the certified graph. The exact manifest in
the state config proves the expected vector members, so empty family vectors are valid while the
combined manifest cannot be empty.

## EVM anchoring contract

`EvmNetworkCollectionOperation` resolves exactly one `EvmJointTip` per required network. It passes
the same typed handle to native and ERC-20 child operations. Native-only, token-only, and mixed
networks therefore have the same anchor model.

Every EVM read that can affect a holding fact must:

- use the certified EVM network binding;
- select the joint-tip block hash, not `latest` or an independently resolved number;
- use the EIP-1898 `{ blockHash, requireCanonical: true }` selector;
- retain redacted source-read evidence;
- verify response network/source identity;
- re-verify the joint tip as required by the state contract before fact recording;
- fail on missing hash, drift, mismatch, malformed response, or provider-source disagreement.

One resource child must never resolve its own competing tip inside a network collection.
An EVM provider that cannot honor the canonical hash selector is unsupported and fails the required
collection; there is no number/tag fallback.

Bitcoin retains the analogous one-joint-tip-per-network/source contract and exact-anchor balance
verification.

## ERC-20 execution contract

### Existing reusable capability boundary

The existing `EvmCallReadCapability` and `EvmCallReadProvider` are the correct generic authority for
ERC-20 reads. The EVM transport already supports hash-selected `eth_call`, and `mfm-evm-core`
already owns `balanceOf(address)` and `decimals()` calldata encoding helpers.

The ERC-20 implementation extends the EVM collector adapter/provider binding to include call reads.
It does not add an ERC-20-specific transport or capability.

### Token metadata observation

For each unique token on a network, `ObserveErc20TokenMetadataState` calls `decimals()` at the shared
joint-tip hash and returns typed metadata bound to:

- network ID;
- chain ID;
- token contract address;
- observed decimals;
- block number and block hash;
- redacted source evidence.

The result is reused by every balance observation for that token in this network batch. No symbol,
name, price, wallet ID, or display metadata is read from the token contract.

The call data is exactly the four-byte `decimals()` selector `0x313ce567`, with no arguments or
trailing bytes.

The state accepts exactly one canonical 32-byte ABI word. The high 31 bytes must be zero and the
remaining byte is the `u8` decimal count. It fails closed when the call reverts, returns a value of
any other length, has non-zero high bytes, disagrees with the certified network/source binding, or
cannot be proven at the joint tip. This intentionally defines the first supported ERC-20 subset as
contracts with a usable standard `decimals()` surface.

### Token balance observation and fact writing

For each required token source key, `ObserveErc20BalanceState`:

1. consumes the shared `EvmJointTip` and matching typed token metadata;
2. calls `balanceOf(account)` at the exact joint-tip hash;
3. decodes exactly one 32-byte ABI word as an unsigned 256-bit integer;
4. verifies token, account, network, chain, source, and anchor binding;
5. returns a typed source-near observation.

The call data is exactly the four-byte `balanceOf(address)` selector `0x70a08231` followed by one
32-byte ABI word containing twelve zero bytes and the normalized 20-byte account. No shorter,
longer, or differently padded request is accepted.

The balance word is the full unsigned 256-bit range. Zero is a successful observation: it is
recorded as a zero-valued fact and rendered as a zero-valued holding in the report. Zero is not
missingness and must not be removed by collection, receipt assembly, fact selection, or projection.

`RecordErc20BalanceFactState` writes one Platform fact only after those checks pass.

The fact contract is:

| Surface | Fields |
|---|---|
| Kind | `evm.address_erc20_balance_snapshot` |
| Subject | `network`, `chain_id`, `contract_address`, `account` |
| Result | `block_number`, `block_hash`, `raw_units`, `decimals`, `coverage`, `source_status` |
| Metadata | store-owned `store_commit_order` and ordinary fact evidence |

The subject and response contain no portfolio wallet ID, symbol ID, valuation, endpoint, credential,
or secret. `raw_units` is a decimal digit string; report formatting uses integer/string decimal
arithmetic and never a float.

Exact `balanceOf` for the complete `(token, account)` subject is a `complete_at_anchor` observation.
Coverage and source status are state-owned closed semantics, not request policy.

### Replay

ERC-20 replay recomputes and verifies, from retained evidence:

- the destination token contract;
- the exact `decimals()` or `balanceOf(address)` selector and calldata, including account padding;
- the EIP-1898 block-hash selector and `requireCanonical = true`;
- the raw return bytes and exact 32-byte result shape;
- the decoded decimal count or full `uint256` balance;
- the certified network, chain, provider-source, token, account, and shared-tip binding;
- the typed token metadata, balance observation, recorded fact identity, batch receipt, and report
  query evidence.

Tampering with any of those fields fails replay. Replay does not construct a live EVM provider or
consult runtime config.

## Collection receipt and report gate

### Exact fact-content identity

Receipts use one reusable fact-boundary type, `FactContentIdentity`, computed only from existing
canonical fact primitives:

```text
FactContentIdentity {
  fact_descriptor_hash,
  subject_material_hash,
  response_schema_id,
  response_hash,
}
```

`fact_descriptor_hash` fixes the complete fact descriptor and its schema contract;
`subject_material_hash` fixes the canonical typed subject; and `response_schema_id` plus
`response_hash` fix the canonical typed response artifact. The identity itself is canonically
encoded and content-addressed under the `mfm.fact.content-identity.v1` digest domain.

This identity deliberately excludes `FactClaimId`, source-run coordinates, `artifact_id`, artifact
evidence location, `store_commit_order`, catalog watermark, and query-return order. Those identify a
claim or storage occurrence, not its semantic fact content. Two independently recorded claims with
the same `FactContentIdentity` are semantically equivalent for this workflow. A response change,
subject change, or descriptor/schema change necessarily produces a different identity.

### Source receipts

Each network/resource collection produces typed source-near receipts only after all required fact
record handles complete. At minimum they contain:

- network/source binding;
- resource kind;
- sorted observed source keys and the exact `FactContentIdentity` of each typed fact value;
- the exact shared anchor;
- successful observation count;
- fixed admissible coverage/status evidence.

They do not contain portfolio presentation or valuation fields.

### Exact portfolio receipt

`AssemblePortfolioCollectionReceiptState` receives:

- the expected logical manifest and logical-to-source mapping derived from `PortfolioConfig`;
- every typed source receipt from child operations, including exact `FactContentIdentity` values.

It must prove:

- every expected logical requirement maps to one successful source result;
- no expected result is missing;
- no result or source key is duplicated;
- no unexpected result is present;
- every source result has admissible coverage and `source_status = ok`;
- every result on one network has the exact same height/number and hash;
- every result agrees with its configured network, chain/source identity, account, and token;
- every logical requirement is bound to the exact `FactContentIdentity` produced for it;
- at least one logical requirement completed.

Counts alone are not authority. The output is one typed `PortfolioCollectionReceipt` whose sorted
holding entries each contain:

```text
CollectedHoldingReceipt {
  logical_requirement_key,
  source_key,
  fact_content_identity,
  anchor,
}
```

The receipt also carries the exact network anchors and manifest identity needed by reporting. Its
canonical representation is the complete authority; count summaries are only diagnostics.

### Anchor-constrained fact selection

The internal report operation consumes the receipt handle as a typed dependency. For every logical
holding, it compiles a fact query using:

- the exact source key derived from the portfolio;
- the expected fact kind and exact descriptor hash retained in `FactContentIdentity`;
- the receipt's exact network anchor;
- admissible state-owned coverage/status;
- the certified store scope and query policy.

The exact source-and-anchor query uses a certified scan bound `N` and requests at most `N + 1`
claims. Existing query-result cardinality evidence makes the boundary explicit:

- `Exact(n)` with `n <= N` proves that the returned candidates exhaust the scope;
- `AtLeast(N + 1)` fails with `candidate_bound_exhausted` before hydration or selection.

There is no successful truncated case. On exact exhaustion, the state hydrates every candidate,
recomputes its `FactContentIdentity`, and filters by equality with the receipt before applying
deterministic claim ordering. It then requires at least one identical-content claim and selects
among only those equivalent claims using retained query evidence. If the fact index later supports
an exact identity predicate, it may additionally narrow the query, but hydration and canonical
identity verification remain mandatory authority. Selecting a claim first and checking its identity
afterward is forbidden because a conflicting claim could otherwise mask the exact collected fact.

Selection no longer searches for the newest common historical anchor. A newer concurrent fact at a
different anchor cannot replace this collection, and a conflicting concurrent fact for the same
source and block hash cannot replace the exact fact value produced by this run. Multiple claims with
identical fact content are semantically equivalent; claim selection among those identical values
remains deterministic and evidence-backed.

The selection policy receives this new durable identity:

```text
mfm.portfolio.holding.collection-receipt-anchor.v1
```

The old `latest-network-coherent` policy is not used by the sole public snapshot workflow.

Missing exact-anchor fact material or a fact-content identity mismatch is a hard failure. Because
collection already proves one shared anchor, `no_common_network_anchor` is no longer a normal public
product path. Any anchor disagreement is a collection-receipt integrity failure.

Public `network_pins` remain projections of selected fact anchors and must equal the receipt. They
are descriptive output, not independent authority.

## Public output contract

The sole root binds one version-1 `PortfolioPublicOutputs` value with two projections:

| Field | Contract |
|---|---|
| `snapshot` | Exact configured holdings, quantities, valuations, source coverage, and network pins. |
| `report` | Per-wallet and portfolio totals by configured quote unit over that exact snapshot. |

The snapshot contains only explicit wallet-to-symbol holdings. Its source wire type is direct:

```rust
pub struct AnchoredHoldingSource {
    pub holding: HoldingSourceConfig,
    pub anchor: ObservationAnchor,
}
```

Each observation carries one `source: AnchoredHoldingSource`, its raw/scaled quantity, quote values,
and state-owned coverage tag. `HoldingSourceConfig` is the same closed `Native` or
`Erc20 { contract_address }` algebra used by semantic config. `Observation.kind`,
`Observation.role`, `Observation.protocol`, and `ObservationSource.balance_reader_kind` are deleted;
there is no parallel reader string or classification enum. A zero native or token balance remains a
present zero-valued observation.

Unreferenced symbol definitions may remain in the snapshot's semantic symbol material, but they
contribute no holding or value. Every wallet and the portfolio still have one total row for every
normalized `PortfolioConfig.quote_codes` entry, including a canonical zero when their explicit
observations sum to zero.

Because this version supports only positive native and ERC-20 asset balances, each wallet/portfolio
quote total is the direct shape `{ quote, total_value_dec }`. At wallet scope, `total_value_dec` is
the canonical decimal sum of every explicit observation's `value_dec` for that quote. Portfolio
scope is the canonical decimal sum of its wallet totals. Both reducers use checked integer/decimal-
string arithmetic with no floats. Delete all role dispatch, debt sign inversion, positive-bucket
merging, `net` derivation, and the current assets/collateral/debt/staked/net fields; keeping
permanently zero speculative categories would preserve the unsupported model by another name. A
future executable position source may introduce a new direct exposure/total contract with its full
semantics.

Delete the current `generated_at_ms` placeholder from both projections and remove its snapshot
assembly argument, hard-coded zero injection, replay plumbing, and snapshot-to-report propagation.
Network pins are exact chain-state/as-of anchors, not wall-clock timestamps. Public snapshot/report
semantics intentionally contain no generation wall-clock time; run-event timestamps remain
operational metadata only. A future timestamp may be added only when it is derived from explicit
typed evidence and has defined cross-network semantics.

The public output does not expose collection receipts, `HoldingRequirementKey`, source keys,
`FactContentIdentity`, internal fact refs/claim IDs, artifact identities, scan bounds, provider
evidence, or runtime routes. Those remain retained execution/replay evidence. Snapshot and report
collections are canonically sorted, and both project network pins that exactly match the collection
receipt.

## Policy ownership

The public request exposes no collector policy. Policy belongs to stable semantic owners:

| Concern | Owner |
|---|---|
| Which holdings exist | Exact `PortfolioConfig` wallet-to-symbol relation |
| Network identity | Semantic `NetworkConfig` |
| Runtime endpoint/provider | Process-local runtime config |
| Joint-tip selection policy | Fixed operation-version policy in certified joint-tip state config |
| Source-read/fact-scan bounds | Fixed operation-version policy in certified child state config |
| Native quantity scale | Bitcoin semantics or EVM native-currency network config |
| ERC-20 quantity scale | `decimals()` observation at the exact anchor |
| Fact coverage/status | Resource state semantics |
| Report fact anchor | Typed collection receipt |
| Valuation | Semantic symbol valuation config |

The parent operation derives the joint-tip selection policy and every source-read/fact-scan bound
from closed, versioned operation policy and writes the exact values into child state config before
certification. Those values are therefore hash-defining spec material. Adapters enforce them but do
not own or default them.

There is no caller-configurable honesty claim or source-read knob in the run request, and there is
no outcome-affecting hidden adapter default.

## Error and partial-execution semantics

Aggregate publication validation, repeated after exact run-start resolution, rejects:

- invalid or non-canonical portfolio config;
- unknown or mismatched network/wallet/symbol join;
- unsupported network/source combination;
- empty collection demand;
- duplicate source alias that would double-count;
- invalid network identity or native scale;
- invalid token contract address;
- invalid valuation routes.

No graph is expanded from a value that fails that validation.

After execution begins, any required source failure fails the workflow. The report graph cannot run
without a valid exact receipt. There is no omitted branch, fabricated zero for a missing holding,
partial-success snapshot, historical fallback, or reuse-existing-facts mode.

Facts written before a later child fails remain append-only Platform facts. Their existence does not
turn the failed run into a successful report and does not authorize a later run to skip refresh.

Provider and token diagnostics remain closed and redaction-safe. They must not retain RPC URLs,
headers, provider bodies/messages, filesystem paths, credentials, signer material, or signed
transactions. This does not prohibit the narrowly typed call-result or runtime-bytecode evidence
explicitly required for replay; it prohibits opaque transport bodies and provider-authored message
strings from crossing the diagnostic boundary.

## Determinism, certification, persistence, and replay

The cutover preserves these invariants:

- planning has no ambient IO;
- config-derived branching completes before certification;
- live values cannot change topology in the same run;
- normalized keys determine all iteration and node ordering;
- operation, state, adapter, value, fact, and public-output descriptors are certified;
- derived child configs become concrete certified config artifacts;
- `RunAdmitted` records the one exact entry-point ID and exact catalog source identity;
- each append remains atomic and stream history remains append-only;
- facts and retained evidence remain content-addressed according to existing contracts;
- runtime capability binding occurs only after admission;
- replay recomputes pure behavior and consumes retained read/fact/side-effect evidence only;
- replay, status, stream, and public-output paths do not load catalog or runtime config;
- no structured hashed value contains a float;
- no persisted or returned surface contains secrets.

The report phase remains fact-backed. Combining collection and report in one public operation does
not give report states live chain authority. Only collector states declare BTC/EVM read
capabilities; report states consume fact-index evidence and the typed collection receipt.

## Architecture and ownership

| Concern | Owner |
|---|---|
| Portfolio/network/wallet/symbol/source model | `mfm-portfolio-model` |
| Public objective and deterministic demand compiler | portfolio snapshot operation crate |
| Bitcoin joint-tip/native collection subgraph | BTC internal operation/state crates |
| EVM joint-tip and family coordination | EVM internal collection operation crate |
| Native balance observation and fact semantics | reusable BTC/EVM state crates |
| ERC-20 metadata/balance/fact semantics | reusable EVM state crate |
| Generic `FactContentIdentity` and canonical derivation | `mfm-facts` |
| State intent -> capability/evidence binding | BTC/EVM adapter crates |
| JSON-RPC implementation | reusable BTC/EVM transport crates |
| Exact receipt assembly | portfolio operation-local pure state |
| Fact selection, valuation, snapshot, report | portfolio states and internal report operation |
| Exact catalog resolution and entry-point dispatch | `mfm-app` |
| Runtime routes/signers/endpoints | runtime config and app live assembly |
| CLI/REST request transport and rendering | thin binaries |

The implementation should prefer deleting wrapper crates/types over renaming them when they have no
consumer after cutover. It must not move workflow planning into app or binaries to reduce crate
count.

## EVM contract operation removal and state preservation

### Delete operation surfaces

Delete the entire `mfm-op-evm-contract-lifecycle` crate and its:

- four entry configs and builders;
- four `Operation` implementations;
- operation descriptor registry;
- operation output/public-output handle structs;
- root draft and launch-plan helpers;
- operation tests.

Delete the associated app entry-point IDs, request schemas/types, dispatch arms, setup kinds,
catalog examples, CLI/REST docs, dependencies, fixtures, and discovery tests. Do not add a
replacement “contract states operation.”

### Retain executable domain-state primitives

Retain in the contract state/model boundary:

- `ContextBoundDeployContractState`;
- `ContextBoundConfigureContractState`;
- `ContextBoundValidateContractState`;
- `DeployAction`, `ConfigureAction`, and `ValidateAction`;
- typed context, intent, input, output, submission, receipt, confirmation, and validation values;
- signer/transaction/fee/finality policies;
- the exclusive account-nonce resource claim;
- deployed -> configured -> validation lineage contracts.

Retain in the contract adapter/capability boundary:

- mutation preparation, signing, submission, and ambiguity recovery;
- nonce-occupancy investigation;
- receipt/finality verification;
- validation chain, code, call, and log reads;
- runner bindings for the three states;
- side-effect and read replay verification;
- runtime factory/provider abstractions.

Registering internal state descriptors/runners does not make them public entry points. There is no
contract operation descriptor after cutover.

### Delete continuation imports and impossible model paths

`ImportDeployedContractState` and `ImportConfiguredContractState` exist for configure-only and
validate-only workflows. Delete them with:

- `ImportDeployedSpec` and `ImportConfiguredSpec`;
- source-run import and external-adoption request/evidence/policy types;
- import runners and source-run certification authority;
- import/adoption replay branches and fixtures;
- import-only provenance and error variants.

Tighten producer contracts:

- configure accepts deployed resources only from `ContextBoundDeployContractState`;
- validate accepts configured resources only from `ContextBoundConfigureContractState`.

Simplify deployed/configured provenance and validation models so removed import/adoption variants
cannot be represented. Do not retain optional fields that are now permanently empty.

### Finish retained state semantics

The retained states must not carry inert config:

- remove configure confirmation read/event assertion fields that no implementation consumes;
- make contract validation observe and verify `ContractProfile.deployed_code_hash` rather than
  retaining an ignored code-identity field;
- remove any capability or model surface that remains unused after those decisions.

Direct deploy/configure lineage must retain a typed block number and block hash for every successful
transaction receipt. The configure state derives one `ConfiguredContractAnchor { block_number,
block_hash }`: the last successful configure receipt in certified transaction order, or the deploy
receipt when configuration submitted no transaction. Receipt confirmation must prove that anchor
canonical/final under its certified policy; a block number without its hash is insufficient. The
configure confirmation and `ConfiguredContractInstance` retain this anchor directly, so validation
constructs its request from typed input without reading an artifact or consulting a provider.

When `ContractProfile.deployed_code_hash` is present, the validation state's typed read request must
carry that expected hash and the configured instance's exact validation block hash. The adapter uses
`eth_getCode` with EIP-1898 `{ blockHash, requireCanonical: true }` for the configured address. Its
retained external-read artifact contains the returned runtime bytecode, exact selector, redacted
source evidence, observed byte length, and observed Keccak-256 code hash. The validation report may
project only the length and hash, but replay authority retains the content-addressed bytecode. The
state includes all of the following in its `valid` projection:

- the code read is for the configured address, network, chain, provider source, and certified block;
- the observed byte length is non-zero;
- the observed code hash equals the profile's expected hash.

A well-formed, authenticated read that observes empty code or a different hash produces a successful
validation report with `valid: false`. Malformed code bytes, an unauthenticated response, a source or
anchor mismatch, or internally inconsistent evidence fails execution instead of becoming a policy
result. Replay verifies the retained bytecode artifact's schema and content digest, recomputes
Keccak-256 from those bytes, and repeats every binding and equality check without a live provider.

When `deployed_code_hash` is absent, validation schedules no code-identity read and makes no code
identity assertion. This preserves the field's existing optional meaning without carrying an inert
request or response branch. The validation state still evaluates its other configured assertions.
A future product objective may make code identity mandatory by requiring the field in its own
high-level configuration; the reusable state does not invent that policy.

A validation state may complete successfully with a typed report whose `valid` field is false. That
is appropriate for a reusable reporting state. A future high-level objective that requires a
valid contract must compose an explicit pure `require-valid` gate.

### Preserve end-to-end tests without a production operation

Use a test-only typed graph:

```text
deploy state -> configure state -> validate state
```

It must exercise context lineage, producer restrictions, exclusive signer/nonce claims,
side-effect saga policy, live runner bindings, receipt/finality behavior, resume, replay, and
validation projection. Compile-fail typestate tests remain with the state boundary. The test graph
must not be registered as an app entry point or setup kind.

## Destructive cutover

The implementation deletes rather than deprecates:

- all eight current entry-point IDs and their request schemas;
- standalone collector and report public-output/root helpers;
- `CollectThenReportRequest`, BTC/EVM public collector policies, derived child config vectors, and
  their relational builder;
- standalone collector setup kinds and duplicated example values;
- the authored `SymbolConfig.kind`, `role`, `protocol`, `balance_reader`, and
  `underlying_symbol_id` fields, plus parallel observation/report fields and branches;
- `SymbolKind`, `SymbolRole`, `BalanceReaderConfig`, protocol reader IDs/configs, the Aave
  model/module and IDs/configs, and all associated tests and descriptors with no remaining consumer;
- role-based assets/collateral/debt/staked/net total fields and the inert `generated_at_ms`
  placeholder, including its assembly/replay/projection plumbing;
- family-count readiness config/state/output;
- old public historical-selection policy usage;
- all EVM contract operation and import/continuation surfaces;
- stale docs, examples, snapshots, fixtures, tests, registry assertions, and dependency edges.

The new ID, request schema, portfolio source schema, ERC-20 values/facts, receipt values,
snapshot/report output schemas, operation descriptors, and report selection policy start at version
1 because they are new contracts. Old version-1 contracts are removed, not migrated.

Previously certified branch runs using deleted descriptors intentionally become non-resumable and
non-replayable. Drop and recreate the experimental branch database, republish the new catalog
contents, and regenerate fixtures and snapshots. Do not update, delete, or truncate append-only
catalog rows in place, and do not keep old descriptors trusted solely to read experimental history.

No implementation step may add:

- an alias or deprecated entry point;
- legacy request/config decoding;
- a schema fallback;
- a dual old/new setup document;
- a facade operation calling the new operation;
- a dynamic collector registry;
- a replay compatibility shim;
- a partial ERC-20 placeholder.

## Implementation sequence

This is one logical cutover even if developed through compile-green commits:

1. Reshape `PortfolioConfig`, network-native semantics, and aggregate validation.
2. Add typed ERC-20 metadata/balance/fact states and focused unit tests.
3. Extend the EVM collector adapter/provider and replay verifier for call reads.
4. Refactor BTC/EVM collectors into internal at-anchor operations and network coordinators.
5. Replace mutable child vectors and count readiness with config-derived demand and exact receipt.
6. Constrain report fact queries to receipt anchors, add ERC-20 projection/hydration, and simplify
   the new snapshot/report output schemas.
7. Replace app/setup/CLI/REST discovery and launch assembly with the sole snapshot entry point.
8. Delete contract operations/imports while preserving and tightening the three executable states.
9. Replace examples, docs, integration tests, metadata contracts, and branch database fixtures.
10. Run focused Cargo checks during development and the repository Nix gates required by
    `AGENTS.md` before commit/merge readiness.

## Acceptance criteria

### Public surface and configuration

- Public discovery returns exactly `mfm.portfolio/snapshot@1`.
- Every deleted entry-point ID is rejected; no alias or latest selection exists.
- The request strict-decodes exactly one `portfolio: CatalogRef<PortfolioConfig>` field.
- Setup exposes no standalone collector, contract action/context, or import/adoption value kind.
- Catalog evidence is retained at admission and never consulted by runtime/replay/read-only paths.
- Unknown config/request fields and secret-shaped values fail closed.

### Portfolio model and planning

- `SymbolConfig` has one direct holding source rather than independently writable kind/reader paths.
- Snapshot observations project that direct source and have no parallel kind/role/protocol fields.
- BTC-native, EVM-native, and EVM-ERC-20 are the only accepted holding source combinations.
- Protocol, staking, and Aave variants are absent rather than planning-time placeholders.
- Empty, mismatched, or double-counting portfolio aggregates fail validation.
- Unreferenced symbol definitions produce no collection nodes; only explicit wallet-to-symbol edges
  create holdings.
- Demand is exactly the explicit wallet-to-symbol relation.
- No native holding is inferred for a token-only wallet.
- No account-by-all-tokens product is generated.
- Graph and manifest ordering are deterministic across normalized equivalent inputs.

### Collection graph

- BTC-only portfolios expand no EVM nodes.
- EVM-only portfolios expand no Bitcoin nodes.
- EVM native-only networks expand no ERC-20 balance/metadata nodes.
- EVM token-only networks expand no native-balance nodes.
- Mixed networks expand both resource operations.
- Exactly one joint tip is resolved per required network.
- Native and ERC-20 reads on one EVM network consume the same joint-tip handle.
- Configuration-sized network receipt vectors use certified typed handle bindings; no erased or
  runtime-variable state input shape is introduced.
- Resource operations and states remain reusable but have no external root/public-output path.

### ERC-20

- `decimals()` executes once per unique token/network at the shared anchor.
- `balanceOf(account)` executes once per required token source key at that anchor.
- Hash-bound call requests require canonical anchor identity and retain redacted read evidence.
- Revert, empty/malformed result, `u8` decimal overflow, malformed balance, source mismatch, or anchor
  drift fails the required collection.
- `decimals()` accepts only a zero-padded `u8`; `balanceOf(address)` accepts the full 32-byte
  `uint256`, including zero.
- A zero token balance is recorded and reported as zero, never treated as absent.
- The ERC-20 fact is source-near, float-free, and contains token/account/chain/anchor/raw units/scale.
- ERC-20 evidence-only replay verifies destination, exact calldata, canonical hash selector, raw
  bytes, decoding, certified source binding, fact identity, receipts, and report queries.

### Receipt and reporting

- Exact receipt assembly detects missing, duplicate, unexpected, mismatched, or empty results.
- Every logical holding is covered exactly once.
- `FactContentIdentity` is derived in `mfm-facts` from descriptor hash, subject-material hash,
  response schema ID, and response hash, excluding claim/storage occurrence metadata.
- Every selected fact is constrained to the receipt's exact network anchor.
- A newer concurrent fact at another anchor cannot replace this run's collection.
- A conflicting same-anchor fact cannot replace the receipt's exact `FactContentIdentity`.
- Identical-content claim selection remains deterministic and evidence-backed.
- Query cardinality must prove exact candidate exhaustion within the certified bound; saturation at
  `N + 1` returns `candidate_bound_exhausted`, never a truncated selection.
- Exact identity filtering happens before deterministic claim ordering.
- Missing exact-anchor facts fail; the workflow emits neither a fabricated zero nor a partial
  report.
- Public network pins come from selected facts and exactly match the receipt.
- Report states have no live BTC/EVM capabilities.
- Replay uses retained collector and fact-query evidence without live providers or mutable catalog.

### Public output

- The root exposes exactly one typed object with `snapshot` and `report` projections.
- Snapshot observations use the direct holding source and include exact anchor, quantity, value, and
  coverage; zero balances remain present.
- `AnchoredHoldingSource` contains the exact `HoldingSourceConfig` and `ObservationAnchor`; old
  observation kind/role/protocol/reader fields are absent.
- Quote totals contain only `{ quote, total_value_dec }` and equal direct canonical sums of explicit
  observation values; unsupported role reducers/subtotals are absent.
- Every wallet and portfolio has one row per configured quote code, including canonical zero when
  its direct sum is zero.
- `generated_at_ms` and all clock/zero plumbing are absent; output intentionally has no wall-clock
  generation time, while network pins provide chain-state anchors.
- Internal requirement/source/fact/artifact/provider identities and scan bounds are not public.
- Snapshot/report ordering is canonical and both network-pin projections equal the receipt.

### Contract state primitives

- No contract entry point, setup kind, operation descriptor, or lifecycle operation crate remains.
- Deploy, configure, and validate state descriptors, runners, capabilities, and replay support remain.
- Configure accepts only the retained deploy-state output; validate accepts only configure-state
  output.
- Import/adoption states, configs, evidence, replay, and model variants are absent.
- Configure output directly retains one certified `(block_number, block_hash)` validation anchor,
  using the last successful configure receipt or the deploy receipt when no configure transaction
  exists.
- With an expected hash, contract validation reads code at a certified block hash and includes code
  presence and exact hash equality in `valid`; mismatch or empty code returns `valid: false`.
- Without an expected hash, validation performs no code-identity read.
- Malformed or unauthenticated code evidence fails execution, and replay recomputes every code
  identity check without a live provider.
- Replay verifies the retained runtime-bytecode artifact schema/digest and recomputes Keccak-256;
  recorded length/hash alone are insufficient evidence.
- No retained configure field is inert.
- A test-only deploy -> configure -> validate graph covers live execution, resume, replay, and
  compile-time lineage without becoming a production operation.

### Repository verification

- CLI, REST, architecture, design, runbooks, examples, and RFC_CONFIG no longer describe old paths.
- `docs/persisted-public-surfaces.md` inventories the new request, config, state/fact/receipt schemas,
  errors, and public output, with removed surfaces absent.
- `docs/evm-rpc-routing.md` covers the EVM call/code-read bindings required by portfolio ERC-20 and
  retained contract validation after contract entry points disappear.
- Cargo metadata tests enforce removed operation/dependency edges and retained state boundaries.
- Fact/schema descriptor tests cover the new source and receipt contracts.
- Focused unit/integration tests cover native-only, token-only, mixed, BTC/EVM, failure, and replay
  matrices.
- `cargo fmt --all -- --check`, focused Cargo checks/tests, `nix run .#check`,
  `nix run .#test`, `nix run .#test-db`, and final `nix run .#ci` pass before merge readiness.

## Resolved decisions and accepted limitations

- The sole public objective always refreshes before reporting.
- There is no public or internal report-only root workflow.
- There are no public collectors or contract workflows.
- ERC-20 is required in the first implementation, not a future placeholder.
- ERC-20 scale comes from anchored `decimals()`, with no authored fallback.
- EVM native scale belongs to semantic network config.
- Native and token resource operations remain separate and share a family/network anchor.
- Protocol/staking/Aave holdings are unsupported and absent from authored config.
- Public output contains the detailed snapshot and direct quote-total report, with no role buckets or
  ambient/placeholder generation timestamp.
- Empty portfolios, source aliases, partial collection, and fact-reuse modes are errors/not present.
- Contract import/adoption workflows are not preserved speculatively.
- Existing experimental runs and schemas are intentionally not compatible.

There are no open design questions in this RFC. Any future expansion must propose a complete new
user objective or holding-source contract without reopening removed compatibility paths.

## Related documents and current implementation evidence

- `RFC_CONFIG.md` — catalog and exact-reference architecture; public-surface clauses superseded here
- `docs/design.md` — authoritative certification, fact, replay, persistence, and secret contracts
- `docs/architecture.md` — operation/state/adapter/transport taxonomy and placement rules
- `docs/persisted-public-surfaces.md` — persisted/public schema inventory updated by this cutover
- `docs/evm-rpc-routing.md` — live and replay routing for EVM balance, call, and code reads
- `docs/portfolio-collect-then-report.md` — current authority contract; must be replaced at cutover
- `docs/evm-contract-lifecycle.md` — current operation/import runbook; must become reusable-state
  documentation or be removed
- `crates/app/src/entry_point.rs` — current eight-way public dispatch
- `crates/app/src/config_setup.rs` — current setup-value inventory
- `crates/ops/portfolio-collect-report-op` — current mutable child-vector/count-readiness composition
- `crates/ops/evm-collectors-op` — current native-only EVM operation
- `crates/states/evm` — current EVM joint-tip/native fact state boundary
- `crates/evm-capabilities` and `crates/transports/evm` — reusable exact-block call support
- `crates/states/portfolio` and `crates/adapters/portfolio` — current historical fact selection
- `crates/ops/evm-contract-lifecycle-op` — operation surface deleted by this RFC
- `crates/states/evm-contracts` and `crates/adapters/evm-contracts` — retained state/adapter boundary
