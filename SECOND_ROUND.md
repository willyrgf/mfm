# Second-round EVM architecture and correctness charter

Status: open second-round refactor charter for `refac-evm`.

Baseline reviewed: `8c61dc85959e8233928f67b948048eabda5f896e` (`8c61dc85`), with a clean
worktree before this document was authored and 16 commits ahead of the configured upstream.

This document combines the product correction about independently reusable EVM balance collectors
with the correctness review performed after `PROBLEM_EVM_SCATTERED.md` was executed. It is the
acceptance charter for the second round. The first refactor remains valuable and must not be rolled
back wholesale, but the goal is not complete while the findings below remain.

Where this charter conflicts with topology, ownership, entry-point, state-count, or completion
claims in `PROBLEM_EVM_SCATTERED.md`, this charter supersedes those claims. The original remains an
audit record, not the target architecture for this round.

Line references describe the reviewed baseline and will move as the implementation changes.

## Governing rules

The optimization order remains:

1. fewer concepts;
2. fewer execution and replay paths;
3. fewer public types and schemas;
4. fewer duplicated responsibilities;
5. fewer places a future EVM change must touch;
6. less code, once correctness, security, and replay guarantees are satisfied.

The second round also retains these non-negotiable rules:

- There is no backward-compatibility requirement. Delete superseded types, schemas, packages,
  aliases, constructors, re-exports, and code paths in the commit that replaces them.
- Do not add fallback RPC routes, redirect following, `collect_or_reuse`, legacy schema readers,
  deprecated facades, or parallel old/new implementations.
- Keep commits coherent and buildable. One commit owns one logical correction.
- Preserve reusable EVM mutation, keystore-backed signing, `Create`, `Call`, and exact-anchor
  contract validation. Their current lack of a public workflow does not make them disposable.
- Do not restore the deleted fixed contract lifecycle or the deleted nine-state collector graph.
- Network and filesystem IO remain explicit runtime capabilities. State reduction and replay remain
  deterministic and evidence-only.
- Secrets, endpoints, authorization material, passwords, private keys, signed raw transactions, and
  secret-bearing diagnostics must never enter persisted or public surfaces.

## Verdict

The first round is substantial and directionally strong, but “complete with no findings” is not
supported. The goal must remain open for three independent reasons:

1. EVM balance collection is a reusable source operation, while portfolio snapshot is only one
   composition that consumes it. Portfolio-owned EVM execution states and direct EVM
   observation/value paths into report computation are therefore no longer honest boundaries.
2. Three major transaction correctness/composability defects and five moderate validation/runtime/
   transport defects reproduce on the reviewed baseline.
3. The available CI report is not tied to a Git SHA, and the reported architect reviews are not
   independently retained. Neither can serve as formal completion evidence.

The correction is not to recover the old implementation. The desired result is the current compact
EVM substrate plus one reusable two-state collector operation and a store-backed portfolio report
operation, with the correctness defects fixed at their owning boundaries.

## What the first round got right

The following outcomes are preserved:

- Exactly five EVM-named packages currently remain:
  `mfm-evm-capabilities`, `mfm-evm-signing`, `mfm-states-evm`, `mfm-adapters-evm`, and
  `mfm-transports-evm`.
- The old EVM core, contract model/state/adapter island, collector operation package, compatibility
  aliases, and custom transaction encoder are gone.
- Transaction construction and signing have one Alloy-backed EIP-1559 path.
- Write authority and keystore signing remain available for portfolio rebalancing and future wallet
  operations.
- Direct contract creation, direct calls, and exact-anchor validation remain as composable
  substrate without restoring a fixed deploy/configure lifecycle.
- `mfm-states-evm` currently contains exactly two reusable EVM states: transaction submission and
  contract validation. The two EVM-related portfolio collection states are separate and are the
  ownership issue corrected below.
- EVM balance collection itself was successfully collapsed from nine states and three operations to
  one external-read state and one atomic managed-write state.
- One checked session resolves an anchor once, performs exact-hash reads, deduplicates ERC-20
  metadata, bounds concurrency, and finishes with a number-to-hash canonicality check.
- The supplied CI report says all 14 stages passed in 409.5 seconds, and focused EVM/signing/
  keystore/state/adapter/transport/portfolio tests reportedly passed independently.

The last point is useful historical context, not merge-readiness evidence. The report does not name
the tested SHA, so it cannot prove the state of `8c61dc85` or any second-round commit.

## Architecture correction: collectors are reusable operations; portfolio is composition

`PROBLEM_EVM_SCATTERED.md:1020-1026` explicitly said that another balance-collection consumer would
justify a reusable collector state family. The portfolio snapshot must compose collectors as
independent operations; the report consumes only their receipt authority and store-backed facts.
Future objectives must be able to compose the same collector operations without depending on
portfolio types. The original portfolio-owned premise at
`PROBLEM_EVM_SCATTERED.md:212-215` is therefore false.

Independence here means ownership, composability, and internal schedulability, not another
application entry point. The EVM collector is an exported, reusable internal library operation that
other operation crates and an internal scheduler root can call. It is not a separately discoverable
MFM objective, setup target, CLI/REST command, or app-rendered output contract.

The correction affects ownership, operation topology, report selection, and replay dispatch. It
adds the two balance states to the existing EVM state package, but does not justify a collector-only
capability, transport, model, adapter, replay, or application package.

### One public objective composed from three reusable stages

Keep exactly one public entry point:

```text
mfm.portfolio/snapshot@1
  |
  `-- PortfolioSnapshotOperation
        |
        +-- BtcNetworkCollectionOperation(s)
        |     read blockchain -> compute observations -> write facts -> receipt(s)
        |
        +-- EvmBalanceCollectionOperation(s)
        |     read blockchain -> compute observations -> write facts -> receipt(s)
        |
        `-- PortfolioReportOperation
              read receipt-pinned facts from store
              -> compute portfolio snapshot
              -> project portfolio report
```

The family collectors may run concurrently. `PortfolioReportOperation` starts only after every
required collector receipt exists. `PortfolioSnapshotOperation` constructs no protocol state node
and consumes no protocol observation directly; it only calls reusable operations and connects their
typed receipts to the report operation.

