# Implementation design: simplify MFM at semantic boundaries

**Status:** proposed implementation contract after adversarial architect review; not implemented.
**Reviewed production checkout:** `50cf01e8`.
**Architectural authority:** [RFC_REFACTOR_SIMPLIFICATION.md](RFC_REFACTOR_SIMPLIFICATION.md).
**Experimental evidence:** [the corrected construction experiment](EXPERIMENT_REFACTOR_SIMPLIFICATION.md#8-focused-follow-up-validation), candidate `cb2a95ff`.

This document fixes implementation decisions for the first complete Portfolio collection vertical,
the subsequent private Effect cutover, and the shared construction boundary those changes lead to.
It supplements the RFC rather than creating another architecture. Current production contracts in
[docs/design.md](docs/design.md) and placement in [docs/architecture.md](docs/architecture.md) remain
authoritative until the corresponding coherent implementation changes them and their tests together.

The code and wire sketches below specify the target, not a compiled API or executed acceptance
fixture. Generated schema digests must come from the implemented checked definitions. Agreement
among reviewers is design review; only the named consuming tests can establish implementation claims.

## 1. Scope and sequence

Use the existing RFC implementation order. Independently complete error-custody and reporting
corrections precede the semantic cutovers where they are prerequisites. The next three architectural
cutovers are:

| Cutover | Complete responsibility change | Construction API in that commit |
| --- | --- | --- |
| A. Portfolio collections | Admission, one collection Read, native observation, progress, snapshot/enrichment, configuration/publication, public product rendering and all consumers | The current single compiler/association path |
| B. Private transaction protocol | One semantic deployment/configuration Effect, private nonce/signing/wire custody, exact native settlement correspondence and all consumers | The same current compiler/association path |
| C. Direct construction | Consuming builder, contributed Catalog, owner-bound adapters, exact descriptors, recovery/checkpoints and every authoring/cold-loading consumer | One replacement construction/association path; delete the current DSL |

Cutovers A and B remove their superseded semantic protocols completely. They do not install a new
builder beside the old compiler. Cutover C replaces the shared API across all consumers together;
one representative consumer is a validation oracle, not an exemption for other consumers.

Sparse recovery usage, Store/PostgreSQL simplification, configuration pagination/transactions and
public resource-free historical observation retain their separate RFC cutovers. C must establish
the pure qualification/attachment separation needed by historical observation; exposing that use
case across Runtime/Application remains the later inspection cutover.

The ZEC/NEAR/Aave example stays a design exercise. No new live protocol, wallet, bridge, token
deployment fixture, distributed scheduler, signer process restart or production performance claim
is included here.

## 2. Contracts for cutover A

### 2.1 Ownership

| Owner | Owns | Boundary constraint |
| --- | --- | --- |
| Chain | Ordered collection Request, Holding, Observation and domain Failure | No Portfolio continuation, native protocol dispatcher or provider handle |
| EVM domain | Public collection Binding, native Receipt/Fault and pure native qualification/projection | No configuration repository, Runtime continuation or IO authority |
| Live EVM | Private anchored collection IO, existing transport/ABI codecs and explicit provider resources | Receives one collection Request and its Binding; no Portfolio policy or progress |
| Portfolio | Checked progress, one `CollectHoldings` State, snapshot/enrichment projections | Does not parse EVM addresses or perform provider IO |
| Application | Product config admission, current compiler composition, publication and public product conversion | Native decoding delegates to the native owner; selected configuration never refreshes an admitted run |
| Runtime / Journal / Store | Existing call/original custody, transitions, exact frames and append acknowledgements | No network/protocol branch or collection-owned recovery engine |

Move Portfolio-specific code out of `crates/live/evm/src/client/portfolio*` into Application.
Pure EVM public-value codecs stay in the EVM owner. Live EVM must lose its Portfolio dependency.
The temporary current-compiler source composition and resource source alias belong to Application,
so Live does not acquire a reverse dependency on Application.

### 2.2 One progress value

Names below identify the chosen shapes. Keep fields private and route direct construction and
deserialization through the same checked invariants. Existing checked identities are reused;
correlation does not need a new public identity framework.

```rust
struct PortfolioProgress {
    portfolio_id: PortfolioId,
    admission: Option<PortfolioAdmission>,
    remaining: Vec<CollectionDemand>,
    completed: Vec<CompletedCollection>,
}

struct CollectionDemand {
    correlation: String,
    binding: Object, // exact public native collection Binding
    sources: Vec<DeclaredSource>,
}

struct DeclaredSource {
    source: BalanceSource,
    required: bool,
}

struct CompletedCollection {
    correlation: String,
    binding: Object,
    observed_at: ObservationPoint,
    holdings: Vec<ProductHolding>,
}

struct ProductHolding { // private Portfolio detail; removed by terminal projection
    required: bool,
    holding: Holding,
}

struct Holding {
    source: BalanceSource,
    raw_units: Unsigned256,
    denomination: u8,
}
```

`PortfolioProgress::new` admits nonempty ordered demands with `completed` empty. Subsequent progress
values move a demand into completion; they contain no immutable input root, caller generic, next
ordinal, active status, duplicate count or separate required-source ID set. `remaining.first()`
selects the active demand; `completed.len()` supplies a product ordinal when rendering needs one.

The private `ProductHolding` retains the required bit until selection without repeating source
fields. Do not retain `CompletedCollection { demand, observation }`: that would repeat the entire
source plan in both branches. Expected source identity is checked against the transient observation,
then moved into the one completed Holding. The private wrapper is discarded in terminal output.

Make `PortfolioId` construction checked with private storage; its current public String field must
not bypass admission. Retain existing public text and Object limits.

### 2.3 Present-value invariants and transition invariants

Shared constructors and postdecode checks enforce facts available in their own values:

- Between one and 64 collections across `completed + remaining`, with nonempty source/holding lists.
- At most **64 total sources across the Portfolio**, not 64 sources per collection multiplied by 64.
- Unique correlations and globally unique source IDs across both parts.
- Checked source identities/targets, public text, unsigned-256 quantities and denominations `0..=255`.
- One exact ledger per collection and agreement between completed targets and their common point.
- Valid nested Object envelopes/commitments and shared ledger equality. The native payload remains
  opaque to Portfolio; its decoder must not depend on EVM or a generic validator context.

Application admission, selected native request/receipt checks and publication explicitly invoke the
native owner's codecs for supported schemas, Binding/source-instance agreement and native
point/descriptor correspondence. Those checks occur at their owning boundaries; do not claim the
generic progress decoder performs them or that an opaque Object shell certifies native meaning.

There is no duplicated original demand plan against which a standalone decoder can prove historical
completeness or order. A self-consistent omission/reordering can be locally valid. Do not add a root
copy, ordinal or status certificate and call that history authentication.

The State transition establishes the stronger relationship against its actual input:

1. Prepare the front demand's Request.
2. Check that the prepared canonical Request still matches that demand.
3. Qualify the complete observation against the ordered requested sources.
4. On success, remove exactly that front demand, move correlation/Binding/source metadata into one
   completion, append it once, and preserve the old completed prefix and remaining suffix.
5. On refusal, propose domain failure without modifying progress.

Application constructs exactly one Read occurrence per admitted demand and one terminal projection.
Runtime's exact initial commitment, trusted transitions and Store's append-only history supply their
separate continuity guarantees. Current-value validation does not replay historical execution.

### 2.4 Semantic ports

```rust
struct CollectionRequest {
    binding_ref: ContentRef,
    sources: Vec<BalanceSource>,
}

struct ObservedHoldings {
    observed_at: ObservationPoint,
    holdings: Vec<Holding>,
}

enum CollectionObservation {
    Complete(ObservedHoldings),
    Refused(CollectionFailure),
}

enum CollectionFailure {
    NetworkMismatch,
    AnchorReplaced,
    AnchorUnavailable,
    RequiredFieldUnavailable,
}
```

Request construction checks the nonempty ordered source set, uniqueness, common exact ledger and
Binding commitment. `prepare` derives it from the active demand, stripping required flags, product
identity, admission and continuation. No second durable native request is introduced:
the current compiler's native intent is exactly `CollectionRequest` with identity translation.

`CollectHoldings` is endomorphic: Input and Output are `PortfolioProgress`; Failure is
`CollectionFailure`. Under cutover A, it implements the existing Read/capability contracts.
Observation retains the current API's required value derives until C removes the framework's
mandatory Observation serialization/schema requirement. Actual persisted product values keep
their own derives.
Current callbacks may still repeat pure identity translation/encoding when checking Read evidence.
The claim for A is one semantic Request and no second durable native-request representation;
removing the obsolete generic translation path belongs to C.

The four failure variants are the intrinsic semantic reasons, with no separate classification flag,
collection ordinal, whole input or native receipt copy. Runtime's domain call retains the actual
progress/Request/native original; Application derives correlation and ordinal from those facts.

| Failure | Classification | Reason |
| --- | --- | --- |
| NetworkMismatch | Permanent | The selected endpoint does not supply the declared supported instance |
| AnchorReplaced | Retryable | A new attempt can select a fresh anchor without changing the admitted input |
| AnchorUnavailable | Retryable | The unchanged Request can succeed on a later observation |
| RequiredFieldUnavailable | Permanent | The selected supported token contract requires the unavailable ABI field |

The new Request contains **no selected anchor**. Therefore inherit no `InputInvalidated` classification
or checkpoint restart from the retired staged input. Default Stop and zero allowances remain.
An explicitly selected retry policy repeats only the unfinished collection with a fresh anchor.
Waiting for future token deployment/upgrades is outside the fixed required-ABI policy.
`RequiredFieldUnavailable` concerns the native protocol's required ABI, not the product's
`DeclaredSource.required` flag. A failed optional candidate still fails the all-or-nothing collection;
the flag only affects terminal enrichment selection.

### 2.5 One output, two projections

Both terminal States return one checked `PortfolioHoldings`:

```rust
struct PortfolioHoldings {
    portfolio_id: PortfolioId,
    collections: Vec<HoldingsCollection>,
}

struct HoldingsCollection {
    correlation: String,
    binding: Object,
    observed_at: ObservationPoint,
    holdings: Vec<Holding>,
}
```

`SnapshotProjection` requires `remaining` empty and moves every Holding into output.
`EnrichmentProjection` requires `remaining` empty, selects `required || raw_units != 0`, and strips
the required bits. Both have `Never` as domain Failure; invoking them prematurely or violating
their admitted construction invariants is an internal invocation failure.

Retain the current EVM enrichment admission rule: each collection has a required native source.
The generic product invariant is required-source coverage; the native admission owner establishes
which supported source is required. Snapshot admission can accept token-only collections. Filtering
must produce a nonempty collection under the admitted enrichment rule; an impossible empty result
is an invariant failure, not a new empty Request or silently dropped collection policy.

Do not add a mode enum to Progress/output to duplicate the selected terminal State and entry point.
Remove quotes, target scale, totals and valuation reports. JSON output retains raw decimal quantity
strings and actual denominations. Human rendering can insert a decimal point using string operations;
it needs no powers-of-ten arithmetic, rounded value or additional persisted display amount.

Native and token denomination are plain `u8` throughout this path. The old `0..=30` bound supported
deleted scaling arithmetic; it does not constrain honest ERC-20 metadata. Test 31 and 255 as supported
observations and ABI value 256 as an owned range rejection. Preserve reusable arithmetic types
where another real consumer still requires them.

The CLI/REST run-observation envelope remains owned by Application's current transport contract.
Its successful `product` field has one shared presentation, for both entry points:

```text
{ portfolio_id, collections: [
  { correlation, binding: public EvmCollectionBinding,
    anchor: { number: decimal string, hash: EvmHash },
    holdings: [ { source_id, address: EvmAddress, token: EvmAddress | null,
                  raw_units: decimal string, denomination: integer 0..255 } ] }
] }
```

The `binding` JSON is the same checked native body used in §3.1, not a locator or a second Object
envelope. Publication uses the canonical persisted `PortfolioHoldings`, not presentation JSON.
Rendering qualifies native ledger/point/Binding correspondence through the EVM owner, preserves
collection/source order, and emits null for native token. There is no snapshot/report wrapper,
display amount field, sum, quote or mode. Human amount formatting is computed from the two raw
fields. A literal holding example is:

```json
{
  "source_id": "main-token",
  "address": "0x1111111111111111111111111111111111111111",
  "token": "0x2222222222222222222222222222222222222222",
  "raw_units": "1500",
  "denomination": 3
}
```

Domain refusal is the existing RunView failure/original envelope containing the exact selected
`CollectionFailure` and native Receipt. Application derives any public correlation/ordinal from
the retained Read input; add no second product failure/report family. Operational failure retains
the selected native Fault through that same existing envelope. The reporting prerequisite in §1
owns deletion of the old failure report, independently of this output change.

### 2.6 Publication and provenance

The retained public collection Binding plus selected source targets/ordering and point suffice to
regenerate exact source configuration. Do not retain the original config, per-source copies of the
same Binding, full native batch receipts or additional request/receipt/code-reference tuples in
every product row. Existing Read frames retain exact native originals, and Program retains selected
implementation provenance. Lifecycle evidence has different real requirements and retains originals.

A shared output schema removes the old enrichment-only schema distinction. Both
`publish_enrichment` and dependent provenance qualification must check:

1. A qualified Succeeded RunView for the supported **enrichment entry point**.
2. The exact selected `PortfolioHoldings` output codec and its checked local invariants.
3. Existing exact RunId, terminal head and output-reference correspondence.
4. Retained initial admission's entry point agrees with the qualified Program/RunView; source name
   and digest remain retained admission facts, requiring no reload of the original configuration.
5. Regenerated snapshot body matches the retained full Bindings and ordered selected source facts.

Dependent provenance qualification checks the referenced enrichment RunId/head/output and entry
point, then compares the dependent snapshot body with this pure regenerated body, excluding its
attached provenance envelope and target revision name. Original enrichment admission digest and
new snapshot config digest normally differ; never require their equality.

A snapshot with the same output shape cannot be published as enrichment. These are supported
Application contracts under trusted installed code, not attestation against an arbitrary in-process
author deliberately mislabeling a Program. Add no mode wrapper, Program whitelist or new authority.
Provenance qualification does not re-evaluate selection against candidates that the output no longer
contains. The trusted enrichment terminal State and qualified entry point establish that policy;
retained output establishes the exact selected configuration.

Publication reads retained output and pure native public codecs; it performs no provider calls or
config rediscovery. The first cutover still uses existing executable association and may require
configured resource tables. It does not establish attachment-free historical observation.

## 3. Native EVM collection implementation

### 3.1 Public Binding, configuration and resources

```rust
struct EvmCollectionBinding {
    route: EvmBalanceRoute,
    native_denomination: u8,
}

struct EvmBalanceRoute {
    chain_instance: EvmChainInstance, // existing chain ID + expected genesis vocabulary
    endpoint: EvmEndpoint,           // public endpoint identity, not private URL/credentials
}
```

Each demand/completion/output retains its exact public `binding: Object` once. There is no
`CollectionExecutionConfig` wrapper or repeated stored Binding ref. Preparation derives
`CollectionRequest.binding_ref` from that Object's exact `value_ref()`, committing the entire
`EvmCollectionBinding`, including denomination. It must equal the separately selected occurrence's
admitted Binding reference. This is distinct from physical resource lookup by `EvmTransactionRoute`;
never compare their hashes or introduce another route-reference field. Every requested source
ledger carries the exact existing
`EvmChainInstance` native Object. Native target codecs decode account and optional token. Qualify
all source schemas, identities, route and binding before the first provider call.

Application configuration places the public Binding once per collection and source ID/account/token
once per source. One root Portfolio ID selects the product. Remove the obsolete quote selector,
target scale, per-source chain-ID copies and separate route/selector graph. Native admission derives
source ledgers from the explicit complete Binding and builds its exact Binding commitment.
It never learns the expected genesis from the endpoint it is about to validate.

Live resource matching uses complete `EvmTransactionRoute { chain_instance, endpoint_ref }`
vocabulary, replacing the chain-ID-only `EvmPhysicalTarget`. Equal chain IDs with different genesis
facts must not alias. Collection construction checks its selected exact public Binding; lifecycle
anchored calls continue to resolve their complete existing route.
Affected identity/codec helpers must preserve their original errors. In particular, do not inherit
the current `EvmTransactionRoute::binding_ref` conversion to a bare `EvmDomainError::Program`;
derive the checked Object/ref with a source-preserving conversion or repair that helper and its
affected callers in the same cutover. This does not authorize an unrelated global error rewrite.

`native_denomination` is explicit committed operator metadata; reviewed Ether fixtures use 18.
It is not a provider-observed EVM fact. No default 18, currency registry or chain-name branch supplies
that guarantee for arbitrary networks. Token denomination is separately observed at the selected anchor.

Freeze the first-party configuration as an Application-owned, strictly checked wire:

```text
EvmPortfolioConfig = { entry_point, input }
input = { portfolio_id, collections: [CollectionConfig], provenance: optional EnrichmentProvenance }
CollectionConfig = { correlation, binding: EvmCollectionBinding, sources: [SourceConfig] }
SourceConfig = { source_id, address: EvmAddress, token: optional EvmAddress }
```

The only entry points are `mfm.portfolio/snapshot@2` and `mfm.portfolio/enrich@2`. All structs reject
unknown fields. Missing or null `token` means native; a non-null token is a checked token address.
Missing or null `provenance` means absent; only snapshot accepts present provenance and verifies it
through §2.6. Serialization emits both optional keys, using null when absent. The existing config
digest commits the canonical submitted JSON, so omitted and null spellings can have distinct source
digests; no new input-normalization or digest framework is introduced. Regenerated publication has
one deterministic serialized spelling. `required` is derived during admission and is not a config key.

This synthetic public fixture exercises two networks with the same implementation; its hashes and
addresses declare test inputs, not verified live deployments:

```json
{
  "entry_point": "mfm.portfolio/enrich@2",
  "input": {
    "portfolio_id": "example",
    "collections": [
      {
        "correlation": "main",
        "binding": {
          "route": {
            "chain_instance": {
              "chain_id": 1,
              "expected_genesis_hash": "0x1111111111111111111111111111111111111111111111111111111111111111"
            },
            "endpoint": { "endpoint_id": "main-public" }
          },
          "native_denomination": 18
        },
        "sources": [
          { "source_id": "main-native", "address": "0x1111111111111111111111111111111111111111", "token": null },
          { "source_id": "main-token", "address": "0x1111111111111111111111111111111111111111", "token": "0x2222222222222222222222222222222222222222" }
        ]
      },
      {
        "correlation": "other",
        "binding": {
          "route": {
            "chain_instance": {
              "chain_id": 10,
              "expected_genesis_hash": "0x3333333333333333333333333333333333333333333333333333333333333333"
            },
            "endpoint": { "endpoint_id": "other-public" }
          },
          "native_denomination": 6
        },
        "sources": [
          { "source_id": "other-native", "address": "0x4444444444444444444444444444444444444444", "token": null }
        ]
      }
    ],
    "provenance": null
  }
}
```

### 3.2 Application assembly under the current compiler

Cutover A uses the current `OperationDefinition`/`Plan`/`Read`/`Pure` vocabulary. Application's
snapshot and enrichment definitions each lower to this shape:

```text
Body = (Vec<Operation<CollectionDefinition>>, Pure<SelectedProjection>)
CollectionDefinition::Body = Read<CollectHoldings, CollectionRead>
```

`CollectionRead` is the new shared capability; EVM contributes the single current implementation
and constructor. `CollectionDefinition` retains only occurrence configuration needed by the current
compiler to select the exact Binding. It adds no State, copied demand root or runtime continuation.
Application owns the two operation definitions and this composition alias:

```rust
type PortfolioResources = EvmResources<(SnapshotOperation, EnrichmentOperation)>;
```

The assembly workflow is:

```text
parse/admit config and provenance without IO
  → build checked PortfolioProgress and native Binding Objects
  → resolve public physical routes against explicit operator provider tables
  → current compile(entry, selected Operation, &progress, &resources, limits)
  → Runtime.start(caller RunId, complete Program, &progress)
  → Read(collection 1) → Read(collection 2) → selected terminal Pure projection
```

Canonical Binding Objects select declarations; the physical provider table does not set product
policy or refresh admitted values. Both networks use the same native implementation. The existing
`Sources` parameter remains only because the current compiler requires it; C deletes it and these
operation definitions together. Move the alias out of Live so native resource code never imports
Application or Portfolio composition. Preserve current limits/default Stop during assembly.

### 3.3 One existing provider interface

Replace the broad stage-based balance observer on `EvmReadProvider` with:

```rust
fn collect<'a>(
    &'a self,
    binding: &'a EvmCollectionBinding,
    request_ref: &'a ContentRef,
    request: &'a CollectionRequest,
) -> ProviderFuture<'a, EvmCollectionReceipt, EvmCollectionFault>;
```

Extend the existing future alias with a defaulted error parameter if needed; do not add a parallel
provider framework. Retain the existing anchored contract-call facet and transaction provider
primitives. The bound adapter forwards its qualified Binding and canonical Request. Passing only
Request would lose the declared native denomination; passing PortfolioProgress would cross ownership.

The JSON-RPC provider owns the private sequential collection algorithm and reuses existing `rpc`,
block selector, ABI decoder and cause-capture functions. No provider concurrency, retry loop,
deadline policy or new transport layer is introduced.

### 3.4 Exact native Receipt

```rust
struct EvmCollectionReceipt {
    request_ref: ContentRef,
    observed_chain: EvmChainInstance,
    outcome: EvmCollectionOutcome,
}

enum EvmCollectionOutcome {
    Complete { anchor: EvmBlockAnchor, values: Vec<EvmCollectedAmount> },
    WrongChain,
    NoInitialAnchor,
    AnchorUnavailable { selected: EvmBlockAnchor },
    AnchorChanged { selected: EvmBlockAnchor, observed: EvmBlockAnchor },
    UnavailableField { context: UnavailableTokenField },
}

struct EvmCollectedAmount {
    raw_units: Unsigned256,
    denomination: u8,
}

enum UnavailableTokenField {
    Decimals { anchor: EvmBlockAnchor, source_index: u8 },
    BalanceOf { anchor: EvmBlockAnchor, source_index: u8, observed_denomination: u8 },
}
```

The successful vector has exactly the requested length. Request reference and array position provide
association; add no serialized ordinal, account, token or copied source plan to each success row.
RPC balance replies contain scalars, not independent attestations of the queried account. Independent
provider-argument tests establish which target was queried. Pure validation cannot detect an arbitrary
permutation of otherwise valid amounts when there is no independent source evidence for that claim.

Selected owner checks enforce request-ref equality, exact ledger/route, complete count, native unit
agreement, token-field eligibility and native outcome relationships. `WrongChain` requires an actual
identity disagreement. `AnchorChanged` requires equal block number and different hash. Source indexes
for field unavailability must identify a token source. Token denomination observed before failed
`balanceOf` has no independently expected value in Request: validate width/field eligibility and
retain it exactly, rather than inventing a unit-agreement check.
Malformed or inconsistent admitted native Objects are internal failures; no serialized qualified flag
replaces these checks.

The Receipt is the declared exact native original, not a dump of every HTTP response or successful
intermediate RPC. Preserve it once in the existing Read completion record. Failed observations have
their own declared original Fault or rejection Receipt; successful intermediate quantities do not
become partial progress or another fault payload.

### 3.5 Protocol

1. Qualify the complete Request against the Binding locally: zero IO on any mismatch.
2. Read `eth_chainId`; read genesis with `eth_getBlockByNumber("0x0", false)`, requiring block number
   zero. Compare the observed chain instance to the independently declared expectation. An actual
   mismatch returns `WrongChain` and makes no balance calls.
3. Select `latest` **once**. A successful null response returns `NoInitialAnchor`.
4. In declared source order, use the exact selected selector
   `{ "blockHash": selected.hash, "requireCanonical": true }` for every state read:
   - Native: `eth_getBalance(account, selector)`; use the declared native denomination.
   - Token: `eth_call(decimals(), selector)`, then `eth_call(balanceOf(account), selector)`.
5. Validate supported response shape. Token ABI values are exact 32-byte words; denomination must
   fit `u8`, units must fit `Unsigned256`. Empty return data is `UnavailableField`, meaning the
   requested ABI field was unavailable. It does **not** prove contract absence or authorize a
   guessed denomination. Malformed data is an operational owner Fault with its cause.
6. Read the selected block **number**, requiring returned number agreement. A successful null
   response is `AnchorUnavailable`; a different hash at that number is `AnchorChanged`; the same
   hash completes the Receipt even if latest has advanced.

A returned-number disagreement is a native protocol Fault. Retain full-width expected and observed
anchors in the existing `RpcRejection::AnchorMismatch` or reviewed owner cause; do not truncate
block numbers to `u64` or compare against latest instead.

Do not fall back to numbered/latest state reads. Number bracketing can miss an ABA reorganization;
the selected hash contract is deliberate. [EIP-1898](https://eips.ethereum.org/EIPS/eip-1898)
defines the canonical hash selector. ERC-20 denomination metadata is optional, so the selected
supported token-read contract explicitly refuses unavailable required metadata rather than guessing.
[ERC-20](https://eips.ethereum.org/EIPS/eip-20)

Generic RPC error codes do not prove reorganization, absence or unsupported capability. Retain the
actual owner error. Supported provider acceptance must verify both `eth_getBalance` and `eth_call`
selector behavior; failure rejects that supported provider contract without a fallback.

The current confirmation step already rereads the selected number. The existing ordinary-head-growth
problem arises because each source selects `latest` again and compares it with the first source's
anchor. The new protocol selects once; tests must advance latest **between sources**, not merely
before final confirmation.

### 3.6 Fault context and category

`EvmCollectionFault` owns the concrete existing `EvmOperationalError` cause and one tagged context:

```text
ChainId
Genesis { observed_chain_id }
SelectAnchor { observed_chain }
NativeBalance { observed_chain, anchor, source_index }
TokenDecimals { observed_chain, anchor, source_index }
TokenBalance { observed_chain, anchor, source_index, observed_denomination }
ConfirmAnchor { observed_chain, anchor }
```

This retains facts already established before the failing attempt, including chain ID when genesis
fails and denomination when token balance fails. Preserve the complete available cause ancestry,
operation and reviewed parser/RPC fields under the existing dependency-text trust contract. Do not
append MFM secrets, private locators, full requests or connection objects.

Fault classification delegates to its concrete existing owner cause. No repeated request ref,
serialized classification, completed-quantity prefix or generic incident bag is added. Intrinsic
decoding checks context shape and supported index bounds. Current fault classification does not
receive the Request, so do not promise extra source-index/request-length or binding authentication
and do not add a universal cold Fault hook merely to justify a redundant field.

| Condition | Result | Recording |
| --- | --- | --- |
| Local request/binding/native codec disagreement | InvocationDiagnostic/Internal | No provider call or operational outcome append |
| Remote transport/parser/range failure | EvmCollectionFault with context and concrete cause | Existing failed Read append; acknowledge before classification |
| Observed wrong chain/genesis | Rejection Receipt → NetworkMismatch | Exact native facts retained in domain-failure Read completion |
| Actual selected-hash replacement | Rejection Receipt → AnchorReplaced | Same custody; retry is whole collection |
| Successful null anchor observation | Rejection Receipt → AnchorUnavailable | Preserve absence semantics; invent no reorganization |
| Empty required token ABI field | Rejection Receipt → RequiredFieldUnavailable | Preserve known field/source/anchor facts; invent no absent contract |
| Impossible admitted Receipt/projector relationship | InvocationDiagnostic/Internal | No invented native incident or successful completion |

## 4. Workflow, cancellation and cold limits

### 4.1 Read acknowledgement ordering

```text
checked progress → prepare Request
  → encode/admit/decode its authoritative canonical Object
  → selected local Request/Binding check
  → native collection IO
  → encode native Receipt once → admit/decode that same Object
  → selected pure projection + State interpretation in one completion callback
  → append exact native original and proposed success/domain failure together
  → Store acknowledgement → advance or classify/recover
```

A Read has **no receipt-only acknowledgement before interpretation**. That boundary belongs to
Effects. An operational Fault instead follows original encoding/admission → failed append →
acknowledgement → classification/policy. First-original encoding failure reports its cause, known
context and unavailable original identity/detail; it never retries the serializer.

Cancel before completion acknowledgement: the entire unfinished collection may repeat when no
completion is retained. Cancel during append: the physical append may have committed without the
caller receiving acknowledgement; use existing exact-head/ambiguity semantics. Cold load of an
already committed completion advances without re-observation. Previously acknowledged collections
remain authoritative and are not reread. Interrupted RPC attempts have no claimed per-RPC audit record.
Recording failure is reported as recording failure, never as successful audit through that failed Store.

### 4.2 Current versus target cold guarantees

Cutover A retains the current complete executable association and selected schema admission.
Current `Runtime.read` does not generally decode/project every native Receipt against Request and
Binding. Do not claim that any well-shaped, rehashed cross-slot substitution is rejected by that API.
Test pure owner constructors/projectors directly and actual cold retry/output/publication paths.
Read completion is atomic, so there is no cold replay of an acknowledged Read's interpretation.
Typed Progress/publication decoding supplies its own local constructor checks.

C installs actual typed codec validators and shared static native validators; the later inspection
cutover exposes them without resource attachment. Neither performs generic historical replay,
recomputes arbitrary State outputs nor certifies every unselected historical row. Semantic revisions
remain trusted code contracts, not executable-byte authentication.

## 5. Freeze the subsequent private Effect boundary

Cutover B uses current capability/implementation association. Change `TransactionEffect<R>`'s
Command to the semantic recipe R. Deployment/configuration consume `DeploymentRequest` and
`DeployedContract`; the native owner privately derives the nonce-free EIP-1559 command/ref.
Retain current `implementation_ref` provenance until C changes it atomically to `leaf_ref`.

| Step | Owner / invariant |
| --- | --- |
| Prepare/check | Deterministic semantic Command, authoritative canonical decoding, complete Binding agreement before command append |
| Command acknowledgement | Runtime derives existing EffectId from RunId/Program/State+visit/semantic command ref; establishes barrier; waits for acknowledgement before native IO |
| Private custody | Derive native command/ref only from admitted terms/Binding; load/reserve, sign if no retained winner, retain first winner, qualify it, submit/reconcile only exact retained bytes |
| Settlement | Encode original once; decode/project it before append; acknowledge settlement before interpretation |
| Settled cold interpretation | Reproject purely from retained command/Receipt/Binding/code identity, then interpret; no provider, signer or authority IO |
| Unresolved resume | Reprepare and compare the acknowledged semantic command under the existing contract; load corresponding authority, never refresh admitted terms or replace command identity |
| Recovery | Pending retry preserves exact command authority; Stop retains RecoveryStopped; settled Effect cannot retry/restart; checkpoints cannot cross acknowledged barrier |

Retain `NonceDomain`, `Reservation`, `PreparedRecord`, `ExactRawTransaction`, authority ports and
their existing append/ambiguity contracts. Delete public reservation/preparation States, prepared
context wrappers and injection stages physically; do not merely privatize their codecs.

Uncertain reservation/retention acknowledgement resolves through existing exact committed-or-absent
loading. It cannot authorize submission of an unacknowledged candidate or replacement of a retained
winner. Returned Faults retain the actual causal chain and known stage, native command ref, nonce
and winning hash where available. Add no diagnostic IO, duplicate semantic Command/Binding,
successful-intermediate log or second private progress representation.

Add `native_command_ref` directly to revised `EvmTransactionSettlement` alongside existing EffectId,
nonce, receipt and outcome. Do not install the specimen's `CustodyReceipt` wrapper in production.
Pure checks can verify native ref, EffectId, action shape and deterministic CREATE address from
admitted sender plus retained nonce. They cannot independently prove hash-to-wire correspondence
after the winning wire is unavailable; live custody supplied that guarantee before acknowledgement.

Freeze remote disagreement as the **existing**
`EvmTransactionOperationalError::Provider { operation, cause: EvmOperationalError }`, classified
`OutcomeUnknown`. Extend reviewed `RpcRejection` facts, not the outer Fault hierarchy:

| Remote observation | TransactionProviderOperation / retained rejection |
| --- | --- |
| Wrong chain/genesis | VerifyChain / ChainInstanceMismatch { expected, observed: complete EvmChainInstance } |
| Receipt hash or sender disagreement | Receipt / ReceiptIdentityMismatch { expected_hash, observed_hash, expected_sender, observed_sender } |
| Receipt target/action shape disagreement | Receipt / ReceiptActionMismatch { expected_target, observed: ProviderReceiptResult } |
| Wrong successful CREATE address | Receipt / CreatedAddressMismatch { expected, observed } |
| Wrong submit-returned hash | Submit / existing HashMismatch |
| Canonical receipt block disagreement | CanonicalBlock / existing AnchorMismatch |

Move the existing four-variant `ProviderReceiptResult` from Live into EVM once and reuse it in
provider success and retained rejection. Delete the superseded remote-only AdapterFailure variants;
do not add a mirrored outcome enum. `ProviderFailure` retains RPC method/field, Validation stage and
`Rejected { cause: RpcRejection }`, preserving the source chain beneath the transaction operation.
Add RpcField variants ChainInstance, ReceiptIdentity and CreatedAddress; reuse ReceiptOutcome for
action/target. A chain-ID disagreement names ChainId; a genesis-only disagreement names
GetBlockByNumber. Typed successful remote facts have no invented upstream exception: response text
unavailable/not retained and empty upstream causes are explicit, rather than a fabricated cause or
full request.

The same pure fact checker returns these reviewed causes. Ingress wraps them as Provider Faults;
local/stored impossible disagreement converts the same available cause to Internal. Cold checking
only rechecks facts actually retained. Existing lossy/internal remote mappings are gaps to close.
`OutcomeUnknown` never claims an externally settled rejection: it preserves acknowledged semantic
command/native-ref/winner authority and the existing policy schedule. StandardRecovery Stop leaves
RecoveryStopped with that unresolved authority; explicit supported resume reconciles the same
command and bytes. It does not replace them or infer that no mutation occurred.

Constructor tables may be !Send/!Sync; capture only execution-safe handles. Keep Keystore owning
thread and actor lifetime unchanged. Reconstruction with that same owner is not process restart.

## 6. Shared construction contract for cutover C

### 6.1 One Catalog and one authoring surface

```text
Catalog::register_value<V>()                    -> Result<()>
Catalog::register_pure<S>()                     -> Result<()>
Catalog::register_read<S,A>()                   -> Result<ReadLeaf<S>>
Catalog::register_effect<S,A>()                 -> Result<EffectLeaf<S>>
Catalog::register_handler<H>()                  -> Result<()>
Catalog::select_read<S>(&exact_leaf_ref)        -> Result<ReadLeaf<S>>
Catalog::select_effect<S>(&exact_leaf_ref)      -> Result<EffectLeaf<S>>
Catalog::select_handler(&HandlerAbi,&Object)   -> Result<HandlerBinding>
Catalog::qualify(&Object)                      -> Result<ProgramDocument>
Catalog::load(&Object,&Resources)              -> Result<Program>
ProgramDocument::attach(&Resources)            -> Result<Program>
Recovery::new(HandlerBinding,RecoveryAllowances)-> Recovery
recovery.with_restart<T>(&Checkpoint<T>)       -> Recovery

ProgramBuilder<C>::new(&C)                     -> Result<ProgramBuilder<C>>
builder.pure::<S>(Recovery)                    -> Result<ProgramBuilder<S::Output>>
builder.read(&ReadLeaf<S>,&Object,Recovery)     -> ProgramBuilder<S::Output>
builder.effect(&EffectLeaf<S>,&Object,Recovery)-> ProgramBuilder<S::Output>
builder.checkpoint()                          -> Checkpoint<C>
builder.finish(entry,&Catalog,&Resources,limits)-> Result<Program>
```

Each append requires `S::Input = C`. Pure derives fallible metadata/ref immediately: no PureLeaf
wrapper or deferred resolver callback solely to make its signature look uniform. Read/Effect
appends clone already checked handles and canonical Binding Objects; encoding does not occur
there, so they retain no obsolete Result. Finish performs complete qualification and capacity checks.
`load` is strictly `qualify` followed by `attach`, not another loader. Typed restart selection retains
private checkpoint origin/owner claims until finish; fresh callers do not construct erased positions.

`new(&input)` encodes once, admits and decodes that canonical input with its actual owner, then
retains only initial contract/ref and private owner claim. Caller keeps input for existing Runtime
start; Runtime rejects substitution before admission/IO. Every append preserves initial ownership
and updates terminal ownership. Zero-State Programs qualify both against independently contributed
value codecs, without a synthetic Identity State or extra start API.

Leaf registration is atomic: derive and check all codec/factory claims before changing Catalog.
Identical same-owner value publication is idempotent because leaves share values; duplicate or
conflicting leaf/handler publication rejects without replacing anything or leaving codec contributions.
Inspection derives from those same entries. Registration computes the complete intrinsic descriptor
ref once; immutable exact-key selection cannot fall back to family ID or nominal schema alone.

### 6.2 Direct adapter ports and resources

Keep the RFC's State prepare/interpret and direct native ports. Each Read/Effect adapter owns
associated `Resources`, `Binding`, `Receipt` and `Fault`. The exact reference flow is:

```text
ReadAdapter<Request, Observation>:
  bind(&Resources, binding_ref, &Binding) -> Result<Self, InvocationDiagnostic>
  check_request(binding_ref, &Binding, &Request) -> Result<(), InvocationDiagnostic>
  observe(&self, request_ref, &Request) -> async Result<Receipt, AdapterError<Fault>>
  project(binding_ref, &Binding, selected_leaf_ref, request_ref, &Request, &Object, &Receipt)
    -> Result<Observation, InvocationDiagnostic>

EffectAdapter<Command, Observation>:
  bind(&Resources, binding_ref, &Binding) -> Result<Self, InvocationDiagnostic>
  check_command(binding_ref, &Binding, &Command) -> Result<(), InvocationDiagnostic>
  reconcile(&self, effect_id, command_ref, &Command)
    -> async Result<EffectAdapterOutcome<Receipt>, AdapterError<Fault>>
  project(binding_ref, &Binding, selected_leaf_ref, effect_id, command_ref, &Command, &Object, &Receipt)
    -> Result<Observation, InvocationDiagnostic>
```

The factory obtains `binding_ref` from the exact admitted Binding Object and pairs it with Binding
decoded from that same Object. Native owner checks compare their actual Request/Command meaning;
the generic factory needs no universal binding-ref accessor on arbitrary requests. Never re-encode
Binding to recover identity. A bound adapter captures that ref only when its actual IO/resource
checks need it. Registration takes neither an independent binder nor another resource generic.
Projection receives the factory-selected leaf ref, admitted request/command ref, exact original
Object and Receipt decoded from that same original Object; Effect additionally receives EffectId.
This canonical reference is not a universal physical-route ref. CollectionRequest explicitly commits
the full Binding; lifecycle requests can retain their different owner-defined physical-route checks.

Registration requires `A: ReadAdapter<S::Request, S::Observation>` or the corresponding Effect
relationship. It proves exact Observation equality. Program registration additionally requires
`A::Fault: ClassifyError`; the lower Capabilities contract requires Fault only to be `MfmValue`,
preserving dependency direction. Do not add blanket Send/Sync requirements on Observation merely
because native IO handles have execution bounds.

The explicit nongeneric Resources table borrows owner tables by private TypeId. Reject duplicate
table insertion. `Any` requires a concrete static owner type; borrowing the table does not require
its fields to be Send/Sync or permit arbitrary non-static resource-type erasure. Bind captures
authorized execution handles; provider/signer availability can subsequently change.

Fresh claims retain exact State/adapter/resource owners. Handler creation retains H **and Params
codec owner**. Initial/terminal codecs and selected checkpoint owners also qualify. None of these
private identities are persisted. Same refs in another Catalog do not authorize a different fresh
Rust implementation. Cold qualification instead relies on reviewed semantic revisions.

Observation has no framework MfmValue/Serde/schema requirement. Read constructs/consumes it inside
one pure completion job. Effect projects before settlement append, discards Observation, and
projects again purely after acknowledgement, hot or cold. No Any cache, Observation mirror,
serialized continuation or acknowledgement token is added. Lifecycle evidence still keeps derives
and originals where its actual product contracts require them.

### 6.3 Exact descriptor and Program wire

One checked `LeafDescriptor` has this shape:

```text
LeafDescriptor {
  domain: "mfm.leaf.v1",
  state: StableId,
  input: ContentRef, output: ContentRef, failure: ContentRef,
  execution:
    Pure
    | Read   { adapter: StableId, request, binding, receipt, fault: ContentRef }
    | Effect { adapter: StableId, command, binding, receipt, fault: ContentRef }
}
```

Its exact canonical content ref is the intrinsic key. The descriptor contains no Observation schema,
resource pointer, instance Binding, recovery parameter or extra implementation/capability identity.
Stored contract refs include actual generic schemas; StableIds include reviewed semantic revisions.
Behavior-changing hidden specialization requires a revised existing identity or checked committed
instance meaning. Do not add ObservationId or claim executable-byte authentication.

The target Program baseline is v10:

```text
ProgramWire {
  domain: "mfm.program.v10", entry_point_id,
  admitted_context_contract_ref, initial_value_ref, root_success_contract_ref,
  declarations: [StateDeclaration], bindings: [Object], limits
}

StateDeclaration {
  execution: Pure { leaf_ref }
           | Read { leaf_ref, binding_ref }
           | Effect { leaf_ref, binding_ref },
  handler: { abi: { implementation: ContentRef, params: ContentRef }, params: Object },
  recovery_targets: [StatePosition], allowances
}
```

The domain is exactly `mfm.program.v10`, revising the current `mfm.program.v9` baseline.
The Binding table is sorted, unique and exactly the selected set. Mode/binding selection is a sum
type; it has no correlated optional binding flag. Handler
parameters are canonical Object, replacing PolicyParams. Existing exact HandlerAbi supplies its
intrinsic identity; no HandlerLeaf or second descriptor hierarchy is needed.

Each leaf ref resolves the immutable complete installed descriptor. Do not repeat NativeAbi,
State implementation/input/output/failure fields or a descriptor payload table in Program wire.
The associated Program contains resolved contracts/callbacks; checked observation requires installed
code anyway. Bare bootstrap can expose retained bytes/refs when code is absent but cannot certify
adjacency or native meaning. Missing selected revision is an explicit qualification failure.

Reject incompatible v9 Programs under the new baseline; install no legacy decoder, compatibility
bridge or history rewrite. The experimental v9 bridge is evidence only.

### 6.4 Recovery and checkpoints

`HandlerBinding::new::<H>(&H::Params)` produces one canonical parameter Object and private H/Params
claims. Dynamic exact HandlerAbi selection obtains the installed owner claims. Both use the same
factory and owner-decoded canonical parameters at qualification, never pre-encoding parameter data.
Register Stop explicitly through the ordinary kernel contribution. Recovery has explicit handler,
allowances and selected restart targets; delete inherited defaults rather than hiding them in a wrapper.

`checkpoint()` creates a typed handle containing a private originating-builder token, boundary index
and current codec owner. It grants no Runtime authority. Selecting it as a target retains the claim
until finish checks origin, boundary, earlier/current position, exact target input owner and contract.
Reject foreign-origin, forward, terminal and duplicate targets before erasing to StatePosition.
An unused checkpoint has no persisted/runtime meaning.

The union of selected declaration targets is the checkpoint inventory, as in the current association
contract. No marker table, public origin ID, scope tuple or authoring-depth mechanism is needed.
Runtime retains active inputs, allowances, visits and dynamic Effect barriers and independently
authorizes restart. Sparse usage representation remains its separate cutover.

### 6.5 One pure qualification, then attachment

```text
retained Object / fresh builder draft
  → private canonical ProgramWire decode and structural validation
  → Catalog qualification of exact descriptors, codecs, handlers, checkpoints and fresh claims
  → ProgramDocument with selected pure validators and qualified public facts
  → preflight every selected resource table
  → owner-bound static constructors
  → complete executable Program
```

Installed codecs are real pure `Object.admit(descriptor) + Object.decode::<V>()` validators, not
just SchemaDescriptor entries. Checked constructors and nested Object digest qualification must
survive decoding. Schema-only checks cannot establish those invariants.

The same selected pure callbacks/decoded public Binding and handler parameters serve executable
and later observation restoration. Static native checks/projectors validate available current slots
and native Request/Command/Receipt/Binding correspondence without constructing an adapter. No second
validator registry, restoration engine or partially executable Program is introduced.

Complete document/revision/codec/handler/checkpoint qualification precedes attachment. All selected
table presence checks precede the first constructor; unselected tables are optional. Owner-specific
table/Binding checks may still reject inside bind after earlier constructors. Constructors are
synchronous and IO-free, and failure returns no partial executable Program.

Runtime bootstrap remains a bare retained Program Object, not code-qualified authority.
Code-qualified ProgramDocument supports pure observation; only complete Program supports start/resume.
The later inspection use case must instrument zero live construction/attachment in every public
RunView state. No generic State preparation/output replay is promised. Existing unresolved Effect
resume runs selected `S::prepare` on retained `Call.input`, canonicalizes the candidate and requires
exact Object equality with the acknowledged Command before reconcile. It never overwrites that
Command. Resource-free read and settled interpretation do not refresh or re-prepare it.

## 7. Revision and deletion ledger

For new collection families use new nominal families at version 1; for changed retained families
increment their reviewed current version. Generate exact schema hashes from the checked types,
review enclosing changes and assert retired-schema rejection. Never hand-author plausible digests.

| Cutover | Revision obligations |
| --- | --- |
| A | New Progress/Request/Holding/Observation/Failure/public output and collection native Binding/Receipt/Fault families at version 1; new collection/projection State identities at revision 1; EVM balance route version 1 → 2; the exact §3.1 config; snapshot/enrichment entry points `@1` → `@2` and all clients |
| B | Revised deployment/configuration State/capability/native implementation identities; revised native settlement with native_command_ref; RpcRejection/RpcField/ProviderReceiptResult and enclosing ProviderFailure/EvmOperationalError/transaction Fault contracts, CollectionFault and anchored lifecycle Read contracts/selected descriptors/fixtures; all affected enclosing lifecycle schemas and configured implementation selectors |
| C | Program v10 and LeafDescriptor; recovery parameter Object/handler producers; remove generic NativeAbi; evidence implementation_ref → leaf_ref plus TransactionEvidence, ContractValueEvidence and enclosing lifecycle schemas |

Unaffected native EIP-1559 command, NonceDomain, Reservation, PreparedRecord, Journal and Store
contracts retain identities where their facts/meaning did not change. Revisions are not a migration
promise. Reject superseded data without mutating acknowledged histories or reprovisioning live storage.
Shared cause-schema evolution in B is not transaction-only: inventory every enclosing selected
Read/Effect contract. Preserve the existing Read classification versus transaction wrapper
OutcomeUnknown distinction and reject superseded affected data together.

| Current location | Cutover / delete or replace | Retained responsibility |
| --- | --- | --- |
| `crates/domains/chain/src/balance/{context,observe,completion,planning}.rs` | A: generic caller/prepared/candidate/completion/source-definition handoffs | Nongeneric collection values/State boundary |
| `crates/domains/chain/src/balance/arithmetic.rs` | A: obsolete scaling/sums and arithmetic-only failures after reference inventory | Reusable arithmetic with actual other consumers |
| `crates/domains/evm/src/balance/{stages,native}.rs` | A: public chain/anchor/decimals/confirmation stages, injection and project_balance_context dispatcher | Native collection codec/qualification/projection |
| `crates/domains/evm/src/{lib,balance,balance/route}.rs` | A: stage intents/subjects/evidence/family dispatch, chain-ID-only ledger/physical target, old per-source Binding and scaling denomination wrappers | Shared address/hash/U256, full chain instance, lifecycle contracts |
| `crates/domains/portfolio/src/{lib,collection,enrichment,planning}.rs` | A: paired continuations, init/enter/resume/consolidation, valuation/report/quote/target-scale families | One checked progress, collection Read, two projections, one output |
| `crates/live/evm/src/client/portfolio*` | A: Portfolio-specific config/admission/publication/rendering moved to Application | Pure native codecs remain native-owned |
| `crates/live/evm/src/{lib,resources,json_rpc}.rs` | A: broad stage observer/registrations and per-source latest selection | One private protocol, explicit resources, anchored lifecycle/transaction primitives |
| `crates/app/src/{lib,config,run_view}.rs`, both binary READMEs/fixtures | A: old product schemas/selectors/output conversion and implicit enrichment schema guard | Exact admission, public rendering, entry-point-qualified publication/provenance |
| Chain/EVM transaction and Live custody owners | B: public Reserve/Prepare States, prepared wrappers and injection sequence | Private custody and precise command/settlement authority |
| `crates/kernel/program/src/{construction,typed_source,native,native_abi,recovery,callback}*` | C: source DSL/traversal/discovery/defaults/injection/implementation wrappers/NativeAbi/PolicyParams | Typed association, one descriptor, explicit recovery, pure callbacks |
| `EvmResources<Sources>`, Application inspection and every Program consumer/example/test | C: Sources phantom, recursive component discovery and old authoring/load API | Nongeneric owner resources and Catalog-derived inventory |

Actual references decide physical file deletion versus rewriting retained owner functions. Keep
`MfmValue`/`PersistedSchema` derives and useful macro support. Do not remove lifecycle evidence
serialization merely because Observation no longer universally requires it.

## 8. Logical implementation commits

Each production cutover updates `docs/design.md`, placement/owner docs, crate/binary READMEs,
examples, producers, consumers, fixtures and retained tests in the same coherent change. Lowercase
subjects below name responsibilities, not permission to retain incomplete parallel designs.

1. Complete the RFC's independent causal-custody/report-liveness corrections with their own tests.
2. `make collection observation the portfolio execution unit`: all of A, including shared product
   and native types, public config/output/publication, old protocol deletion and both transports.
   Splitting by crate leaves incompatible current contracts and is not a coherent commit.
3. `make transaction protocols private to one effect`: all of B, with revised exact settlement,
   private custody helper, deletion of superseded public stages/wrappers/codecs and managed authority evidence.
4. `replace source lowering with typed construction`: all of C across every consumer; remove old
   wire/DSL/NativeAbi/mandatory Observation codecs and revise retained leaf provenance together.
5. Follow the remaining independent RFC representation/storage/config/inspection cutovers.

Preparatory documentation can stand alone. A preparatory reusable primitive is a separate production
commit only when it is independently useful, has an actual consumer and leaves one coherent API.
Do not split inseparable contracts merely to reduce diff size.

## 9. Falsification and acceptance

These are required consuming scenarios, not evidence that the target already passes. Record each
prediction before running the candidate. A counterexample changes the contract or implementation;
reviewer agreement and a green unrelated task do not close it. Preserve the corrected specimen's
historical binder and unchanged-semantic-ID counterexamples in the experiment document.

### 9.1 Cutover A: actual Application and native protocol

| Conjecture / owner | Challenge and independent oracle | Required result |
| --- | --- | --- |
| Checked progress / Portfolio | Direct constructors and Object decoding: duplicate IDs/correlations, empty lists, cross-ledger point, invalid child Objects, 65 total sources distributed across collections | Typed rejection at the owning boundary; no unchecked constructor/deserializer escape |
| Transition continuity / Portfolio | Execute the real State with two differently valued collections; compare actual input prefix/suffix and source declarations, then invoke premature terminal projection | Exactly one front demand moves; prior completion and remaining suffix are exact; premature projection is Internal |
| Local identity / Live EVM | Wrong source instance, endpoint or selected Binding; also same physical route with denominations 18 versus 6 | Zero provider calls and no operational append; denomination substitution changes Binding/Request commitments |
| External identity / Live EVM | Script a successful chain-ID/genesis mismatch against independently declared expected values | Native rejection preserves actually observed facts; no balance calls; Permanent domain failure |
| One anchor / JSON-RPC | Literal HTTP request sink with independently assigned account/token quantities; advance latest between sources; simulate selected-number replacement and ABA schedule | Every state read uses the same literal canonical hash selector; ordinary growth succeeds; genuine final replacement refuses; no numbered/latest fallback |
| Native coverage / EVM | Pure projector with wrong request ref/count/native denomination, wrong field source or impossible replacement relationship | Internal native correspondence rejection; assert available original context, not a fabricated provider incident |
| Source association / real collector | Distinct literal accounts/tokens with different independently scripted amounts, inspect provider arguments and resulting ordered holdings | No permutation or wrong-target read; do not claim arbitrary valid scalar permutation is independently detectable cold |
| Truthful absence / JSON-RPC | Null latest, null selected-number confirmation, empty decimals/balanceOf response, generic RPC failure | Respectively AnchorUnavailable, AnchorUnavailable, RequiredFieldUnavailable, original operational Fault; invent no reorg or absent contract |
| Units / Application and native decoder | Literal amounts including raw 1500/denomination 3, metadata 31 and 255, ABI word 256 | Raw values retained and rendering exact; 31/255 accepted; 256 rejected with concrete range cause, no scaling path |
| Retry boundary / Application–Runtime | Two collections; refuse the second once by authenticated anchor replacement; explicit StandardRecovery allowance 1; repeat with default Stop | Fresh-anchor whole second collection retry only; first remains acknowledged and unread; Stop performs no retry |
| Causal custody / adapter–Runtime | Distinguishable nested transport/parser faults at genesis, token balance and confirmation; failed first original encoding and failed append | Known stage/chain/anchor/denomination plus complete available causes retained; acknowledge before classification; encoding/recording failure does not claim original durability |
| Cancellation / Runtime–Store | Interrupt private IO and completion append, then reload through existing exact-head Store semantics | Unacknowledged collection can repeat; committed history never rolls back; no invented per-RPC record or duplicate acknowledged transition |
| Projection and publication / Application | Required zero native plus optional zero/nonzero tokens; delete source config; publish enrichment and qualify dependent snapshot; try snapshot output with same schema | Exact selection/order/Binding; publication uses retained facts and zero provider calls; snapshot fails enrichment-entry gate; tampered run/head/output/body fails |
| Complete public cutover / CLI and REST | Run maintained cross-transport client scenario with new literal config/output; submit retired config/output selectors | Both transports share the new raw-holdings contract; old surfaces reject; product code lives in Application and Live loses Portfolio dependency |

Use deterministic provider/HTTP fixtures for impossible schedules and byte-level selectors. Use the
managed pinned Reth/PostgreSQL client scenario to establish the real native-balance path and actual
method acceptance. Successful method parsing alone does not prove canonical-hash semantics.
Exercise selected versus unknown/noncanonical hash behavior for both `eth_getBalance` and `eth_call`
and record the exact supported claim. Shipping provider acceptance requires establishing canonical
hash-selector semantics for **both** methods. If the pinned fixture cannot establish a required
method, that provider claim remains blocked; parsing and scripted calls do not substitute for it.
Empty-address `eth_call` can validate method support and empty-field refusal; it does not establish real token metadata or
`balanceOf` truth. Valid token ABI decoding and ordering can be tested against scripted literal
responses. A managed deployed-token oracle remains unvalidated unless an independently checked
token fixture is actually used; it is not a reason to add the illustrative bridge integrations.

The A cold test restores the actual current Program and acknowledged progress/output, checks exact
originals and exercises typed output qualification, publication and unfinished-collection retry.
It does not certify arbitrary
well-hashed receipt substitutions through schema-only `Runtime.read`, authenticate omitted historical
demands, or prove Fault indexes against a Request the classifier never receives.

### 9.2 Cutover B: real deployment/configuration custody

Use the maintained managed Effect regressions and the actual shipping deployment/configuration
States. Test doubles sit at provider, signer, authority and Store boundaries; do not reconstruct
another Effect state machine merely to pass the assertions.

| Conjecture | Challenge / oracle | Required result |
| --- | --- | --- |
| Semantic authority remains Runtime-owned | Gate each command/settlement append and observe real provider, signer and authority calls | No native IO, authority access or signing before command acknowledgement; pure checks remain earlier; no interpretation before settlement acknowledgement; all old recovery/barrier guarantees survive |
| First acknowledged winning wire is authoritative | Cancel after reserve/sign/retain; race retained candidates; ambiguous reservation/retention; restart with retained winner and unavailable signer | Exact committed-or-absent loading; one nonce authority and exact winning bytes; no candidate broadcast before custody acknowledgement |
| Native settlement belongs to this semantic occurrence | Swap command ref, EffectId, route, action shape or CREATE address; use actual signed-wire decoding and managed chain state | Pure owner rejects inconsistent stored facts; native_command_ref is derived from admitted recipe/Binding; independent wire/chain oracle confirms sender, nonce, target and calldata |
| Remote failure is distinguishable from local defect | Script every §5 mismatch, including submit hash disagreement after actual acceptance, and separately inconsistent retained authority | Exact Provider → EvmOperationalError → ProviderFailure → reviewed rejection plus operation/facts survive; OutcomeUnknown preserves unresolved authority; Stop retains RecoveryStopped; explicit resume uses same command/winner without resigning; local/stored mismatch is Internal |
| Settled recovery is purely retained execution | Reconstruct Runtime/Store with the same owning keystore thread, count provider/signer/authority activity and inspect lifecycle evidence | Zero native IO for settled interpretation; exact originals preserved; no process-restart claim; unresolved resume compares acknowledged Command exactly |
| Partial outcome remains honest | Complete deployment, fail/stop configuration, inspect actual chain plus lifecycle/history records | Deployment remains real and authoritative; later failure does not pretend rollback or erase deployed partial outcome |

Keep the maintained first-party contract artifact and managed task contract unless an actual changed
requirement demands a reviewed update. Stored original equality, literal independently decoded wire
facts and actual chain state are separate oracles; agreement between two serializers is insufficient.

### 9.3 Cutover C: real construction and cold loading

| Conjecture | Challenge / oracle | Required result |
| --- | --- | --- |
| Fresh ownership survives erasure | Shadow initial/terminal value codecs, foreign State/adapter/Resources and Handler/Params owners under equal reviewed references; include zero-State Programs | Reject before attachment; no independent binder or resource generic; exact initial and terminal owner checks both occur |
| Installation is atomic | A registration whose later codec/factory claim collides; inspect existing exact selections afterward | Failed contribution changes nothing; previous selections remain exact; identical same-owner value publication stays idempotent |
| Direct ports express the real boundary | Intended compile negatives for wrong Input, native Request/Command or Observation; positive plain Observation containing Cell without Serde/Sync | Compiler rejects mismatches for intended reasons; actual completion consumes the plain Observation without a hidden serialization path |
| Heterogeneous configured authoring stays ordinary Rust | Construct scalar → collection → scalar from actual config and exact/Object-selected leaves, independent native owner and two network Bindings | Real sequence executes and restores; independent owner adds no central protocol branch; no recipe-function requirement or source DSL |
| Multiple owners share one business contract | Use the same CollectHoldings State for EVM collection → independently owned Book collection → EVM collection, with distinct native schemas/resources and exact/Object selection | Framework and Portfolio acquire no protocol branch; native owners supply qualification/projection; existing scalar extension loop separately verifies changing typed endpoints |
| Qualification is complete before attachment | Valid first declaration, later malformed Binding/handler parameter/checkpoint/ref; missing installed revision; selected resource table absent | Zero constructors/IO for document rejection or missing-table preflight; no partial Program; owner-specific bind refusal remains explicit |
| Checkpoints retain fresh membership | Foreign origin, wrong target codec owner, forward, terminal or duplicate targets; unused checkpoint; legitimate selected earlier/current target | Reject illegal selected targets before erasure; unused handle adds no wire/runtime inventory; target union and Runtime barriers remain sufficient |
| One descriptor is intrinsic identity | Literal canonical descriptor and v10 wire; unknown leaf, changed adapter revision/schema, same family with different complete ABI; retired v9 | Exact selected ref only; transitive ABI qualification; no fallback, repeated ABI mirror, legacy bridge or silent old-data interpretation |
| Canonical originals remain authoritative | Nontrivial input/Binding/parameter codec normalization; exact Request/Command/original Objects and nested digest tampering | Encode admitted originals once, decode/use those same Objects; bind/check/project receive the same admitted Binding ref/value pair; typed validators reject checked-value/nested-digest faults; no reconstructed Binding hash or serialized Observation |
| Full current cold restoration works | Recreate fresh Catalog/resources, load real v10 Programs and exact Read/Effect history; test available current slots against selected static native validators | Hot/cold output and exact original/provenance equality; selected typed codec and native correspondence checks without generic State/output replay |
| Semantic revisions remain a declared trust boundary | Reproduce unchanged-ID 42 → 21 specialization and then bump reviewed identity | Unchanged IDs do not authenticate behavior; changed selected identity is rejected when missing or inconsistent; retain counterexample rather than inventing attestation |

Later public resource-free inspection separately tests every RunView state with no live tables and
an attachment counter of zero. `Catalog::qualify` being pure does not by itself prove that every
Application/Runtime read/publication caller uses the pure path.

### 9.4 Verification and evidence

Follow [docs/build-and-verification.md](docs/build-and-verification.md); `nixfied.nix` owns actual
task IDs and compositions. Start with affected domain/provider/Application tests, then public
consumer and managed scenarios. Cross-crate/persistence cutovers require one final `nix run .#ci`
on the exact candidate. Do not separately rerun every task immediately before that composed gate.

For each candidate record commit, production diff, selected commands/results, independent literal
fixtures, retained original refs, provider arguments, acknowledgement gates and deletion search.
Every trace digest must name retained inspectable evidence; a digest with removed bytes is a limited
historical claim. Failed setup/gates and unsupported claims stay visible. A final green candidate
does not rewrite historical counterexamples.

## 10. Architect conjectures and attempted refutations

The review used a dedicated integrating architect plus collection, native protocol, construction,
Effect and falsification architects. Proposals were circulated across owners, revised and then
reviewed against this actual document. The decision below records what survived criticism and why;
it is not a vote or an architectural proof.

| Conjecture challenged | Criticism | Retained decision |
| --- | --- | --- |
| Progress should preserve an immutable root plus cursor/status | More representations and validation; neither an ordinal nor a root copy proves execution history | Move remaining → completed; prove input-relative transition, state the decoder's narrower guarantee |
| Completed result should contain both demand and observation | Repeats all source identities, with more correspondence sites | One Holding per source and one temporary required-bit wrapper |
| Shared output is automatically safe for publication | Removing the distinct enrichment codec removes the current implicit program distinction | One output plus explicit qualified enrichment-entry gate in publication and provenance |
| Full Binding and route need the same reference field | Unit policy belongs to full Binding; physical route has different facts and a different hash | Request binding_ref commits full Binding; physical route only selects resources |
| Progress needs a Binding wrapper plus its full reference | Once the ref denotes the entire Object, it duplicates Object.value_ref() | Retain binding:Object directly; derive Request.binding_ref; delete the wrapper |
| Pure native ports only need decoded Binding | Normalizing codecs can make re-encoding a different identity; generic code cannot inspect arbitrary Request fields | Factory passes exact admitted binding_ref with its same-Object decoding; no universal Request trait |
| The old 0–30 denomination limit should survive | It exists for removed scaling; metadata above 30 need not require arithmetic | u8 metadata, exact raw units, string rendering; retain arithmetic only for actual remaining consumers |
| Every native stage should remain a public State or durable intermediate | Private duplicate-safe RPCs do not need their own framework transitions | One collection Read; selected exact original and known Fault context; no per-RPC durability claim |
| Empty ABI output proves absent token/contract | Scalar empty data supplies no such evidence | Required field unavailable; no guessed units, code-absence claim or silently omitted candidate |
| Anchor replacement requires old checkpoint/input invalidation | New Request has no selected anchor; old restart machinery solves a deleted representation | Retry unfinished collection at fresh anchor; default Stop remains explicit |
| Read needs original acknowledgement before interpretation | Current Read persists original and interpreted result atomically; copying Effect order adds another boundary | One Read completion append after fused projection/interpretation; retain separate Effect settlement acknowledgement |
| All appends should be infallible | Pure metadata derivation actually fails; hiding it needs deferred callbacks or more handles | Pure returns Result; Read/Effect append retained checked selections mechanically; finish owns association checks |
| Fault should copy Request ref and add a universal cross-slot hook | Current classifier lacks Request context; Runtime already retains exact occurrence/intent | Native tagged known facts plus concrete cause; honest intrinsic Fault validation scope |
| Descriptor/ABI mirrors must accompany every leaf ref | Code-qualified observation already requires the exact installed revision | One installed descriptor and references-only Program v10; no repeated generic NativeAbi |
| Schema admission proves complete cold meaning | Current Runtime does not decode/project every receipt or authenticate arbitrary history | Name current A limits, target selected typed/static checks, and separate later public no-attachment validation |
| Private Effect collapse can remove nonce/wire authority | Public staging is arbitrary; physical custody and acknowledgement are necessary | Delete public stages/codecs; retain one private protocol using existing exact authority records |
| Remote transaction mismatch is an Internal defect or settled rejection | Remote disagreement can arise after broadcast and cannot prove mutation failed | Existing Provider Fault chain with typed facts and OutcomeUnknown; same fact validator maps local corruption to Internal |

## 11. Implementation readiness and simplification gate

Cutover A has a chosen representation, exact product config, owner boundaries, failure/retry contract,
workflow, complete consumer/deletion scope and falsifiers. The next work is actual A implementation
after its prerequisite corrections, rather than another detached architecture specimen. B and C
have frozen shared seams here; their real production candidates must pass their own gates.

Before writing a production candidate, the engineer records the selected cutover and prerequisite
status, the current affected source/schema/consumer inventory, and planned family/version/State
revisions. Record generated target schema hashes once the implemented checked definitions exist;
they are not invented prerequisites to writing those definitions. Do not leave an ownership
decision to be resolved by adding a wrapper.
If an unforeseen requirement breaks a stated contract, produce the smallest counterexample and
return it to the integrating architect before adding another layer.

A candidate is ready for acceptance only when:

1. Every current producer/consumer/document uses that one design; retired API/dispatch/schema paths
   are physically absent, with retained behavior mapped to consuming tests.
2. The changed contracts and causal guarantees are documented in `docs/design.md` and placement in
   `docs/architecture.md`; public schema/README/example changes accompany the cutover.
3. Relevant §9 predictions have passed on the exact candidate, or a counterexample has explicitly
   revised the design and prediction. Unvalidated scope is stated, not relabeled as support.
4. Review measures removed and added concepts, public types, schemas, branches and future change
   sites, plus production-code LOC. Increased LOC needs a concrete necessity; fixture growth or
   unchanged production cannot establish savings. No dependency or parallel execution framework
   is added without actual need.

This document changes no production code: production-code LOC delta is **0**. It deletes no current
implementation. The deletion ledger is an acceptance obligation, not a claimed achieved saving.

## 12. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Pinned provider honors canonical hash selection for every selected method | Protocol specification and scripted argument checks do not establish live behavior | Coherence claim is unsupported; reject that provider contract without fallback | Managed method/hash tests with declared scope; independent token oracle only if claiming live token truth |
| Explicit native denomination and expected network identity are correctly reviewed operator facts | Native denomination is not an RPC fact; chain/genesis agreement alone does not authenticate all ledger history | Units/route meaning can be wrong despite exact raw values | Visible exact Binding, differing-unit/network fixtures, independent expected genesis; retain stated provider/operator trust boundary |
| Full target construction/persistence works with actual consumers | Corrected specimen is bounded and production delta was zero; Program v10 and shared validators have not executed | Missing owner/cold association may require design revision | Complete C consuming cutover and hostile/hot/cold tests; later zero-attachment public inspection |
| Private Effect cutover preserves physical authority under actual schedules | Bounded custody probes do not prove every crash/restart or key-owner scenario | Duplicate/unauthorized submission or overstated recovery | Maintained managed schedules, independent wire/chain oracle and exact ambiguity loading; preserve thread-affine owner lifetime; leave broader restart unvalidated |
| Collection-level durability is sufficient for current product behavior | It deliberately removes independently acknowledged provider stages | A real per-RPC audit requirement would conflict with the chosen unit | Review consuming product expectations and interrupted-attempt tests; document no durable physical-attempt claim |
| Physical deletion yields worthwhile production simplification | No production refactor or workload measurement has occurred | New machinery could merely relocate complexity | Complete consumer deletion inventory and concept/change-site/LOC accounting on A, B and C separately |
| Reviewed semantic revisions can be trusted without code attestation | Unchanged IDs already admitted changed hidden behavior in the experiment | A misrevisioned installed executable can violate promised semantics | Retain counterexample, require reviewed identity changes and exact missing-revision rejection; do not advertise authentication of executable bytes |

These uncertainties constrain acceptance claims. They do not authorize a second implementation,
automatic compatibility path, new provider fallback or an unbounded expansion of the example.