The only portfolio-specific planning it retains is the deterministic projection of one normalized
`PortfolioConfig` into generic BTC/EVM source configs and report requirements. It owns no JSON-RPC,
ABI, EVM address parsing/normalization/codec/validation, token metadata, fact-publication,
fact-hydration, retry, replay, or family-specific state logic. The child-operation calls and typed
dependency edges are the complete composition plumbing.

Do not add `mfm.evm/balance-snapshot@1`, an `evm_balance_collection` public setup kind, a public EVM
target resolver, or an EVM app output renderer. One internal scheduler-owned cycle root wraps
`EvmBalanceCollectionOperation` and binds only its receipt so collectors can run without a portfolio
report. That wrapper is not discoverable through app ingress and may not reconstruct the states. If
another public objective later needs balances, it composes the same operation.

### One bounded EVM collection per operation call

The generic EVM collector config is one canonical type:

```text
EvmBalanceCollectionConfig
  network_id
  non-zero chain_id
  native_decimals
  strictly sorted, unique, bounded sources

EvmBalanceSource
  account
  asset = Native | Erc20(non-zero contract)
```

One operation call represents one network and one exact set of `(account, asset)` pairs. It uses one
route, one checked session, one anchor, one atomic fact batch, and one failure domain. Multiple
networks or operational groups are separate operation calls with caller-owned scope and operation
keys. Those graph identities and the enclosing run identify an invocation; do not add a semantic
`collection_id` merely to replace the public target that is no longer being created.

The config contains no portfolio ids, wallet ids, symbols, quote currencies, endpoint URLs,
credentials, retry policy, schedules, or process-local concurrency tuning. Portfolio planning maps
its wallet/symbol demand to this EVM source algebra before calling the generic operation.

### Minimal EVM state, fact, and receipt surface

Keep exactly two balance states:

1. `CollectEvmBalancesState` owns the sorted plan, retained evidence, exact-anchor reducer, and
   `EvmBalanceObservationBatch`.
2. `RecordEvmBalanceFactsState` validates the complete batch, records every fact atomically, and
   returns one checked `EvmBalanceCollectionReceipt` in the same managed-write result.

The receipt contains only what a later store reader needs to prove exact coverage: network and
chain identity, canonical anchor, sorted source identities, and the verified content identities
published by that operation. Its dependency proves that this run completed the atomic publication;
the content identities intentionally do not distinguish two byte-identical append occurrences. It
does not copy full balance response material into a parallel snapshot. The observation batch is an
internal edge between the two states, not a public operation result.

Use one EVM-domain fact:

```text
kind: evm.balance_snapshot

subject:
  network_id
  chain_id
  account
  asset = Native | Erc20(contract)

response:
  block_anchor
  raw_units
  decimals
```

The fact contains no portfolio id, wallet id, symbol, route, endpoint, operation key, run id, or
source ref. Source ref remains retained read-evidence provenance. Two collector calls that overlap
on an account/asset observe the same logical fact subject and may legitimately append different
anchored observations.

Do not restore:

- native and ERC-20 sub-operations;
- per-account or per-token states;
- token-metadata states;
- joint-tip states;
- collector checkpoints for complete balance batches;
- nested batch/network receipt pyramids;
- method-specific RPC capabilities or evidence mirrors;
- a direct EVM snapshot output bypassing storage;
- a collector-only client, adapter package, transport package, replay stack, or address vocabulary;
- a public application surface for the internal collector operation.

### Store-backed portfolio report operation

`PortfolioReportOperation` is a reusable operation in the existing portfolio operation package; it
does not need another crate. Its input is the normalized portfolio/report config plus typed vectors
of `BtcNetworkCollectionReceipt` and `EvmBalanceCollectionReceipt` handles. It owns the report
topology:

```text
SelectHoldingsState
  read/hydrate exact receipt-authorized BTC + EVM facts from store
  validate demand, coverage, identity, and anchors
  |
  `-- AssembleSnapshotState
        compute one canonical PortfolioSnapshot
        |
        `-- ProjectReportState
              project one structured PortfolioReport
```

The two typed receipt vectors are already the fan-in and scheduler barrier: the read state cannot
run until every producing managed write completes. Do not add a `PortfolioCollectionReceipt`, a
receipt-assembly state, or a generic family-receipt wrapper that duplicates their source, anchor,
and content-identity material. These typed receipt edges are the only family-aware composition
plumbing; the root never interprets their contents. Human/text rendering remains in the binary;
the operation returns both the computed snapshot handle and structured report handle.

The storage boundary must preserve that edge: one managed-write append commits its fact records,
fact-index visibility, and receipt state output atomically, and the subsequent multi-family read
uses one consistent read frontier. This is the existing append/read contract to exercise, not a new
portfolio transaction coordinator.

The report must read both families from the store. Delete the current asymmetry where Bitcoin uses
receipt-pinned fact selection but EVM facts are ignored in favor of a direct
`EvmNetworkSnapshot` handle. The EVM receipt is dependency and selection authority, not balance
material smuggled around the store.

This is a same-run, receipt-pinned read, not “read whichever fact is latest.” The report may read
only the exact content identities named by completed child receipts and must hydrate and rederive
those identities before computing. It performs no unbounded fact-index scan and cannot substitute
different content from older or unrelated runs. A byte-identical claim with the same descriptor,
subject, anchor, and response is semantically interchangeable and need not acquire a new public
claim-occurrence identity. Reusing different earlier collection requires a separate, explicit
receipt/as-of authority and is outside this round; it is not a fallback mode.

Each collector's fact batch is atomic on its own. The report runs only after all required receipts
commit. If one collector fails, no report is produced; already committed append-only facts from
other collectors remain valid observations, and resume continues the incomplete graph without
rerunning completed writes. Do not introduce a cross-family database transaction or compensate by
deleting facts.

“Fresh” means one canonical anchor for each collector invocation/network, not an imaginary
simultaneous cross-chain tip. The snapshot preserves every network pin. Any maximum-age or as-of
policy must be explicit caller/report config; it is never inferred by selecting the current latest
fact. Overlapping scheduled collectors cannot contaminate the report because only receipt-authorized
content is eligible.

### App ingress and scheduling

The application surface remains unchanged in shape:

- the closed setup document stores portfolio configuration only;
- entry-point discovery returns exactly `mfm.portfolio/snapshot@1`;
- admission resolves that portfolio target once and certifies the complete composed graph; and
- generic CLI/REST start, resume, status, stream, output, and replay surfaces remain the only public
  run controls.

The EVM collector operation has no public app resolver, target schema, discovery id, or standalone
output renderer. Multiple EVM collector instances can execute independently in distinct child
scopes or internal scheduler-owned roots, for different networks, wallets, and token sets. The
scheduler supplies run/invocation identity and cadence; neither belongs in semantic collection
config or an embedded collector loop.

App assembly still registers the internal EVM state runners, fact descriptor, operation descriptor,
and replay verifier needed by a certified portfolio graph. Internal executability is not public
ingress; none of those registrations may appear in entry-point discovery.

Many simultaneous collectors require a shared HTTP connection pool and a process-local concurrency
limit per `source_ref`, in addition to the existing per-run read bound. These are runtime/transport
resources, not semantic config. One capability call still corresponds to one explicitly owned HTTP
exchange; connection-pool sharing does not authorize redirects, retries, or failover. Do not
introduce a cross-run balance or token-decimals cache without an independently specified anchor and
invalidation contract.

### Package organization

The target EVM package surface is deliberately six packages:

| Package | Sole durable responsibility |
|---|---|
| `mfm-evm-capabilities` | Checked read/transaction authority, session evidence, and canonical EVM protocol evidence types |
| `mfm-evm-signing` | Canonical transient EIP-1559 envelope signing/finalization |
| `mfm-states-evm` | Balance collection/publication, transaction, and exact-anchor validation semantics |
| `mfm-adapters-evm` | Runtime and replay bindings for those reusable states |
| `mfm-transports-evm` | Source-bound JSON-RPC implementation |
| `mfm-op-evm-collectors` | Reusable EVM collector topology plus its internal scheduler cycle wrapper; no app entry point |

The new package lives at `crates/ops/evm-collectors-op`, not at the repository root. There remain
only two top-level `crates/evm-*` directories: capabilities and signing.

Do not merge the five lower layers merely to reduce Cargo package count. Their dependency firebreaks
prevent states from depending on HTTP, transports from depending on workflow topology, generic
signing from depending on runtime state, and app-only bindings from becoming protocol APIs.

Do not merge `mfm-op-btc-collectors` and `mfm-op-evm-collectors` into a protocol-agnostic collector
bucket merely to avoid one manifest. They share the generic `Operation` composition mechanism, not
state semantics, facts, or protocol plans. A combined package would make single-family consumers
depend on both and create a vague dumping ground. Filesystem regrouping is cosmetic and, if desired,
must be a separate repository-wide category cleanup rather than an EVM-only exception.

## Review finding 1: provider outages can terminalize submitted transactions

### Confirmed behavior

`crates/adapters/evm/src/transaction.rs:815-910` performs receipt, canonical block, and head reads
after durable submission is known. Every `EvmCapabilityError` is converted by
`capability_error` at `transaction.rs:1232-1234` into `RuntimeError::InvalidRunnerOutput`.

`crates/kernel/runtime/src/attempt.rs:111-117` marks invalid runner output and runtime validation as
non-retryable. The observed attempt is terminalized. A transaction may therefore succeed on-chain
while a temporary transport/provider outage permanently fails its MFM run.

The existing `RuntimeError::Blocked` path is already used when a receipt is absent, moves, is not
canonical, or lacks confirmation depth. Provider unavailability must preserve the same durable
side-effect ledger rather than becoming validation failure.

### Required correction

Create one closed, phase-aware EVM capability-to-runtime error classifier shared by collection,
validation, and transaction adapters:

- `EvmCapabilityError::InvalidRequest` represents deterministic request/contract failure and maps to
  terminal runner validation.
- After submission is durably known, every provider, route, transport, HTTP/RPC, response, and
  source-binding failure maps to operational block. None can prove that the transaction failed or
  authorize replacement mutation.
- Before mutation and for read-only states, closed availability/resource diagnostic classes map to
  operational block, while deterministic request/response contract violations remain terminal.
  Classification uses typed variants/codes, never provider strings.
- State reducer errors, retained-evidence mismatches, signed-hash mismatches, and certified-authority
  violations remain terminal and fail closed.

In particular, after submission is known, an availability error must never return a failure that can
cause the submit node to execute again. Resume continues from the durable side-effect phase and only
re-observes receipt/confirmation. There is no rebroadcast fallback.

Use the existing blocked/operational outcome unless the runtime genuinely cannot retain the required
redacted diagnostics with it. Do not add several retryable error enums or protocol-specific runtime
states. Do not globally mark every capability failure retryable: post-submission uncertainty and
pre-mutation deterministic rejection have different authority.

### Required tests

- Receipt lookup provider failure leaves the run operationally blocked, not terminal.
- Canonical block and head provider failures after receipt retention do the same.
- Resuming with a healthy session completes without another raw-transaction submission.
- A malformed retained receipt or signed-hash mismatch remains terminal.
- Missing runtime route and source mismatch can be repaired without changing certified semantic
  config.
- The independent balance collector also blocks, rather than terminalizes, on temporary provider
  unavailability.

## Review finding 2: gas estimation differs from the signed transaction

### Confirmed behavior

`EvmTransactionEstimate` at `crates/evm-capabilities/src/lib.rs:362-414` contains sender,
destination, value, input, and access list, but not nonce or EIP-1559 fee fields.
`crates/transports/evm/src/lib.rs:204-219` consequently sends neither nonce nor
`maxFeePerGas`/`maxPriorityFeePerGas` to `eth_estimateGas`.

The final signed transaction contains all of them. The simulation can therefore describe different
execution, notably for direct CREATE address context and contracts that inspect effective gas price.

### Required correction

There must be one pre-gas-limit transaction description:

1. Resolve the exact pending nonce and checked EIP-1559 fee inputs.
2. Construct `EvmTransactionEstimate` from the intent plus those observations. It contains chain id,
   type-2 identity, nonce, sender, destination/create kind, value, input, access list,
   `maxFeePerGas`, and `maxPriorityFeePerGas`.
3. Validate Alloy's nonce/fee width constraints before estimation IO, then send that exact
   description to `eth_estimateGas` against the `pending` block context.
4. Add the checked gas-limit policy to the same description to produce `EvmUnsignedTransaction`.
5. Sign that unsigned transaction through the sole Alloy path.

Delete the old intent-only estimate constructor and any duplicate reconstruction of transaction
fields. Estimation and signing must derive from one admitted representation so future transaction
fields cannot drift between them.

### Required tests

- Transport JSON for CALL and CREATE contains type `0x2`, chain id, exact nonce, both fee fields,
  and the `pending` block parameter.
- Estimate and unsigned transaction expose identical common fields.
- Boundary U256-to-RPC quantities are canonical and never truncated.
- A changed nonce or fee observation changes the estimate and signing digest as expected.
- A local EVM parity test estimates and submits the same type-2 transaction description.

## Review finding 3: receipt logs are not composable outputs

### Confirmed behavior

`EvmTransactionReceipt::logs()` exposes `&[EvmTransactionLog]`, but
`EvmTransactionLog` at `crates/states/evm/src/transaction.rs:958-976` has private fields and no
read API. An operation cannot inspect factory, smart-wallet, token, flash-loan, or other protocol
events without serializing and reparsing an internal persisted type.

### Required correction

Keep fields private if that protects construction invariants, but provide a complete lossless typed
read surface for:

- emitting address;
- ordered topics;
- data bytes;
- block anchor;
- transaction hash and transaction index;
- log index; and
- removed status.

Read-only accessors over the already canonical persisted strings and block anchor are sufficient for
the first consumers; operations can decode ABI data with Alloy. Operations must be able to
deterministically interpret logs without JSON round trips or knowledge of private field layout. Do
not add a second public “decoded log” DTO containing the same fields or speculative event wrappers.

Because `EvmTransactionReceipt` is described as complete receipt evidence, add read-only accessors
for its currently hidden transaction index, sender, destination, gas quantities, cumulative gas, and
session provenance in the same API pass. Keep construction and mutation private.

The same commit owns the reverted-log invariant from finding 8 below because it defines which logs
are safe for downstream composition.

### Required tests

- A consuming operation can inspect address, topics, data, anchor, and indexes through public APIs.
- Accessors preserve full U256 indexes and arbitrary bounded log bytes.
- Accessors do not permit mutation or construction of incoherent logs.
- Rustdoc demonstrates deterministic event identification without serialization.

## Review findings 4 and 5: validation budgets are inconsistent and late

### Confirmed behavior

The validation state admits up to 64 calls, 128 KiB of runtime code, and 128 KiB for each expected
return, while the reducer caps code plus all returns at 4 MiB:

- constants: `crates/states/evm/src/contract_validation.rs:28-33`;
- per-return validation: `contract_validation.rs:247-250`;
- config validation without an aggregate sum: `contract_validation.rs:317-334`;
- reducer-only aggregate check: `contract_validation.rs:641-677`.

Thirty-two maximum returns consume the whole 4 MiB and leave no room for mandatory non-empty code.
The theoretical admitted maximum is 8.125 MiB.

Live execution worsens the problem. `crates/adapters/evm/src/lib.rs:167-180` accumulates every raw
decoded response. `EvmContractValidationEvidence::from_observations` then creates hex evidence for
the code and every response before the reducer applies the aggregate bound. The transport has a
one-MiB per-RPC body cap, but 64 retained responses plus their hex copies can still allocate tens of
MiB before rejection.

### Required correction

Keep the 4 MiB aggregate evidence limit and define one feasible budget:

```text
maximum code bytes       = 128 KiB
maximum total evidence   = 4 MiB
maximum total returns    = maximum total evidence - maximum code bytes
maximum calls            = 64
maximum one return       = 128 KiB
```

Config and plan validation checked-sum the canonical decoded lengths of all expected returns and
reject anything above the total return budget. Reserving the full code allowance guarantees that
every admitted policy can succeed for any admitted code size. Do not raise the aggregate limit or
defer an authoring error to live execution.

Replace the raw-response batch with one state-owned, non-serializable bounded evidence builder or an
equivalent single consuming API:

1. Admit code immediately, checking non-empty, code hash, per-code bound, and aggregate budget.
2. For each plan call in order, read one response, enforce its explicit capability response bound,
   compare it with the expected return, checked-add its decoded size, canonicalize its one evidence
   observation, and release the raw bytes before the next call.
3. Stop before later calls or the canonicality read as soon as a bound is crossed.
4. Finish with the number-to-hash canonicality observation and the one serializable evidence value.

The reducer independently rechecks order, equality, hashes, and the same aggregate budget against
retained evidence. Live construction and replay must share budget helpers; they are not separate
semantic implementations.

The owning commit must also prove that the worst-case canonical config, plan, and evidence artifacts
fit the 16 MiB PostgreSQL artifact admission bound. Raw-byte accounting alone is insufficient because
hex encoding and duplicated call contexts expand the persisted representation. Prefer retaining an
ordered plan index/request digest with each response instead of copying the complete call context
into evidence when the certified plan already owns that context. If call calldata or access-list
material can still exceed the artifact bound, add aggregate input bounds at authoring rather than
waiting for store rejection.

`EvmCall` should carry a required maximum return size so the transport can reject oversized method
results at the request boundary. Validation can use the exact expected-return length as its maximum;
balance and metadata calls can use their protocol-fixed sizes. Keep the transport-wide HTTP-body cap
as an outer defense, not as the semantic validation budget.

### Required tests

- Exact aggregate return budget is admitted; budget plus one byte is rejected at config admission.
- Maximum code plus the exact return budget reduces successfully.
- Forged/deserialized over-budget evidence still fails replay reduction.
- Oversized code prevents the first contract call.
- An oversized or aggregate-crossing return prevents every subsequent RPC and canonicality read.
- Exact-bound builder and reducer results agree.
- A hostile near-transport-limit response cannot be accumulated repeatedly.

## Review finding 6: external-read ingress performs synchronous filesystem IO

### Confirmed behavior

The generic external-read runner returns an async ingress future but invokes a synchronous executor
hook at `crates/kernel/runtime/src/runner_kit/external_read.rs:102-111`.

EVM validation and portfolio EVM collection use that hook to call
`LiveTransportRuntime::validate_evm_network_binding`, which reaches
`RuntimeConfig::load_evm_route` and `fs::read_to_string` through
`crates/runtime-config/src/lib.rs:267-275`. This blocks an async runtime worker. The transaction
ingress path already uses `tokio::task::spawn_blocking` correctly.

### Required correction

- Make `ExternalReadPlanExecutor::validate_ingress` asynchronous and await it in
  `ExternalReadRunner`.
- Change the EVM read-validation closure to one async contract shared by contract validation and
  balance collection.
- Add one app-owned async route-validation method that performs the selective runtime-config file
  load and parsing inside `spawn_blocking`.
- Remove the synchronous EVM validator type and both portfolio/EVM copies of its path.

Do not remove admission validation, defer all validation until execution, call `block_in_place`, or
keep sync and async variants. Runtime routing stays process-local and is never certified or replayed.

### Required tests

- The generic runner actually awaits a pending async ingress validator.
- Failed validation performs no bind or RPC request.
- A current-thread Tokio test or equivalent boundary guard proves the filesystem read runs on a
  blocking worker.
- Contract validation and balance collection use the same async route validator.

## Review finding 7: automatic HTTP exchanges violate session authority

### Confirmed behavior

`crates/transports/evm/src/lib.rs:75-80` builds a reqwest client with the default redirect policy.
Reqwest follows a bounded number of redirects by default, while `docs/evm-rpc-routing.md:53-57`
requires one session to remain fixed to one endpoint and explicitly forbids reselection/failover.

Reqwest 0.12.28 also defaults to retrying supported protocol NACKs. The current feature graph does
not enable the HTTP versions that activate those retries, so this is latent rather than an observed
duplicate today. Leaving the default allows later feature unification to change transaction
submission semantics without an MFM code change.

### Required correction

Set `reqwest::redirect::Policy::none()` and `reqwest::retry::never()` on the one EVM HTTP client.
Treat every 3xx response through the existing non-success status path. Do not manually follow,
rewrite, whitelist, or record redirect targets, and never forward authorization material to them.

All transaction repetition remains in the adapter's durable lookup-before-rebroadcast recovery
path. One `submit_raw_transaction` capability call means exactly one HTTP exchange. A later
byte-identical broadcast requires a separate explicit recovery invocation and never chooses a new
nonce, fee set, or envelope.

The transport implementation should share its bounded HTTP connection pool across the many EVM
sessions required by independent collectors, while each session retains its exact endpoint and
source evidence. Pool sharing must not create a public unchecked client escape hatch.

### Required tests

- An origin server returning 307 causes bind/request failure.
- The redirect target receives zero requests.
- Authorization is never sent to the redirect target.
- One capability submission performs exactly one origin request even when protocol features change.
- Recovery performs its exact-hash lookup before at most one explicit byte-identical rebroadcast.
- Direct requests still bind once, probe chain id once, and retain the original source ref.

## Review finding 8: reverted receipts accept logs

### Confirmed behavior

`EvmReceipt::validate` at `crates/evm-capabilities/src/lib.rs:509-524` validates log identity and
removal status but not the relation between receipt status and logs.
`EvmTransactionReceipt::validate_against` at
`crates/states/evm/src/transaction.rs:1123-1158` likewise accepts a reverted receipt containing
otherwise coherent logs. Top-level EVM revert rolls logs back, so this is inconsistent evidence.

### Required correction

- `EvmReceipt::validate` rejects `Reverted` with any log as `IncoherentReceipt`.
- The persisted state reducer independently retains the same invariant for forged replay evidence.
- Do not strip or silently normalize logs from a reverted receipt.

### Required tests

- Reverted receipt with a coherent-looking log is rejected at capability conversion and replay.
- Reverted receipt with no logs remains a valid terminal reverted outcome.
- Successful receipt with bounded coherent logs remains valid and composable.

## Documentation correction: `source_ref` is audit provenance

`docs/design.md:749-760` says reducers and replay reject source substitution even when network and
chain match. That contradicts both implementation and the authoritative persisted-surface policy.

The implementation intentionally validates semantic network id, chain id, and certified transport
implementation. It does not compare `source_ref` with certified config or current runtime config.
`docs/persisted-public-surfaces.md:46` correctly classifies source refs as audit provenance only.

The design contract must say:

- one live attempt uses one fixed source-bound session;
- source ref is checked public provenance explaining which process-local route served that attempt;
- it may differ across attempts or resume after runtime routing changes;
- replay never resolves it against current runtime config and does not treat it as semantic policy;
- post-commit alteration is rejected by normal artifact digest/stream integrity, not by comparing it
  to a current route; and
- a provider identity that must affect semantic trust has to be authored explicitly as certified
  oracle/source identity rather than smuggled through runtime routing.

Fix `docs/design.md` and misleading tests/comments. Do not add source ref to balance facts, semantic
configs, idempotency keys, or reducer equality merely to make the incorrect sentence true.

## Shared EVM primitives to simplify during the move

The collection move exposes two remaining boundary leaks that should be corrected rather than
carried into the new operation:

1. `mfm-states-evm` depends on `mfm-portfolio-model` for `EvmBlockAnchor`. Move the canonical block
   identity into `mfm-evm-capabilities` and remove the portfolio dependency.
2. Capability `EvmBlock` and persisted `EvmBlockAnchor` represent the same number/hash identity.
   Collapse them into one checked capability-owned canonical type if one representation can satisfy
   both live protocol and persisted JCS requirements. Do not retain two public number/hash structs
   merely because they originated in different old crates.

Collection and validation also require the same checked EVM read-session assembly. Replace
`EvmValidationRunnerCapabilities` and portfolio-specific EVM binder fields with one
`EvmReadRunnerCapabilities` used by both state families. Transaction authority remains separate
because it also binds signer material and mutation capability.

## Move and deletion ledger

The reusable-collector commit series must move behavior once and delete the old ownership in the
same owning commits.

Move and rename:

- portfolio `EvmBalanceSource` to the generic EVM source/asset algebra;
- `EvmNetworkCollectionConfig` to generic `EvmBalanceCollectionConfig`, without a portfolio or
  public-target identity;
- collection plan, evidence, reducer, bounded source policy, observation batch, collection receipt,
  fact subject/response, fact descriptor, and visibility into `mfm-states-evm`;
- collection execution, ERC-20 encoding/decoding, ordered bounded concurrency, atomic fact
  publication, and replay verification into `mfm-adapters-evm`;
- canonical EVM block identity out of `mfm-portfolio-model`;
- private EVM graph expansion out of `mfm-op-portfolio-snapshot` into
  `mfm-op-evm-collectors`; and
- BTC-only portfolio fact selection to one receipt-pinned BTC/EVM store-read path owned by
  `PortfolioReportOperation`.

Delete:

- `crates/states/portfolio/src/evm_collection.rs` after its reusable behavior is moved;
- every EVM capability/session field, executor, selector constant, ERC-20 codec, publication runner,
  and collection replay path from `mfm-adapters-portfolio`;
- portfolio-owned EVM fact/state/schema names and exact-state re-exports;
- direct `CollectEvmNetworkState -> PublishEvmHoldingsState` construction inside portfolio planning;
- `EvmNetworkSnapshot`, the `evm_snapshots` assembly input, and every direct EVM observation bridge
  that bypasses the fact store;
- the Bitcoin-only `PortfolioCollectionReceipt`, `CollectedHoldingReceipt`,
  `AssemblePortfolioCollectionReceiptState`, its runner/replay path, and other fan-in wrappers once
  `SelectHoldingsState` consumes typed family receipt vectors directly;
- the now-empty app-only `crates/app/src/portfolio_snapshot.rs` and
  `crates/app/src/portfolio_snapshot_replay.rs` runner/verifier island;
- the `all_evm_portfolios_do_not_call_the_fact_index` behavior and test; the target behavior is an
  exact receipt-pinned EVM store read;
- the `mfm-states-evm -> mfm-portfolio-model` Cargo edge;
- portfolio-only replay dispatch for EVM collection;
- “portfolio owns EVM balance reads,” “EVM bypasses fact-backed selection,” “exactly five EVM
  packages,” and “exactly two EVM states” assertions that cease to be true;
- every proposed EVM public entry point, public setup target/resolver, app-rendered output, and
  `collection_id`; the sole-public-objective assertion remains correct. An internal scheduler cycle
  root is retained only as a wrapper around the same operation;
- the old intent-only gas-estimate constructor;
- the blanket `EvmCapabilityError -> InvalidRunnerOutput` conversion;
- the synchronous external-read ingress validator; and
- redirect-enabled EVM client construction.

Do not leave compatibility exports, old schema aliases, deprecated state names, forwarding modules,
or a second replay verifier for the deleted ownership.

## Buildable commit sequence

Every subject below is intentionally lower case. Normative documentation and tests change in the
commit that changes their owning behavior; the final documentation commit is an audit, not a place
to defer contract updates.

### 1. `make evm rpc sessions source-stable and bounded`

- Disable redirects in the EVM HTTP client.
- Disable reqwest's implicit retry policy so adapter recovery owns every repeated mutation exchange.
- Share the bounded HTTP pool and add the process-local per-source concurrency guard without
  exposing an unchecked client.
- Correct `source_ref` semantics in `docs/design.md`, routing docs, comments, and tests.
- Add origin/redirect-target, single-exchange, shared-bound, and audit-provenance tests.

### 2. `preserve evm operational failures for resume`

- Add one shared phase-aware EVM capability-to-runtime error classification.
- Map all post-submission provider failures and pre-mutation/read availability failures to
  operational block.
- Keep deterministic request, state, evidence, and authority violations terminal.
- Cover receipt, canonicality, head, validation, and balance-read outages.
- Prove resume does not rebroadcast a submitted transaction.

### 3. `make evm gas estimation match signed transactions`

- Add nonce and type-2 fee/chain fields to the one estimation description.
- Build estimate and unsigned transaction from one common admitted representation.
- Send the complete transaction object to `eth_estimateGas`.
- Delete the old intent-only constructor and duplicated field assembly.
- Add CALL, CREATE, boundary, digest, and live parity tests.

### 4. `make evm receipt evidence composable and coherent`

- Add complete typed read access to `EvmTransactionLog` without a duplicate DTO.
- Reject reverted receipts containing logs at capability and replay boundaries.
- Update downstream-operation rustdoc and receipt fixtures.
- Add public-access and incoherent-revert tests.

### 5. `bound evm contract validation before retention`

- Enforce the aggregate expected-return budget during config and plan admission.
- Add the explicit per-call response bound to capability requests.
- Replace raw batch accumulation with one bounded consuming evidence-construction path.
- Release each raw response before the next request.
- Remove redundant full call-context copies from evidence where a plan-bound index/digest is enough,
  and prove worst-case canonical artifacts fit storage admission.
- Retain reducer/replay checks as defense against forged evidence.
- Add exact-bound, plus-one, early-stop, allocation, and replay tests.

### 6. `move evm block identity out of portfolio`

- Establish the one canonical capability-owned block identity.
- Switch transaction, validation, transport, portfolio collection, and tests atomically.
- Delete the portfolio block type and the EVM-state dependency on portfolio model.
- Do not leave an alias or duplicate serialized representation.

### 7. `make portfolio reports consume stored evm facts`

- Replace the direct EVM network snapshot output with one checked collection receipt returned by the
  atomic fact-recording state.
- Rename the portfolio-owned fact kind/schemas to their final generic `evm.balance_snapshot`
  contracts here so the later ownership move is location-only, not a second semantic migration.
- Extend receipt-pinned portfolio selection, hydration, and descriptor authority to both BTC and
  EVM facts.
- Pass the typed BTC/EVM receipt vectors directly into `SelectHoldingsState`; their input edges are
  the managed-write completion barrier.
- Assemble the portfolio snapshot only from material re-read and reverified from the store.
- Delete `EvmNetworkSnapshot`, direct EVM assembly inputs, the all-EVM no-fact-index branch, and
  their replay paths in this commit.
- Delete `PortfolioCollectionReceipt`, its assembler state, app-only runner/replay verifier, and
  generic receipt-entry wrappers in this commit; do not replace them with another fan-in value.
- Update persisted-surface, portfolio-snapshot, state, adapter, and operation documentation with the
  receipt-pinned EVM store-read contract in this commit.
- Add same-run store-read, exact-receipt, different-content stale-fact, byte-identical-equivalence,
  missing-fact, tamper, atomic read-after-write, consistent-frontier, all-EVM, and mixed-family
  tests.

This commit deliberately fixes the dataflow before moving ownership: there is one store-backed
report path and no simultaneous direct/store EVM modes.

### 8. `move evm balance collection into a reusable operation`

- Add `mfm-op-evm-collectors` under `crates/ops/evm-collectors-op`.
- Move the generic EVM source/config/evidence/batch/receipt/fact contracts and the two states into
  `mfm-states-evm`.
- Move live execution, atomic publication, and replay into `mfm-adapters-evm`.
- Define `EvmBalanceCollectionOperation` as exactly the read state followed by the atomic record
  state, exporting only its typed receipt handle.
- Add one internal scheduler cycle draft/launch helper that calls this operation and binds its
  receipt; it owns one-cycle config/root binding but no second topology or app entry point. The
  scheduler supplies cadence and invocation identity.
- Reuse the one EVM read-runner capabilities assembly.
- Replace portfolio's direct state construction with child operation calls.
- Delete the portfolio collection module, adapter execution, obsolete exports, and replay paths in
  this commit without changing the final fact schema established in commit 7.
- Add composition and internal-cycle topology tests plus internal app runner/certification
  registration; do not add an app entry-point registration, public target resolver, output renderer,
  or discovery id.

### 9. `make external read ingress validation asynchronous`

- Make the generic executor ingress hook async.
- Move selective runtime-config filesystem loading to `spawn_blocking`.
- Convert all external-read implementers without retaining a sync fallback.
- Replace the now-colocated collection/validation callbacks with one async EVM read-route
  validator; do not repair and retain the deleted portfolio EVM validator.
- Add generic runtime, app, and adapter boundary tests for both EVM read state families.

### 10. `make portfolio snapshot pure operation composition`

- Define `PortfolioReportOperation` in the existing portfolio operation package around the
  store-read, snapshot-computation, and report-projection states.
- Make `PortfolioSnapshotOperation` do only normalized config projection, BTC/EVM collector child
  calls, and one report-operation call with their receipt handles.
- Pass family receipt handles directly to `SelectHoldingsState`; their input edges are the readiness
  barrier.
- Delete direct portfolio state construction; commit 7 has already deleted the receipt fan-in, so
  this extraction adds no new value or replay path.
- Keep `mfm.portfolio/snapshot@1` as the sole app entry point and keep its public snapshot/report
  output contract.
- Add graph tests proving the root contains only operation composition, every report node depends on
  all required collector receipts, and no family observation bypasses storage.

### 11. `docs: audit the second round evm architecture`

- Search for every deleted package/type/schema/state/entry-point assertion.
- Reconcile `README`, design, architecture, routing, transaction, persisted-surface, app, crate
  README, rustdoc, and metadata-contract text.
- Add an explicit supersession notice for conflicting target/definition-of-done passages retained
  in `PROBLEM_EVM_SCATTERED.md`; do not let searches treat that historical charter as current.
- Record final package/state/entry-point inventories.
- Make no implementation or schema change in this audit-only commit.

## Verification matrix

### Transaction and signing

- One canonical EIP-1559 signing/finalization path remains.
- No raw signed transaction is serialized, cloned into persistent values, logged, or exposed by
  errors.
- CALL and CREATE estimation contain the exact nonce and fee observations later signed.
- Post-submission provider outages never terminalize trusted on-chain success or cause rebroadcast.
- Reverted transactions consume nonce/gas, expose no logs or contract address, and remain terminal
  domain outcomes rather than adapter errors.
- Complete successful logs are inspectable by downstream operation code.

### Contract validation

- Config admission proves its complete expected-return set fits the aggregate budget.
- Live reading stops at the first exceeded bound and never retains all raw plus hex responses.
- Code and every call use the exact hash anchor and finish with number-to-hash canonicality.
- Live reduction and evidence-only replay use the same semantic checks.
- Runtime config, routes, and live sessions are absent during replay.

### Balance collection

- Empty, duplicate, unsorted after deserialization, zero-contract, zero-chain, and over-limit configs
  fail closed.
- Normalization produces the same draft for reordered input sources.
- A collection operation expands to exactly one read and one atomic record state.
- Native-only, token-only, and mixed sets work.
- One session resolves latest once; token decimals are read once per distinct contract; balance
  reads use the exact hash; canonicality is rechecked once.
- Per-run and per-source concurrency bounds hold while output order remains deterministic.
- The complete fact batch and checked collection receipt commit in one managed-write append; a
  failed record attempt exposes neither a partial fact batch nor a receipt.
- Overlapping collectors derive the same source fact identity without sharing run identity.
- Multiple parent scopes call the same operation topology without a portfolio dependency.
- The internal scheduler root calls the same operation and exposes only its receipt; no app
  entry point, public setup target/resolver, discovery id, or renderer exists for the EVM collector.
- Portfolio reads the exact EVM and BTC facts named by same-run collector receipts from the store;
  it never consumes direct EVM balance material.
- EVM collection replay exists only in `mfm-adapters-evm`, works without runtime config, and rejects
  tampered anchors, requests, responses, fact batches, receipts, outputs, or lineage.

### Portfolio composition and report

- `mfm.portfolio/snapshot@1` remains the sole public application objective.
- `PortfolioSnapshotOperation` constructs no family state and contains no RPC, ABI, fact-recording,
  hydration, balance/report computation, or rendering implementation.
- Its graph is collector operation calls followed by one `PortfolioReportOperation` call.
- `PortfolioReportOperation` expands to exactly the store-read selection, pure snapshot assembly,
  and pure structured-report projection states shown above.
- The report cannot start until every required collector receipt commits.
- Receipt-pinned reads deterministically accept duplicate claims with exact content identity and
  reject different-content older/latest substitutes, missing facts, wrong descriptors, subjects or
  anchors, and incomplete source coverage.
- `SelectHoldingsState` replay uses only retained query/hydration evidence at its retained shared
  `StoreReadFrontier`; it never consults the current fact index, EVM route, or live session.
- Portfolio replay verifies selection, snapshot, and report only; it never acquires an EVM
  collection replay path from app or portfolio adapters.
- Every network retains its collector anchor. No test or reducer invents a simultaneous cross-chain
  tip or an implicit latest/max-age policy.
- All-EVM, all-BTC, and mixed portfolios use the same store-backed report topology.
- Public outputs remain only the computed `PortfolioSnapshot` and structured `PortfolioReport`;
  family receipts and fact identities stay internal.

### Runtime and transport

- External-read ingress performs no synchronous filesystem IO on async workers.
- Redirects are rejected and never contacted.
- Implicit HTTP retries are disabled; only ledger-owned recovery can repeat submission.
- A session stays on one endpoint for its attempt and probes chain identity once.
- `source_ref` remains redacted audit provenance and does not become semantic config.
- Provider availability blocks/resumes; invalid certified or retained semantic material fails.
- Endpoints, auth headers, file paths, provider bodies, and secrets never enter evidence or public
  diagnostics.

### Architecture and deletion

- Cargo metadata contains exactly the six target EVM packages and no removed package.
- `mfm-states-evm` contains exactly four state kinds: balance collection, balance recording,
  transaction submission, and contract validation.
- `mfm-adapters-evm` owns their bindings and no operation topology.
- `mfm-states-evm` has no portfolio, app, runtime, transport, signer-implementation, or storage
  dependency.
- Portfolio state/adapter packages define no EVM balance fact and contain no EVM RPC, ERC-20 codec,
  fact publication, or collection replay implementation; report selection only consumes the
  generic fact contract.
- `mfm-op-evm-collectors` exposes one composable receipt-producing operation; its internal scheduler
  wrapper calls that operation and is absent from public app discovery.
- App setup/discovery exposes exactly one public objective: `mfm.portfolio/snapshot@1`.
- The portfolio root depends on BTC/EVM collector operations and the portfolio report operation,
  not on collector state kinds or direct protocol outputs.
- Production source and current contract documentation, excluding explicitly marked historical
  charters and this deletion ledger, contain no old state names, portfolio EVM balance schemas,
  stale five-package claims, proposed EVM entry-point/target/output names, compatibility shims, or
  fallback paths.

## Required commands

Use focused checks while each commit is developed, including the touched package tests and:

```bash
cargo fmt --all -- --check
cargo test -p mfm-integration-tests --test cargo_metadata_contract
```

Before every commit, run the repository-required gates:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

After the major vertical move and again for final merge readiness, run:

```bash
nix run .#ci
```

## Closure evidence

The goal may be closed only when all definition-of-done items pass on one exact clean commit.

Acceptable final evidence includes:

- the full 40-character Git SHA;
- `git status --short` proving a clean worktree at that SHA;
- Cargo metadata proving the exact package/dependency surface;
- focused test results for every finding and collector behavior above;
- `nix run .#check`, `nix run .#test`, and `nix run .#test-db` results for each commit as required;
- a final `nix run .#ci` result cryptographically or operationally associated with the exact SHA;
  and
- a stale-name/dependency/schema audit over the exact SHA.

A CI log produced after a commit timestamp but containing no SHA is insufficient. Unretained claims
that architect agents reviewed the change are also insufficient. Reviews may guide the work, but
completion is established by inspectable code, retained history, exact-SHA verification, and the
defined architecture contracts.

## Definition of done

The second round is complete only when all of the following are true:

- the EVM collector is an independent, certified, replayable library operation with no portfolio
  dependency and no public app objective, and internal scheduled cycles wrap that same operation;
- portfolio calls the BTC/EVM collector operations and then one store-backed report operation
  instead of owning or reconstructing either collector;
- `mfm.portfolio/snapshot@1` remains the sole public entry point;
- both BTC and EVM report inputs are re-read from the store under same-run receipt content authority,
  with byte-identical claims treated as equivalent rather than adding occurrence identity;
- report replay uses retained query/hydration evidence at the retained `StoreReadFrontier` and never
  reads the current fact index, EVM route, or live session;
- no direct EVM snapshot/value path reaches portfolio computation;
- `PortfolioCollectionReceipt`, its assembler state, app runner, replay verifier, and generic
  fan-in wrappers are absent; typed family receipt edges are the only collector-to-report plumbing;
- the complete fact batch and checked receipt are one managed-write append; failure exposes neither
  partially, already committed sibling facts remain append-only, and no report is emitted until
  every required receipt commits;
- every collector/network keeps its own canonical pin, and freshness/as-of policy is explicit rather
  than inferred from the fact index;
- no collector-only capability, transport, app-ingress, address/model, or replay vertical is
  introduced; the reusable operation/state family uses the shared EVM layers;
- provider outages cannot terminalize post-submission observation or cause duplicate mutation;
- gas estimation and signing share one exact transaction description;
- successful receipt logs are safely consumable and reverted receipts cannot contain logs;
- contract-validation configs are feasible by construction and live memory is bounded while reads
  are accumulated;
- external-read admission performs no blocking filesystem IO on async workers;
- redirect following and implicit HTTP retries are disabled;
- design documentation consistently treats `source_ref` as audit provenance;
- write, keystore signing, CREATE, Call, and exact-anchor validation remain reusable;
- exactly six narrow EVM packages remain with the dependency directions above;
- every superseded type, schema, module, registration, and assertion is deleted rather than hidden;
- all focused and repository gates pass; and
- final CI evidence is tied to the exact clean closing SHA.
