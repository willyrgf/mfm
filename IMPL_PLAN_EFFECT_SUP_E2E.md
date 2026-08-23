# Effect support and EVM contract lifecycle e2e implementation plan

## Status

The original six-commit implementation is complete. `FIXES_IMPL_PLAN_EFFECT_SUP_E2E.md` is the
current corrective follow-up and its seven ordered commits are also complete. Where the two plans
conflict, the corrective plan and the repository's current design documents are authoritative; the
historical implementation detail below remains only as the record of the original cut.

This plan was reviewed against `AGENTS.md`, `docs/design.md`, `docs/architecture.md`,
`docs/code-quality.md`, `docs/build-and-verification.md`, the current kernel/runtime/storage/EVM
code, and the locked dependency and Nix task graphs. Five architects independently reviewed the
generic Effect protocol, Runtime/Journal ownership, EVM mutation safety, implementation minimality,
and handoff/verification quality. Their conflicting recommendations are resolved here; the engineer
should implement this design rather than reopen the alternatives unless repository evidence makes
one of the frozen assumptions false.

The first scenario imports an ephemeral development wallet into MFM's in-process keystore, funds it
from pinned Reth's unlocked development account, deploys a first-party contract through one Effect,
configures it through a second Effect, reads the configured value through an anchored Read, and
returns a content-addressed report.

## Outcome

MFM gains one generic durable Effect protocol and one EVM transaction implementation. Each semantic
EVM transaction is one authored `ExecuteEvmTransaction` Effect. Reservation, signing, exact-byte
custody, submission, reconciliation, and settlement are internal durable phases of that one Effect;
they are not separate Program States.

The safety contract is:

> Provider submission may be attempted more than once, but every attempt for one Effect uses the
> same EffectId, command, nonce, signature, transaction hash, and exact raw transaction bytes.

This is not exactly-once transport. It is one authorized semantic transaction across duplicate
submissions of identical signed bytes. Runtime durably records the complete nonce-free command
before adapter entry. The EVM authority then reserves one nonce and retains the exact signed bytes
before provider submission.

## Lowest-complexity decisions

| Question | Frozen decision | Complexity deliberately excluded |
| --- | --- | --- |
| EVM Program model | One `ExecuteEvmTransaction` Effect per transaction | Preparation Effect, second EffectId, Program-visible prepared transaction |
| Capability injection | Reuse `CapabilityInjection`; `InjectionWriter` stays Pure-only | Injected Effects and recursion rules |
| Retained history | Preserve the current authoritative Pure/Read event-log contract; recompute Effect command and EffectId | Full Pure/Read semantic replay and its unbounded read-time work |
| Effect conclusion wire | Adjacency supplies the pending EffectId; evidence binds it | Duplicating EffectId or command in the conclusion |
| Program capacity | Conservative global frame weight | Reverse-DAG path optimization |
| Runtime ambiguity | Preserve caller-driven recovery | Internal reload/retry loops for `Indeterminate` |
| EVM outbox | Reservation, prepared bytes, and settlement facts | Broadcast-command/authorization fact |
| Nonce disagreement | Leave unresolved and append nothing | Unauthenticated durable integrity blocks |
| Provider recovery | Receipt-first; submit once per invocation; resume later | Sleeps, polling loops, transaction lookup, error-string parsing |
| Settlement | Fixed managed-Reth development policy, not production-registered | Configurable confirmation/finality policy without a production contract |
| Lifecycle surface | Fixture-local Operation and report | Public lifecycle API with no product consumer |
| Lost submission acknowledgement | Normal submit-and-yield adapter flow plus one focused connection-drop test | Extra acknowledgement-loss proxy/decorator/server/parser |

The rejected two-Effect design adds four Journal frames to this two-transaction scenario, duplicates
bindings/evidence/failure routing, and creates another durable identity without adding a safety
boundary. The generic prepared frame already authorizes the entire command, while the independent
EVM authority owns the internal crash boundaries.

The rejected full-replay prerequisite would also change an existing authoritative design contract.
Today qualified Pure/Read conclusions are retained event-log authority; Runtime validates their
contracts and Read evidence binding but does not rerun all historical State code. Replaying that
code would add potentially unbounded CPU to `Runtime::read` and would not authenticate a forged
genesis or binding when the Store/process authority itself is untrusted. That broader provenance
design is not part of Effect support.

## Scope

The implementation includes:

- generic `EffectState` and `EffectCapabilityContract` contracts;
- stable Effect identity parsing, value grammar, derivation, and contract vectors;
- Effect authoring through the existing injection policy with Pure-only support topology;
- Program v3 with an explicit top-level wire discriminator and a real Pure/Read/Effect sum;
- Journal frame v2 with separate prepared and concluded Effect records;
- Runtime association, fold, execution, concurrency, cancellation, and recovery behavior;
- a conservative frame-weight capacity bound;
- typed thread-affine secp256k1 import and recoverable signing;
- fixed nonce-free EIP-1559 command, settlement evidence, and anchored-call value contracts;
- a separate EVM transaction-authority port and append-only PostgreSQL implementation;
- exact type-2 RLP signing, raw submission, receipt reconciliation, and anchored contract calls;
- a first-party Solidity fixture compiled by pinned `solc` during the managed test;
- focused kernel, crypto, domain, live-adapter, PostgreSQL, and Reth tests; and
- one managed `effect-e2e` Nixfied task in final CI.

## Non-goals

The first cut does not add:

- a generic arbitrary mutation API;
- a reusable or production-exposed contract-lifecycle Operation;
- CLI, REST, configuration, or `ComposedRuntime` transaction registration;
- persistent encrypted private-key custody or host-process keystore recovery;
- multiple simultaneous unsettled nonces for one account domain;
- transaction cancellation, nonce rollback/reuse, replacement, or fee bumping;
- externally shared wallet/nonce ownership;
- multiple transaction endpoints for one chain instance;
- background scheduling, automatic retries, sleeps, or Runtime timeout policy;
- production chain finality or reorg guarantees;
- writable transaction-authority snapshot restoration;
- a durable block based only on a pending-nonce observation;
- legacy Program, Journal, or PostgreSQL run-history readers; or
- committed Solidity compiler output, generated bytecode, or third-party contract source.

“Cold recovery” in this milestone means rebuilding Runtime, assembly, provider, and database handles
inside one test process while the same ephemeral key-bound signer owner remains alive. It does not
mean recovering a private key after host-process termination.

## Existing baseline and complete cutover

The current system has only Pure and Read execution:

- `mfm-capabilities` owns `ReadCapabilityContract`;
- `mfm-program` owns `State`, `PureState`, `ReadState`, and Read capability injection;
- Program v2 has no top-level version/domain field;
- Runtime registers Pure and Read drivers and `register_adapter` callbacks;
- Journal uses `mfm.run.frame.v1` and fused Pure/Read conclusions;
- PostgreSQL requires `mfm.run-history-postgres.v1`;
- `mfm-signing` exposes free-form string request/result fields;
- `mfm-keystore` stores replaceable opaque bytes but cannot import or sign typed keys;
- `mfm-evm-live` performs duplicate-safe Reads only; and
- production composition intentionally exposes no transaction authority.

The generic Effect commit is an inseparable format/API cutover:

- Program v2 is deleted and v3 is the only reader/writer;
- Journal frame v1 is deleted and v2 is the only reader/writer;
- the PostgreSQL run-history marker becomes `mfm.run-history-postgres.v2`;
- old run-history installations fail exact admission and require a fresh baseline;
- all workspace consumers and golden vectors move in the same commit; and
- no compatibility enum, migration reader, fallback parser, or feature flag remains.

Adding only an Effect enum variant is insufficient: every Pure/Read-only v2 Program would otherwise
still deserialize. Program v3 therefore requires the exact top-level field
`"domain":"mfm.program.v3"`, uses `deny_unknown_fields`, and rejects bytes without that field.

## Ownership after the cut

| Owner | Added responsibility | Explicit exclusion |
| --- | --- | --- |
| IDs / Values | EffectId spelling and checked persisted grammar | EffectId derivation or execution |
| Capabilities | Read and Effect capability/evidence contracts | State outcomes or IO |
| Program | Effect State contract, authoring, v3 graph, frame-weight validation | Adapter registry, execution, Journal wire |
| Runtime | EffectId derivation, Program association, sole semantic fold, adapter entry | Persisted frame encoding or physical storage |
| Journal | Exact v2 frame wire, closure, chain, context-free Effect adjacency | Selected declaration or State semantics |
| Store | Unchanged exact-head atomic frame append | Effect, nonce, signer, EVM, or retry semantics |
| EVM domain | Deterministic transaction/call values and States | Provider, signer, authority, or Runtime handles |
| EVM authority port | Exact append-only nonce/raw/settlement contract | Run history or provider policy |
| PostgreSQL | Concrete authority schema plus existing Store/config implementations | Program or EVM interpretation |
| Live EVM | Provider ingress and transaction adapter orchestration | Program topology or durable run history |
| Application | Test-only composition in the ignored integration test | Production transaction product/API |

`docs/architecture.md` and `docs/design.md` must carry this ownership and execution contract in the
same commits that introduce it.

## Cross-cutting invariants

1. State implementations remain deterministic and perform no ambient IO.
2. Runtime remains the sole Program-aware fold owner.
3. Journal owns exact wire/closure/history qualification only.
4. Store remains mechanical and unchanged.
5. No Effect adapter call occurs before an exact `StateEffectPrepared` frame is known inserted.
6. A prepared append with ambiguous acknowledgement causes zero adapter calls in that invocation.
7. Every Effect adapter accepts `(EffectId, exact command)` repeatedly.
8. An EffectId/command mismatch fails before adapter entry.
9. A nonce is reserved against the complete command reference before signing.
10. Exact signed bytes and their EVM hash are retained before provider submission.
11. A retry never chooses another nonce, signs replacement bytes, or mutates retained facts.
12. No database transaction or lock spans signer or provider IO.
13. One nonce domain has at most one unsettled reservation.
14. Confirmed success and confirmed revert both settle and consume the nonce.
15. All other observations remain unresolved and cannot release the next nonce.
16. Authority replacement changes a public epoch and rejects an old Program before signer/provider IO.
17. Programs, values, frames, evidence, and reports remain content addressed.
18. Hashed structures contain no floats.
19. Private keys, mnemonics, credentials, locators, and raw transactions never enter Program, C0,
    Journal, Store metadata, RunView, public output, error detail, or logs.

## Reference topology

The managed test authors this fixture-local graph:

```text
Admit checked deployment transaction context      C0
  -> Execute deployment transaction               Effect
  -> Build configuration EIP-1559 command         Pure
  -> Execute configuration transaction            Effect
  -> Build anchored value() intent                 Pure
  -> Read configured value at receipt block hash  Read
  -> Build content-addressed report                Pure
```

Each transaction Effect internally follows:

```text
Runtime EffectPrepared(effect_id, command)
  -> authority reserve exact nonce
  -> signer signs exact digest
  -> authority retain exact raw transaction and hash
  -> provider receipt-first reconciliation / exact-byte submission
  -> authority retain terminal settlement
  -> Runtime EffectConcluded(evidence, outcome)
```

The Program contains neither an outbox item nor a prepared-transaction value. Those are durability
mechanisms behind the one declared mutation boundary.

Before `Runtime::start`, the fixture compiles initcode and directly constructs the checked Create
command plus `EvmTransactionContext { caller_context: AwaitingDeployment { binding }, command }` as
C0. Do not add a Pure State merely to rebuild inputs already known at admission. The one closed
fixture-only caller-context enum is exactly:

```text
LifecycleContext = AwaitingDeployment { binding }
                 | AwaitingConfiguration { binding, deployment }
                 | BothTransactionsComplete { deployment, configuration }
```

`deployment` and `configuration` are the exact typed transaction confirmations. Fixed ABI selectors,
values, gas, and fees are fixture constants checked against solc output; they do not bloat the
context. Both Effect occurrences and the anchored Read use the same `<LifecycleContext>`
monomorphization and therefore the same implementation/input/output contracts. Fixture Pure States
advance the closed enum between occurrences. The generic transaction Effect and anchored Read carry
it through unchanged, so the final Pure report can include both transaction hashes without putting
workflow metadata into an EVM command.

The configuration Effect output is not directly type-compatible with the anchored Read input. One
fixture-local Pure State consumes `EvmTransactionCompletion<LifecycleContext>`, takes the contract
address from the closed context and the exact anchor from configuration confirmation, builds the
fixed ABI `value()` intent, and returns `AnchoredContractCallContext<LifecycleContext>`. Keep this
explicit graph node and its frame weight; do not hide workflow conversion inside either generic
capability.

## Generic Effect protocol

### Shared errors

Rename `ReadPreparationError` to one unit `PreparationError`, used by both deterministic Read and
Effect preparation. Rename `ReadAdapterError` to one shared:

```rust,ignore
pub enum AdapterError {
    Unavailable,
    Internal,
}
```

Update every existing Read adapter and call site in the same generic Effect commit. Do not add a
base capability trait solely to share `contract_id`; Read and Effect evidence contracts remain
separate because their authority differs.

### Effect capability and State contracts

Add to `mfm-capabilities`:

```rust,ignore
pub trait EffectCapabilityContract: Send + Sync + 'static {
    type Command: MfmValue;
    type Evidence: MfmValue;

    fn contract_id() -> Result<StableId>;

    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> Result<()>;
}
```

Add to `mfm-program`:

```rust,ignore
pub trait EffectState<C>: State
where
    C: EffectCapabilityContract,
{
    fn prepare(
        input: &Self::Input,
    ) -> std::result::Result<C::Command, PreparationError>;

    fn interpret(
        input: Self::Input,
        evidence: &C::Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure>;
}
```

The Runtime Effect callback receives only `&EffectId` and `&C::Command`; provider, authority, and
signer handles are captured by the registered adapter. The callback returns `C::Evidence` or the
shared `AdapterError`. Keep the current `register_adapter<C, B, F>` name for Read callbacks. Add
`register_effect<S, C>` for the State driver and `register_effect_adapter<C, B, F>` for the mutating
callback; do not overload the Read method with another hidden mode.

A preparation error is trusted local failure: Runtime returns `RuntimeError::Internal`, appends no
prepared frame, and calls no adapter. Adapter `Unavailable` maps to Runtime `Unavailable`; adapter
`Internal`, panic, malformed typed return, or evidence mismatch maps to redaction-safe Internal.

### EffectId

Add `EffectId` to `mfm-ids` with exact spelling:

```text
effect:sha256-jcs-v1:<64 lowercase hexadecimal characters>
```

`mfm-ids` owns construction from 32 digest bytes, parsing, formatting, serde, and rejection of every
other spelling. Add `StringGrammar::EffectId` in `mfm-values` and map the identity shape in
`mfm-program-derive`.

Runtime owns derivation because `mfm-canonical` already depends on IDs. Hash the JCS canonical bytes
of exactly this preimage:

```json
{
  "command_ref": {
    "schema_id": "<command schema id>",
    "content_digest": "<command exact-byte digest>"
  },
  "declaration_index": 0,
  "domain": "mfm.effect-id.v1",
  "program_ref": {
    "schema_id": "<exact mfm-program-document version 3 SchemaId>",
    "content_digest": "<Program exact-byte digest>"
  },
  "run_id": "run:sha256-jcs-v1:<64 lowercase hex>"
}
```

Field names and the domain are exact. `declaration_index` is the zero-based Program index, checked
as `u16` and serialized as a JSON integer. `ContentRef` fields use their existing canonical object
wire. The digest is SHA-256 over these JCS bytes and is wrapped as `EffectId`.

Do not add capability ref, binding ref, attempt, nonce, or adapter identity: Program ref plus index
already binds the declaration, and command ref binds the prepared invocation.

Freeze independent known-answer vectors in IDs (spelling) and Runtime (preimage/derivation). The
reason the prepared frame persists the derived ID is derivation-drift detection: if upgraded code
derived another ID for a settled-but-not-concluded command, it could otherwise reserve the next
nonce and repeat the semantic mutation.

### Program v3

Replace the optional-Read representation with an actual closed sum:

```text
Execution::Pure
Execution::Read {
    capability_contract_ref,
    intent_contract_ref,
    evidence_contract_ref,
    binding_ref,
}
Execution::Effect {
    capability_contract_ref,
    command_contract_ref,
    evidence_contract_ref,
    binding_ref,
}
```

The canonical Program document requires `domain: "mfm.program.v3"`. Preserve the existing explicit
`kind` discriminants, exact contract refs, forward-only `u16` successors, JCS encoding, and strict
unknown-field rejection. Its `SchemaIdentity` keeps kind `PersistedContract`, name
`mfm-program-document`, the general float-free canonical terminal shape, and changes the exact
version from `2` to `3`; freeze the resulting full SchemaId in contract vectors. Delete all v2
structs, schema IDs, fixtures, and fallback paths.

Add `OperationExpansion::effect<S, C>(setup)` parallel to `read`. Both use the existing
`CapabilityInjection<S>` callbacks before and after the kernel-owned occurrence. Generalize that
policy's documentation from Read-only to exact capability/State policy, but keep `InjectionWriter`
capable of emitting direct Pure States only. It cannot emit Read, Effect, child Operation, Match, or
failure handler topology. The EVM transaction capability uses identity injection: no before/after
support State.

Keep current exact typed failure routing. An Effect's confirmed domain failure follows its authored
failure successor; preparation, adapter, Store, cancellation, and panic failures remain Runtime
errors and append no conclusion.

### Capacity

Retain the current conservative global approach. Compute:

```text
1 + sum(execution_weight for every State declaration) <= MAX_RUN_FRAMES

Pure   = 1
Read   = 1
Effect = 2
Match  = 0
```

The leading one is genesis. Use checked `u64` arithmetic during authoring and decode; do not persist
a cached weight. This bound may reject a graph whose mutually exclusive branches could each fit,
but it is simple, safe, deterministic, and consistent with the current global declaration bound.
There is no product requirement to maximize branch capacity.

### Journal frame v2

Change the exact frame domain to `mfm.run.frame.v2`. Preserve admission and fused Pure/Read records,
and add exactly:

```text
StateEffectPrepared {
    effect_id,
    command,
}

StateEffectConcluded {
    evidence,
    outcome,
}
```

`command`, `evidence`, and `outcome` are content refs with exact frame-local canonical object
closure. `effect_id` is the checked scalar. The conclusion has no direct EffectId or command field:
it can only close the immediately preceding pending prepare, and evidence must bind to both. The
current EVM outcome types also avoid echoing the command.

Journal qualification owns these context-free structural rules:

- admission is the first and only admission record;
- a prepare may be the final record of a valid prefix;
- if a prepare is not final, the next record is an Effect conclusion;
- an Effect conclusion is valid only immediately after a prepare;
- Pure, Read, or another prepare cannot intervene; and
- a conclusion closes the pending structural pair.

Journal does not know whether the selected Program declaration is Effect, run State code, derive an
EffectId, bind evidence, interpret outcomes, or select successors. Those checks stay in Runtime.

### Runtime association and fold

Extend immutable assembly with exact Effect State drivers and `(capability contract ref, binding
ref)` Effect callbacks. Registration/`finish` rejects duplicate or internally incompatible driver
and callback entries without seeing a Program. `RuntimeAssembly::associate(&Program)` then
pre-resolves every exact State mode, capability, value contract, and binding, and rejects a missing
or wrong-kind Effect registration before Store IO, as current Read association does.

Use an internal fold state equivalent to:

```text
Selected { declaration, driver, input }
EffectPending { declaration, driver, input, effect_id, command, command_ref }
Succeeded
Failed
```

Both nonterminal states render publicly as `RunViewState::Runnable`; no new public status is needed.

When folding any retained `StateEffectPrepared`, Runtime must:

1. prove the currently selected declaration is the associated Effect State;
2. decode and qualify the retained command against its exact command contract;
3. run deterministic `EffectState::prepare` on the folded input;
4. canonicalize the prepared command and compare its exact bytes/ref with the retained command;
5. derive EffectId from the retained Run/Program/index/command ref; and
6. compare it with the retained EffectId before producing `EffectPending`.

When folding its conclusion, Runtime binds evidence to the pending EffectId and command, qualifies
the retained outcome against the selected success/failure contract, and advances through the
recorded outcome branch. As with current Pure/Read retained conclusions, the conclusion is event-log
authority: cold fold does not rerun `interpret` or compare a newly interpreted historical outcome.
Hot execution always interprets before it constructs the conclusion.

Existing retained Pure/Read handling stays at its current authority level. Do not add historical
`PureState::evaluate`, `ReadState::prepare`, or `ReadState::interpret` replay in this project.
`Runtime::read` may run deterministic Effect preparation solely to qualify retained Effect records;
it performs no Read or Effect adapter IO and appends nothing.

### Runtime execution and ambiguity

For a selected Effect, one progression invocation is:

1. deterministically prepare and canonicalize the command;
2. derive EffectId;
3. construct `StateEffectPrepared` against the exact current head;
4. call Store append;
5. invoke the adapter only after `Inserted`, or when cold fold already found that exact prepare;
6. bind returned evidence to EffectId and command;
7. interpret the evidence into typed success/failure;
8. construct `StateEffectConcluded` against the prepared head; and
9. append the conclusion atomically.

Preserve current Store recovery behavior:

- prepared append `Indeterminate`: return the existing unit `RuntimeError::Indeterminate` with zero
  adapter calls; the caller already holds the caller-supplied RunId used to resume;
- conclusion append `Indeterminate`: return the same error; caller resumes;
- `NotInserted`: perform the existing complete reload/fold and converge on the winner; and
- do not introduce an Effect-specific internal retry or reload loop.

On resume, a committed prepare is found and the adapter may run; an absent prepare is regenerated at
the old exact head. A committed conclusion is folded; an absent conclusion causes the same pending
Effect to reuse authority evidence. Concurrent callers may enter the same pending adapter, so the
adapter/authority contract—not a Runtime mutex—provides convergence.

Dropping a future is safe at every boundary: either no frame/fact exists, or one atomic append may
have committed and the next caller resolves it. Adapter error or panic leaves the prepared Effect
pending and appends no conclusion.

## Signing and keystore cut

### Public signing contract

Replace free-form request/result strings with checked types:

- `SigningDigest`: exactly 32 bytes;
- purpose: the existing checked `StableId`, with no second wrapper;
- `UncompressedSec1PublicKey`: exactly 65 bytes and prefix `0x04`;
- `CompactRecoverableSignature`: exactly 64 bytes plus recovery ID `0..=3`; and
- `PublicSignerIdentity`: signer route ID plus a public algorithm/key value whose content ref is the
  key-instance identity.

The public key-instance value contains the algorithm ID and exact SEC1 public key. Derive its
identity with the existing typed-value content-ref machinery; do not accept a caller-chosen key
instance label. The recoverable secp256k1 algorithm ID is exactly
`mfm.signing.secp256k1-ecdsa-recoverable@1`.

The in-process keystore signer route is the exact `StableId`
`mfm.signer.in-process-keystore@1`. It identifies this signer implementation, not an owner instance;
the derived public-key content ref distinguishes key instances, and two owners holding the same key
are semantically interchangeable. `KeystoreOwner::start()` takes no caller route label.
`KeystoreOwner::import_secp256k1(SecretSecp256k1Scalar)` is async and returns a key-bound
`KeystoreSigner`; duplicate import returns another handle with the same byte-identical
`PublicSignerIdentity`. The fixture keeps the owner controller and signer handle alive across cold
Runtime reconstruction.

`mfm-signing` adds `mfm-values` and `mfm-program-derive` dependencies so `PublicSigningKey` and
`PublicSignerIdentity` are first-class persisted public values. It does not depend on Program,
Runtime, EVM, provider, or keystore implementation crates.

Make every `Signer` handle key-bound:

```rust,ignore
pub type SigningFuture = Pin<
    Box<
        dyn Future<Output = Result<CompactRecoverableSignature>>
            + Send
            + 'static,
    >,
>;

pub trait Signer: Send + Sync + 'static {
    fn public_identity(&self) -> &PublicSignerIdentity;
    fn sign(&self, digest: SigningDigest, purpose: StableId) -> SigningFuture;
}
```

The request does not repeat signer, key-instance, algorithm, or public-key fields already fixed by
the handle. The result contains only the compact recoverable signature. The EVM adapter captures the
expected handle, compares its public identity with the command, derives the Ethereum address as the
last 20 bytes of `keccak256(uncompressed_sec1_public_key[1..])`, and compares that address with the
command wallet before authority/provider IO. It uses the exact purpose
`mfm.evm.sign-eip1559@1`, accepts only recovery parity 0 or 1, and also recovers and compares the
command sender from the actual signature.

### Crypto dependency and behavior

Pin in workspace dependencies:

```toml
k256 = { version = "=0.13.4", default-features = false, features = ["ecdsa", "std"] }
zeroize = "=1.9.0"
```

Both exact versions are already in `Cargo.lock`. Only `mfm-signing` and `mfm-keystore` consume
`k256`. `mfm-keystore` and the secret-constructing app test consume `zeroize`; `mfm-signing` need not.
`mfm-keystore` uses recoverable prehash signing over the supplied 32-byte digest. `mfm-signing` owns
a pure public recovery/verification function over its checked digest/signature/public-key types,
backed by the same pinned k256; live EVM uses that function and has no direct k256 dependency. Freeze
deterministic RFC6979 output, low-S normalization, scalar validation, public key derivation,
recovery, and Ethereum parity with known-answer tests. EVM Keccak and transaction RLP remain in the
live EVM crate.

### Thread-affine owner

Replace the opaque overwrite/list/remove keystore API with typed secp256k1 import returning one
key-bound signer handle. Import accepts a private-field `SecretSecp256k1Scalar` wrapper around
`Zeroizing<[u8; 32]>`; neither the wrapper nor any accessor exposes Debug/Display/serde. Parse and
retain a `k256::ecdsa::SigningKey`, not generic bytes.

The owner design is fixed:

- create the non-`Send`, non-`Sync` `Keystore` inside a dedicated owner thread;
- communicate through a bounded Tokio MPSC channel of capacity 64;
- the async handle sends owned zeroizing import commands or checked signing commands;
- the owner uses blocking receive and answers through one-shot channels;
- duplicate import of the same public key returns another handle to the same retained key;
- one named constant fixes the maximum at 64 distinct key instances; delete public
  `KeystoreConfig` and its unused capacity knob;
- dropping the last channel sender closes the receiver; the owner then exits and drops/zeroizes all
  `SigningKey` values;
- an explicit async shutdown consumes the unique owner controller, requests owner exit, moves only
  the OS join handle into `spawn_blocking`, and awaits it immediately; no `Drop` implementation
  blocks a Tokio worker; and
- no secret-bearing type implements `Debug`, `Display`, serde, or error detail.

Keep compile-fail coverage proving `Keystore` itself is neither `Send` nor `Sync`.

## EVM deterministic contracts

### Placement and fixed forms

`mfm-evm` owns all Program-visible transaction and anchored-call values and State implementations.
Use private fields, checked constructors, and checked deserialization. Reuse one shared `EvmAddress`
across existing and new EVM values: exact lowercase `0x` plus 40 lowercase hexadecimal characters.
Likewise, use the shared checked `EvmHash` and `EvmU256` wrappers for existing provider-originated
block anchors and raw-unit evidence. Make the checked `EvmBlockAnchor` constructor public for exact
external qualification.

Add shared fixed-width wrappers:

- hash: exact lowercase `0x` plus 64 lowercase hexadecimal characters;
- U256: canonical decimal string (`"0"` or nonzero digit followed by digits), range checked;
- chain ID, nonce, and gas limit: `u64`, with chain ID and gas limit nonzero;
- authority epoch: exact 32 public bytes;
- creation initcode: `0..=49_152` bytes;
- call calldata: `0..=131_072` bytes;
- endpoint reference and public IDs: existing checked identity types.

No new hashed value contains a float. Do not leave any field described only as “bounded”.

Freeze explicit `MfmValue` identities rather than derive defaults for the new persisted primitives:

| Rust type | semantic namespace/name/version | schema name | canonical serde form |
| --- | --- | --- | --- |
| `EvmAddress` | `mfm.evm` / `address` / `1` | `mfm.evm-address` | transparent checked lowercase hex string |
| `EvmHash` | `mfm.evm` / `hash` / `1` | `mfm.evm-hash` | transparent checked lowercase hex string |
| `EvmU256` | `mfm.evm` / `uint256` / `1` | `mfm.evm-uint256` | transparent checked decimal string |
| `EvmAuthorityEpoch` | `mfm.evm` / `transaction-authority-epoch` / `1` | `mfm.evm-transaction-authority-epoch` | transparent canonical bytes, exactly 32 decoded bytes |
| `PublicSigningKey` | `mfm.signing` / `public-key` / `1` | `mfm.signing-public-key` | object: algorithm plus exact public-key bytes |
| `PublicSignerIdentity` | `mfm.signing` / `public-signer-identity` / `1` | `mfm.signing-public-signer-identity` | object: signer route plus public key value |
| `EvmChainInstance` | `mfm.evm` / `chain-instance` / `1` | `mfm.evm-chain-instance` | object: chain ID and genesis hash |
| `EvmTransactionRoute` | `mfm.evm` / `transaction-route` / `1` | `mfm.evm-transaction-route` | object: chain instance and endpoint ref |
| `EvmWalletIdentity` | `mfm.evm` / `wallet-identity` / `1` | `mfm.evm-wallet-identity` | object: sender and public signer identity |
| `EvmTransactionBinding` | `mfm.evm` / `transaction-binding` / `1` | `mfm.evm-transaction-binding` | object: route, epoch, wallet |
| `Eip1559TransactionCommand` | `mfm.evm` / `eip1559-transaction-command` / `1` | `mfm.evm-eip1559-transaction-command` | strict object and tagged action sum described below |
| `EvmTransactionSettlement` | `mfm.evm` / `transaction-settlement` / `1` | `mfm.evm-transaction-settlement` | strict object and tagged terminal-result sum described below |
| `EvmTransactionConfirmation` | `mfm.evm` / `transaction-confirmation` / `1` | `mfm.evm-transaction-confirmation` | strict success projection described below |
| `EvmTransactionRevert` | `mfm.evm` / `transaction-revert` / `1` | `mfm.evm-transaction-revert` | strict revert projection described below |
| `AnchoredContractCallResult` | `mfm.evm` / `anchored-contract-call-result` / `1` | `mfm.evm-anchored-contract-call-result` | anchor plus bounded return bytes |
| `AnchoredContractCallFailureReason` | `mfm.evm` / `anchored-contract-call-failure-reason` / `1` | `mfm.evm-anchored-contract-call-failure-reason` | tagged unit sum described below |

All new aggregate structs deny unknown fields. Every sum below uses exactly adjacent
`tag = "kind"`, `content = "value"`, `rename_all = "snake_case"`, and denies unknown fields; unit
variants omit `value`. `EvmAddress`, `EvmHash`, and `EvmU256` are transparent JSON strings. Public
byte fields are one transparent base64url-no-pad string: 32-byte epoch/digests encode to 43
characters, the 65-byte SEC1 key to 87, and variable initcode/calldata/return bytes use the same
spelling. The following field names and shapes are exact; examples are shown in JCS key order:

```text
PublicSigningKey
{"algorithm":"mfm.signing.secp256k1-ecdsa-recoverable@1","public_key":"<65-byte-base64url>"}

PublicSignerIdentity
{"public_key":<PublicSigningKey>,"signer_route":"mfm.signer.in-process-keystore@1"}

EvmChainInstance
{"chain_id":1,"expected_genesis_hash":"0x<64hex>"}

EvmTransactionRoute
{"chain_instance":<EvmChainInstance>,"endpoint_ref":<ContentRef>}

EvmWalletIdentity
{"sender":"0x<40hex>","signer_identity":<PublicSignerIdentity>}

EvmTransactionBinding
{"authority_epoch":"<32-byte-base64url>","route":<EvmTransactionRoute>,"wallet":<EvmWalletIdentity>}

Eip1559TransactionCommand / Create
{"action":{"kind":"create","value":{"initcode":"<base64url>"}},"binding":<EvmTransactionBinding>,"gas_limit":2000000,"max_fee_per_gas":"10000000000","max_priority_fee_per_gas":"1000000000","value":"0"}

Eip1559TransactionCommand / Call
{"action":{"kind":"call","value":{"calldata":"<base64url>","to":"0x<40hex>"}},"binding":<EvmTransactionBinding>,"gas_limit":200000,"max_fee_per_gas":"10000000000","max_priority_fee_per_gas":"1000000000","value":"0"}

EvmTransactionContext<K>
{"caller_context":<K>,"command":<Eip1559TransactionCommand>}

EvmTransactionSettlement / successful CREATE
{"block_anchor":{"hash":"0x<64hex>","number":"1"},"effect_id":"effect:sha256-jcs-v1:<64hex>","nonce":0,"result":{"kind":"success_create","value":{"created_address":"0x<40hex>"}},"transaction_hash":"0x<64hex>"}

EvmTransactionSettlement / successful CALL
{"block_anchor":{"hash":"0x<64hex>","number":"2"},"effect_id":"effect:sha256-jcs-v1:<64hex>","nonce":1,"result":{"kind":"success_call"},"transaction_hash":"0x<64hex>"}

EvmTransactionSettlement / revert
{"block_anchor":{"hash":"0x<64hex>","number":"2"},"effect_id":"effect:sha256-jcs-v1:<64hex>","nonce":1,"result":{"kind":"reverted"},"transaction_hash":"0x<64hex>"}

EvmTransactionConfirmation / created | called
{"block_anchor":<EvmBlockAnchor>,"result":{"kind":"created","value":{"created_address":"0x<40hex>"}},"transaction_hash":"0x<64hex>"}
{"block_anchor":<EvmBlockAnchor>,"result":{"kind":"called"},"transaction_hash":"0x<64hex>"}

EvmTransactionRevert
{"block_anchor":<EvmBlockAnchor>,"transaction_hash":"0x<64hex>"}

EvmTransactionCompletion<K> / EvmTransactionReversion<K>
{"caller_context":<K>,"confirmed":<EvmTransactionConfirmation>}
{"caller_context":<K>,"reverted":<EvmTransactionRevert>}

EvmReadSubject::AnchoredContractCall
{"kind":"anchored_contract_call","value":{"anchor":<EvmBlockAnchor>,"calldata":"<base64url>","target":"0x<40hex>"}}

EvmReadIntent / anchored call
{"chain_id":1,"operation":"mfm.evm.read-anchored-contract-call@1","route_ref":<EvmTransactionRoute-ContentRef>,"subject":<AnchoredContractCall-subject>}

EvmReadValue::AnchoredContractCall
{"kind":"anchored_contract_call","value":{"anchor":<EvmBlockAnchor>,"return_bytes":"<base64url>"}}

AnchoredContractCallContext<K>
{"caller_context":<K>,"intent":<EvmReadIntent>}

AnchoredContractCallResult
{"anchor":<EvmBlockAnchor>,"return_bytes":"<base64url>"}

AnchoredContractCallFailureReason
{"kind":"rejected"} | {"kind":"safe_failure"} | {"kind":"integrity_blocked"}

AnchoredContractCallCompletion<K> / AnchoredContractCallFailure<K>
{"caller_context":<K>,"result":<AnchoredContractCallResult>}
{"caller_context":<K>,"reason":<AnchoredContractCallFailureReason>}
```

`ContentRef` retains its existing exact `{ "content_digest", "schema_id" }` object wire (without
spaces after canonicalization). Existing `EvmBlockAnchor` keeps the object field names `hash` and
`number` while their values become the checked transparent wrappers. The anchored intent operation
ID is exactly `mfm.evm.read-anchored-contract-call@1`.

Freeze the resulting full semantic/schema IDs and canonical golden bytes in domain contract tests.
Write expected JSON/IDs as independent literal vectors, not expectations generated by the same
serializer/descriptor code under test.
Generic caller-context wrappers follow the existing descriptor-composition pattern and are tested
with two materially different `K` shapes. Transient digest/signature types are not `MfmValue` and
expose no serde; exact raw custody belongs to the authority port described below.

The `EvmAddress` cut is repository-wide: replace loose public address strings in existing EVM
values, live decoding, Portfolio/config/app construction, fixtures, contract JSON, and rustdoc in the
same commit. There is one serde-transparent address concept after the cut, not a transaction-only
parallel type.

The hash/quantity cut is equally current-only. Change `EvmBlockAnchor` to exactly
`{ number: EvmU256, hash: EvmHash }`; change the existing `EvmReadValue::Anchor` payload to that
checked anchor and `EvmReadValue::RawUnits` to `EvmU256`. The transparent wrapper strings preserve
the existing canonical JSON shape for valid values while tightening its generated schema and decode
contract. Migrate existing balance Read constructors/interpreters, live ingress, fixtures, and
golden schema/wire vectors in the same commit. Portfolio scaled values, sums, and report decimals
remain their existing decimal-string domain because they have scale/aggregation semantics rather
than the range of one EVM word. There is one provider-facing EVM hash and U256 concept after the cut.

### Chain, wallet, binding, and command

Add a transaction chain instance identified by `(chain_id, expected_genesis_hash)`. The public
transaction route also contains exactly one existing `endpoint_ref`. Endpoint identity selects the
provider for that binding but is not part of the nonce namespace. The supported test composition
registers exactly one `EvmTransactionRoute`; a Program naming another route fails ordinary exact
binding association. Generic Runtime gains no EVM-specific cross-registration rule, and deliberate
manual registration of multiple transaction routes is outside this cut. Endpoint-independent nonce
identity preserves the safe future direction without adding that product surface now.

The public wallet identity contains:

- exact sender address;
- key-bound `PublicSignerIdentity`.

The public Effect binding contains:

- the transaction route (which already contains the chain instance and endpoint ref);
- exact 32-byte authority epoch;
- wallet identity.

The one `Eip1559TransactionCommand` contains the complete binding plus:

- `action: Create { initcode } | Call { to, calldata }`;
- value as checked U256 decimal;
- gas limit as checked nonzero `u64`;
- max-priority-fee-per-gas as checked U256 decimal; and
- max-fee-per-gas as checked U256 decimal, not less than the priority fee.

`Create` owns the `0..=49_152` byte initcode and has no target. `Call` owns its target and
`0..=131_072` byte calldata. Do not encode this as `Option<address>` plus a shared byte field.

The command deliberately omits nonce, transaction type, access list, timeout, poll count,
confirmation count, settlement policy, and provider locator. Its value version intrinsically means
EIP-1559 type 2 with an empty access list and explicit gas/fees. The exact Effect capability version,
not a command or binding field, fixes development settlement behavior.

### Effect State and evidence

Add one capability/State pair:

```text
Capability: EvmTransactionEffect
State:      ExecuteEvmTransaction<K: MfmValue>
Input:      EvmTransactionContext<K> { caller_context, command }
Command:    Eip1559TransactionCommand (identity prepare)
Evidence:   EvmTransactionSettlement
Output:     EvmTransactionCompletion<K> {
              caller_context,
              confirmed: EvmTransactionConfirmation,
            }
Failure:    EvmTransactionReversion<K> {
              caller_context,
              reverted: EvmTransactionRevert,
            }
```

The exact source identities are `mfm.evm.capability.execute-transaction@1` and
`mfm.evm.state.execute-transaction@1`.

The generic caller context is carried through unchanged and never enters the Effect command,
EffectId, signing preimage, or authority rows. This follows the existing EVM context-preserving
State pattern and lets a caller retain earlier transaction results without adding arbitrary workflow
metadata to the transaction command. The identity injection policy emits no support State.
`prepare` validates and clones only the nested command. Evidence contains exactly:

- EffectId;
- reserved nonce;
- EVM transaction hash;
- canonical receipt block number and block hash;
- and a closed terminal result: `SuccessCreate { created_address }`, `SuccessCall`, or `Reverted`.

It excludes raw bytes, command ref, logs, gas observations, moving head, confirmation count,
endpoints, and provider responses. The capability compares the evidence EffectId with the pending
EffectId and requires the terminal result variant to match the command action. A second command ref
would be redundant because EffectId already commits to it; authority retains and compares that ref
independently. The live receipt adapter—not the deterministic domain—proves a created address equals
the sender/nonce CREATE address. `interpret` consumes the input, returns the unchanged caller context
plus typed confirmation for either success variant, and returns that context plus typed revert data
through the failure branch. It does not echo the potentially 131 KiB command into the outcome: the
prepared frame already retains and binds it, and a workflow that needs authored metadata can carry
only that data in `K`.
The confirmation projects exactly transaction hash, block anchor, and
`Created { created_address } | Called`; the revert projects exactly transaction hash and block
anchor. Neither projection repeats EffectId, nonce, command, or raw bytes.

### Anchored contract call Read

Add one context-preserving Read rather than fixture-specific read States:

```text
Capability: EvmAnchoredContractCallRead
State:      ReadAnchoredContractCall<K: MfmValue>
Input:      AnchoredContractCallContext<K> { caller_context, intent }
Intent:     existing EvmReadIntent {
              operation, chain_id,
              subject: EvmReadSubject::AnchoredContractCall { target, calldata, anchor },
              route_ref,
            }
Evidence:   existing EvmReadEvidence
Output:     AnchoredContractCallCompletion<K> {
              caller_context,
              result: AnchoredContractCallResult,
            }
Failure:    AnchoredContractCallFailure<K> {
              caller_context,
              reason: AnchoredContractCallFailureReason,
            }
```

Preparation validates and clones the nested intent. Its subject contains target address,
`0..=131_072` calldata bytes, and exact block hash/number anchor. Preserve the current outer
`EvmReadIntent` struct; add only the subject variant shown above. Its outer `route_ref` must equal the
canonical `ContentRef` of the exact `EvmTransactionRoute` used as the adapter binding. Register it as
`register_adapter::<EvmAnchoredContractCallRead, EvmTransactionRoute, _>` under that binding ref.
Extend `EvmReadValue` with one
`AnchoredContractCall(AnchoredContractCallResult { anchor, return_bytes })` variant. The adapter
accepts `1..=24_576` deployed code bytes and `0..=131_072` return bytes; code is checked only for
non-emptiness and its bytes are then discarded. Evidence retains only the same exact checked result
needed for binding/interpretation.

The provider behavior is exact:

- wrong local intent/binding/route is `AdapterError::Internal` with zero provider calls;
- observe the named block by number immediately before and after the code/call pair; JSON `null`
  becomes `SafeFailure`, the same number with another hash becomes `IntegrityBlocked`, and only the
  exact authored number/hash permits or completes the observation;
- `eth_getCode` and `eth_call` use the EIP-1898 block-hash object with
  `requireCanonical: true`;
- valid nonempty code plus a valid call result becomes `EvmReadEvidence::Returned`;
- a successful empty-code result becomes `Rejected` without calling the contract;
- every `eth_getCode`/`eth_call` JSON-RPC error object, transport/status/timeout, malformed JSON,
  invalid hex, or an oversized code/return body is
  `AdapterError::Unavailable` and appends nothing.

Do not classify call reverts or missing anchors from provider error codes, data, or message strings.
The explicit block-by-number observations own the only `SafeFailure`/`IntegrityBlocked` distinction;
this fixture needs no terminal call-revert evidence.

The State preserves caller context on every branch, returns the typed call result only for the exact
matching `Returned` variant, and maps the other three evidence variants to a closed
`Rejected | SafeFailure | IntegrityBlocked` failure reason. There is no provider-error string in
evidence or failure. It does not echo the intent into the outcome because the fused Read record
already retains and binds it; callers put any separately needed authored metadata in `K`. The
success output reuses the exact `AnchoredContractCallResult` held by returned evidence.

Its source identities are `mfm.evm.capability.read-anchored-contract-call@1` and
`mfm.evm.state.read-anchored-contract-call@1`.

The fixture decodes its ABI result in a following deterministic Pure State. Existing duplicate-safe
Read rules continue to apply.

## EVM transaction authority

### Port placement and API

Add a small `mfm-evm-transaction-authority` port crate between deterministic EVM contracts and
concrete/live implementations. It owns checked non-Program authority records, redaction-safe errors,
and this exact checked record sum. Every field is private with checked constructors and borrowing
accessors; none of these records is `MfmValue` or serde, and the raw wrapper implements neither
`Debug` nor `Display`.

```text
NonceDomainKey {
    authority_epoch: EvmAuthorityEpoch,
    chain_instance: EvmChainInstance,
    sender: EvmAddress,
}

NonceDomain {
    key: NonceDomainKey,
    signer_identity_ref: ContentRef,
}

Reservation {
    effect_id: EffectId,
    command_ref: ContentRef,
    domain: NonceDomain,
    nonce: u64,
}

PreparedRecord {
    reservation: Reservation,
    transaction_hash: EvmHash,
    raw_transaction: ExactRawTransaction,
}

SettledRecord {
    prepared: PreparedRecord,
    evidence: EvmTransactionSettlement,
}

AuthorityState = Reserved(Reservation)
               | Prepared(PreparedRecord)
               | Settled(SettledRecord)
```

`ExactRawTransaction` also lives in this port crate, owns `1..=132_096` bytes, can be cloned only as
owned custody data, exposes bytes by borrowing accessor, and has no serde or text/debug rendering.
The advisory-lock digest uses only the exact JCS preimage for `NonceDomainKey` frozen below;
`signer_identity_ref` is a compared attribute and must never alter the lock/key. A settled load
deliberately returns the nested prepared and reservation records, giving adapters/tests one read-only
path to the exact predecessor facts and raw bytes without SQL access or duplicated columns.

The object-safe async API is:

```text
authority_epoch() -> &EvmAuthorityEpoch

load(effect_id, expected_command_ref)
    -> None | AuthorityState

reserve_or_compare(effect_id, command_ref, domain, observed_pending_nonce)
    -> Reservation

retain_prepared(effect_id, command_ref, tx_hash, exact_raw_transaction)
    -> PreparedRecord

retain_settlement(effect_id, command_ref, exact_terminal_evidence)
    -> SettledRecord
```

Its only public error alternatives are `Unavailable` and `Internal`. Ambiguous commit,
serialization conflict, connection loss, and an unsettled predecessor are `Unavailable`; corrupt
retained facts or an exact-identity mismatch are `Internal`. There is no authority `Indeterminate`
variant because the caller cannot safely distinguish commit outcomes and must enter through `load`
on its next invocation.

`authority_epoch` is a synchronous identity accessor over the epoch captured during exact concrete
authority construction; it performs no IO. Adapter construction requires the Program binding epoch
to equal this value. Every concrete port operation also verifies the schema marker still contains
that captured epoch before it reads or inserts facts. An old binding or replaced schema therefore
fails before signer/provider IO.

`load` checks the expected command ref and rejects same-ID mismatch. `reserve_or_compare` atomically
creates-or-compares a domain and its first reservation, or appends the next reservation. The other
writes are insert-or-compare. `retain_settlement` canonicalizes and qualifies the typed evidence,
then atomically compares its EffectId, nonce, and transaction hash with the reservation and prepared
row selected by the separately supplied expected command ref before insertion. `load` repeats that
qualification and reconstructs the exact nested sum; the adapter compares its returned domain with
the command before any later signer/provider step. `reserve_or_compare` qualifies the predecessor
settlement before treating its nonce as consumed; the existence of an unqualified settlement row
never releases the next nonce. There is no activation, mutation, deletion, rollback, broadcast, or
background API. Do not build a feature-gated reusable conformance suite for the single concrete
implementation; test the public port with focused fakes where consumed and test PostgreSQL directly.

Authority deliberately has only the command ref, not command bytes or action semantics. Its
settlement qualification proves canonical current-type bytes plus exact EffectId, nonce, transaction
hash, command-ref predecessor, and prepared predecessor. The trusted live adapter owns receipt/result
truth, and Runtime capability binding owns Create/Call result-shape semantics. Do not add an action
column or command copy to make PostgreSQL duplicate those layers.

### Authority epoch and nonce domain

Fresh transaction-schema provisioning generates 32 bytes with OS cryptographic entropy and retains
them as the immutable public authority epoch. Exact connection admission reads it. The Program
binding, Effect command, and logical nonce domain include that epoch.

The nonce domain key is exactly:

```text
(authority_epoch, chain_id, expected_genesis_hash, sender)
```

The exact `PublicSignerIdentity` content ref is an immutable compared domain attribute, not a key
dimension. That identity already commits to signer route, algorithm, and public key; do not repeat
those columns. This prevents the same on-chain account from acquiring a second nonce timeline merely
by changing a signer label or reimporting the same key. Endpoints are also not key dimensions; the
one supported route is verified against its chain instance.

A new schema gets a new epoch, so a retained old Program fails local binding/association before
signer or provider entry. An epoch does not detect restoration of an older snapshot carrying the
same epoch. Such restoration is unsupported: after authority loss or rollback, retire the epoch and
wallet and provision a fresh authority/wallet.

### Append-only facts and PostgreSQL schema

Use the existing PostgreSQL service, pool, connection gate, runtime role, locators, and provisioner.
Do not add another database service or pool. Add schema `mfm_evm_tx`, exact marker table
`mfm_evm_tx.mfm_evm_tx_schema`, marker `mfm.evm-transaction-postgres.v1`, and four ordinary
append-only fact tables. The v1 column/key layout is frozen as follows; constraint names are also
catalog contract, and no additional table, column, index, sequence, trigger, or default is admitted:

```sql
CREATE SCHEMA mfm_evm_tx;

CREATE TABLE mfm_evm_tx.mfm_evm_tx_schema (
    schema_contract TEXT COLLATE "C" NOT NULL,
    authority_epoch BYTEA NOT NULL,
    CONSTRAINT mfm_evm_tx_schema_pkey PRIMARY KEY (schema_contract),
    CONSTRAINT mfm_evm_tx_schema_contract_check
        CHECK (schema_contract = 'mfm.evm-transaction-postgres.v1'),
    CONSTRAINT mfm_evm_tx_schema_epoch_check
        CHECK (octet_length(authority_epoch) = 32),
    CONSTRAINT mfm_evm_tx_schema_epoch_key UNIQUE (authority_epoch)
);

CREATE TABLE mfm_evm_tx.nonce_domains (
    authority_epoch       BYTEA NOT NULL,
    chain_id              NUMERIC(20,0) NOT NULL,
    genesis_hash          BYTEA NOT NULL,
    sender                BYTEA NOT NULL,
    signer_schema_id      TEXT COLLATE "C" NOT NULL,
    signer_content_digest TEXT COLLATE "C" NOT NULL,
    CONSTRAINT nonce_domains_pkey
        PRIMARY KEY (authority_epoch, chain_id, genesis_hash, sender),
    CONSTRAINT nonce_domains_epoch_check
        CHECK (octet_length(authority_epoch) = 32),
    CONSTRAINT nonce_domains_chain_id_check
        CHECK (chain_id BETWEEN 1 AND 18446744073709551615),
    CONSTRAINT nonce_domains_genesis_hash_check
        CHECK (octet_length(genesis_hash) = 32),
    CONSTRAINT nonce_domains_sender_check
        CHECK (octet_length(sender) = 20),
    CONSTRAINT nonce_domains_signer_schema_id_check
        CHECK (octet_length(signer_schema_id) BETWEEN 1 AND 512 AND
               signer_schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:[1-9][0-9]*:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_domains_signer_digest_check
        CHECK (signer_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_domains_epoch_fkey
        FOREIGN KEY (authority_epoch)
        REFERENCES mfm_evm_tx.mfm_evm_tx_schema (authority_epoch)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);

CREATE TABLE mfm_evm_tx.nonce_reservations (
    effect_id              TEXT COLLATE "C" NOT NULL,
    command_schema_id      TEXT COLLATE "C" NOT NULL,
    command_content_digest TEXT COLLATE "C" NOT NULL,
    authority_epoch        BYTEA NOT NULL,
    chain_id               NUMERIC(20,0) NOT NULL,
    genesis_hash           BYTEA NOT NULL,
    sender                 BYTEA NOT NULL,
    reserved_nonce         NUMERIC(20,0) NOT NULL,
    CONSTRAINT nonce_reservations_pkey PRIMARY KEY (effect_id),
    CONSTRAINT nonce_reservations_domain_nonce_key
        UNIQUE (authority_epoch, chain_id, genesis_hash, sender, reserved_nonce),
    CONSTRAINT nonce_reservations_effect_id_check
        CHECK (effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_reservations_command_schema_id_check
        CHECK (octet_length(command_schema_id) BETWEEN 1 AND 512 AND
               command_schema_id ~ '^schema:[a-z0-9][a-z0-9._/-]*:[1-9][0-9]*:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_reservations_command_digest_check
        CHECK (command_content_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'),
    CONSTRAINT nonce_reservations_epoch_check
        CHECK (octet_length(authority_epoch) = 32),
    CONSTRAINT nonce_reservations_chain_id_check
        CHECK (chain_id BETWEEN 1 AND 18446744073709551615),
    CONSTRAINT nonce_reservations_genesis_hash_check
        CHECK (octet_length(genesis_hash) = 32),
    CONSTRAINT nonce_reservations_sender_check
        CHECK (octet_length(sender) = 20),
    CONSTRAINT nonce_reservations_nonce_check
        CHECK (reserved_nonce BETWEEN 0 AND 18446744073709551615),
    CONSTRAINT nonce_reservations_domain_fkey
        FOREIGN KEY (authority_epoch, chain_id, genesis_hash, sender)
        REFERENCES mfm_evm_tx.nonce_domains
            (authority_epoch, chain_id, genesis_hash, sender)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);

CREATE TABLE mfm_evm_tx.prepared_transactions (
    effect_id       TEXT COLLATE "C" NOT NULL,
    transaction_hash BYTEA NOT NULL,
    raw_transaction BYTEA NOT NULL,
    CONSTRAINT prepared_transactions_pkey PRIMARY KEY (effect_id),
    CONSTRAINT prepared_transactions_effect_id_check
        CHECK (effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT prepared_transactions_hash_check
        CHECK (octet_length(transaction_hash) = 32),
    CONSTRAINT prepared_transactions_raw_check
        CHECK (octet_length(raw_transaction) BETWEEN 1 AND 132096),
    CONSTRAINT prepared_transactions_reservation_fkey
        FOREIGN KEY (effect_id) REFERENCES mfm_evm_tx.nonce_reservations (effect_id)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);

CREATE TABLE mfm_evm_tx.transaction_settlements (
    effect_id         TEXT COLLATE "C" NOT NULL,
    settlement_bytes BYTEA NOT NULL,
    CONSTRAINT transaction_settlements_pkey PRIMARY KEY (effect_id),
    CONSTRAINT transaction_settlements_effect_id_check
        CHECK (effect_id ~ '^effect:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT transaction_settlements_bytes_check
        CHECK (octet_length(settlement_bytes) BETWEEN 1 AND 65536),
    CONSTRAINT transaction_settlements_prepared_fkey
        FOREIGN KEY (effect_id) REFERENCES mfm_evm_tx.prepared_transactions (effect_id)
        ON UPDATE NO ACTION ON DELETE NO ACTION
);
```

The provisioner generates the epoch before its fresh-install transaction and inserts the one marker
row with a parameterized statement in that same transaction; the static DDL never generates entropy
or interpolates bytes. `nonce_domains` pins the split `PublicSignerIdentity` ContentRef;
`nonce_reservations` pins the split command ContentRef. On every ingress and load, Rust reconstructs
and exactly parses those refs in addition to the SQL checks. Addresses, epochs, and hashes use raw
fixed-length bytes; u64s use canonical decimal text at the SQL boundary; raw transactions use exact
wire bytes. `settlement_bytes` is the exact JCS canonical `EvmTransactionSettlement`; load qualifies
it against that current value contract, requires byte-for-byte recanonicalization equality, and
derives its ContentRef rather than storing a redundant digest.

The domain row and first
reservation are inserted in one transaction; there is no independently visible activation phase.
No relation has update/delete/truncate authority and there are no timestamps, mutable status fields,
broadcast commands, or duplicated canonical command bytes.

Use PostgreSQL `NUMERIC(20,0)` for every Rust `u64` chain ID or nonce, with exact
`1..=18446744073709551615` chain-ID and `0..=18446744073709551615` nonce constraints. Bind canonical
decimal text through an explicit numeric cast, fetch as text, and validate in Rust; PostgreSQL
`BIGINT` is insufficient. Use fixed-length `bytea`
checks for epoch/address/hashes, a `1..=132_096` byte check for raw transactions, a `1..=65_536`
byte check for canonical settlement evidence, the shown EffectId/content-ref checks,
primary/unique keys, and foreign keys that permit only the forward fact sequence. The only indexes
are those PostgreSQL creates for the five primary keys and two shown unique constraints; the
domain/nonce unique index also serves predecessor lookup.

Runtime role privileges are exact `SELECT` on marker/domain/fact tables and `INSERT` on the four
fact tables; it receives no DDL, sequence, update, delete, or truncate authority. All authority
writes set `synchronous_commit=on`.

Provisioning remains fresh-or-exact. It installs only when all managed public/config/transaction
surfaces are absent; accepts only the complete exact current design; and rejects mixed, partial,
legacy run-history v1, wrong owner/ACL, replica, or weak durability state. Update all existing
managed database reset fixtures to drop `mfm_evm_tx`, `mfm_config`, and `public`, then recreate
`public`.

### Reservation algorithm

The concrete reservation transaction hashes the JCS canonical bytes of exactly:

```json
{
  "authority_epoch": "<43-character base64url-no-pad encoding of the exact 32 bytes>",
  "chain_id": 1,
  "domain": "mfm.evm.nonce-domain-lock.v1",
  "expected_genesis_hash": "0x<64 lowercase hex>",
  "sender": "0x<40 lowercase hex>"
}
```

`chain_id` is the checked u64 serialized as a JSON integer; epoch/hash/address use their exact
transparent canonical value wires. Freeze an independent known-answer vector for the preimage,
SHA-256 digest, and lock integer. Pass the digest's first eight bytes, interpreted as signed
big-endian two's-complement, to `pg_advisory_xact_lock(bigint)`. A hash collision only over-serializes
independent domains; unique constraints remain the safety boundary. The transaction then applies:

- the adapter's initial `load` returns an existing exact same-Effect reservation without provider
  lookup;
- after an absent-load race, `reserve_or_compare` rechecks EffectId under the domain lock and returns
  a concurrently inserted exact reservation while ignoring the caller's already observed pending
  nonce;
- a new domain and its first reservation at the provider's observed pending nonce are inserted
  atomically;
- after a settled reservation, authority-next is the prior nonce plus one;
- a new Effect reserves only when provider pending equals authority-next;
- lower or higher pending creates no fact and returns `Unavailable`;
- another Effect remains `Unavailable` while the previous reservation is unsettled;
- nonce overflow is a permanent checked Internal failure; and
- unique EffectId and `(domain, nonce)` constraints are the final concurrency boundary.

Pending nonce is an unauthenticated moving observation. A disagreement is neither a settlement nor
authenticated exact-conflict evidence, so this cut does not persist `IntegrityBlocked` or a domain
block. It remains fenced and caller-driven resume may reobserve it later.

On any authority COMMIT acknowledgement error, return `Unavailable` immediately and perform no
later signer/provider step in that invocation. Do not reconnect/reload in the same port call. The
next caller-driven resume begins with `load`, which observes committed or absent state and converges
by exact comparison.

## Live EVM execution

### Provider boundary and exact codec

Keep the existing Read-only `EvmProvider` contract intact for current fakes. Add a separate
`EvmTransactionProvider` trait containing only the transaction operations required here.
`JsonRpcEvmProvider` implements both traits over its one checked locator/client.

```text
chain_instance()
    -> ObservedChainInstance { chain_id, genesis_hash }

pending_nonce(sender)
    -> u64

receipt(transaction_hash)
    -> None | ProviderReceipt {
         transaction_hash,
         sender,
         result: SuccessCreate { contract_address }
               | SuccessCall { target }
               | RevertedCreate
               | RevertedCall { target },
         block_anchor,
       }

canonical_block(block_number)
    -> EvmBlockAnchor

submit_raw(exact_raw_transaction)
    -> transaction_hash
```

Every method returns a borrowing boxed future with `AdapterError`; parameters/results are checked
private-field types, not JSON values or strings. JSON-RPC error objects, unexpected null or missing
required data, out-of-range quantities, malformed or oversized bodies, and transport failures map
to `Unavailable` unless a more specific reviewed Read evidence mapping above applies. A receipt
result of JSON `null` is the one expected null and maps to `None`. A returned submit hash is compared
with the locally derived hash but is never settlement authority. A mismatched submit hash, receipt
field/shape, CREATE address, or canonical receipt anchor is `Unavailable` with no authority write;
these are external inconsistencies, not local contract failures or durable block evidence. A
chain/genesis mismatch with the fixed route binding is `Internal`; it writes no authority fact and
prevents signing or submission as applicable. Local command/binding/retained-authority mismatches
remain `Internal`.

Pin direct in workspace/live EVM dependencies:

```toml
alloy-rlp = { version = "=0.3.16", default-features = false, features = ["std"] }
```

That version is already locked. Encode the exact unsigned and signed EIP-1559 field lists directly
with `alloy-rlp`; use `alloy_primitives::keccak256` for the signing digest and transaction hash. Do
not add `alloy-consensus` or a direct `secp256k1` dependency for signing.

Keep the root rule that only `mfm-evm-live` consumes `alloy-*`. The live crate exposes and uses
narrow pure checked helpers for public-key-to-Ethereum-address derivation and bounded public Keccak.
The managed app test reuses those helpers for address/topic/raw-hash assertions and therefore does
not add an Alloy dependency or duplicate RLP parsing.

```rust,ignore
pub fn ethereum_address(key: &UncompressedSec1PublicKey) -> EvmAddress;
pub fn evm_keccak256(public_bytes: &[u8]) -> Result<EvmHash, EvmCodecError>;
```

`EvmCodecError` is one redaction-safe `Invalid` alternative. These helpers perform no IO or logging;
the Keccak helper accepts `0..=132_096` public bytes, borrows them, and allocates no input copy.
Adapter codec failure maps to `AdapterError::Internal`.

```text
signing_digest = keccak256(0x02 || rlp([
  chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas,
  gas_limit, to_or_empty, value, input, []
]))

raw_transaction = 0x02 || rlp([
  chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas,
  gas_limit, to_or_empty, value, input, [], y_parity, r, s
])

transaction_hash = keccak256(raw_transaction)
```

Use Ethereum's minimal big-endian integer RLP and empty-byte encoding for zero; reject leading-zero
integers, nonempty access lists, noncanonical RLP, and trailing bytes.

Decode every retained raw transaction and compare type, chain ID, nonce, fee fields, gas, target,
value, empty access list, input, signature, recovered sender, and transaction hash before provider
entry. A prepared-byte/hash/command mismatch is Internal and remains fenced.

Provider request/response bodies retain the existing 512 KiB bound, fixed request deadline,
no-proxy/no-redirect/no-auto-retry client, exact JSON-RPC IDs, bounded error mapping, and redacted
diagnostics. The raw-transaction bound leaves room for its lowercase hex JSON encoding inside that
request ceiling. Never parse provider-specific error-message strings.

### Adapter algorithm

One adapter invocation follows exactly:

1. Validate command, binding, authority epoch, route, quantities, byte bounds, and fee relationship
   locally; compare the captured key-bound signer identity and derive/compare the wallet sender from
   its public key. Any mismatch returns Internal before even authority `load` or provider IO.
2. Call authority `load(EffectId, command_ref)` before signer or provider IO.
3. For every returned `Reserved | Prepared | Settled` state, compare its nested reservation domain
   (key plus signer ref) byte-for-byte with the command-derived expected domain before any fast path,
   signer call, or provider call. Mismatch is Internal.
4. If settled, return retained byte-identical evidence with zero signer/provider calls.
5. If prepared, validate exact raw bytes/hash/command and continue at receipt reconciliation.
6. If reserved, deterministically sign that fixed nonce, reconstruct/recover/compare the transaction,
   and call `retain_prepared`.
7. If absent, query and compare provider chain ID plus genesis hash, observe the sender's `pending`
   transaction count, and call `reserve_or_compare`.
8. After a new reservation, sign and retain exact raw bytes as in step 6.
9. After any authority write returning `Unavailable`, stop immediately.
10. For prepared bytes, reverify chain ID/genesis, then query the exact expected receipt hash first.
11. If the receipt is null, submit the retained raw bytes at most once and return `Unavailable`
    regardless of the submission response. Do not poll or query again in this invocation.
12. If receipt lookup or submission has timeout, disconnect, malformed response, or any JSON-RPC
    error, append/write nothing further and return `Unavailable`. “Already known” and “nonce too low”
    are examples handled through that undifferentiated error path, never strings to parse.
13. If a receipt is present, validate expected hash, status 0/1, block number/hash, sender/target or
    creation shape, derive and compare the CREATE address as the last 20 bytes of
    `keccak256(rlp([sender, nonce]))`, and validate canonical block identity; reobserve the
    receipt/block anchor before settling.
14. Persist the stable success/revert evidence with `retain_settlement` before returning it.

There is no `eth_getTransactionByHash`, post-send receipt query, error-string classification,
internal retry, sleep, or loop. Repeated caller invocations may submit the same bytes once each until
a receipt is observed; they can never authorize different bytes.

### Development-only settlement policy

Version 1 of `EvmTransactionEffect` intrinsically means canonical-receipt settlement on the pinned
managed, non-reorging Reth development fixture. Settlement validates the receipt's block number/hash
against the current canonical block and reobserves the same anchor immediately before the authority
insert. It then settles without a configurable confirmation count. Do not add a one-value policy
field or a second policy identifier to the binding.

This adapter is not registered by production `ComposedRuntime`, CLI, REST, or configuration. An
irreversible production settlement/finality policy is a separate architecture cut with a new
capability contract ID and any required value-schema versions. The exact capability contract ref in
the Program prevents development evidence from being confused with a future production policy.

## Managed end-to-end proof

### Solidity fixture

Add one small first-party source under the app integration-test fixture directory:

```solidity
pragma solidity 0.8.33;

contract MfmEffectFixture {
    address public owner;
    uint256 public value;

    event Configured(uint256 value);

    constructor(uint256 initialValue) {
        owner = msg.sender;
        value = initialValue;
    }

    function configure(uint256 nextValue) external {
        require(msg.sender == owner);
        value = nextValue;
        emit Configured(nextValue);
    }
}
```

Compile during the test with exact standard JSON:

- source key `MfmEffectFixture.sol`;
- optimizer enabled with `runs: 200`;
- `evmVersion: "cancun"`;
- `viaIR: false`;
- `metadata.bytecodeHash: "none"`;
- `metadata.appendCBOR: false`; and
- output selection limited to ABI, creation bytecode object, and deployed bytecode object.

Bound standard-JSON stdout to `8_388_608` bytes and reject compiler errors or malformed output. Do
not commit output artifacts. The test derives constructor/configure/value calldata and checks it
against the emitted ABI selectors.

### Test-owned topology and setup

The ignored `mfm-app` integration test owns its fixture-specific Operation, Pure States, values,
failure routing, and report. They are not exported by `mfm-evm` or added to Application component
inventory. The test consumes the real generic Effect protocol, library EVM transaction
State/capability, anchored-call Read, live provider, signer, PostgreSQL authority, Journal, Store,
and Runtime.

Add test-only `mfm-app` dev-dependencies on `mfm-evm-transaction-authority`, `mfm-keystore`,
`mfm-signing`, `mfm-program-derive`, `mfm-values`, workspace `zeroize`, workspace
`reqwest`. Do not add them to the app's production dependency or composition surface unless already
required there by a public library API; in particular, retain the workspace rule that Alloy is live
EVM-only.

The integration test owns one bounded `RethDevObserver` external-boundary helper over a separately
constructed no-proxy/no-redirect/no-retry reqwest client. It exposes only fixture setup/inspection
operations: list the unlocked dev account, fund the generated address, observe base fee, await the
funding receipt, scan at most 256 canonical blocks and 1,024 total transactions/logs, and query
pending nonce. It uses the same 512 KiB per-response ceiling, checked domain values, fixed request
deadline, redacted errors, and no reusable production API. This helper never submits the wallet's
MFM-signed transactions; those go only through `JsonRpcEvmProvider`.

Before Program authoring, setup calls the production `EvmTransactionProvider::chain_instance()` once
and uses that checked `(chain_id, genesis_hash)` with the endpoint ref to construct the exact
`EvmTransactionRoute`. The transaction adapter rechecks it during execution. Do not duplicate
chain/genesis decoding in `RethDevObserver`.

Generate at most four zeroizing 32-byte OS-entropy candidates and import the first valid secp256k1
scalar through the typed keystore; fail setup if all four are invalid. Derive its address and fund it
with `1000000000000000000` wei from Reth's unlocked development account. Never print the scalar or
raw signed bytes. Freeze the fixture values and transaction quantities:

- constructor value `7` and configured value `42`;
- transaction value `0` wei for both transactions;
- deployment gas limit `2000000` and configuration gas limit `200000`;
- max-priority-fee-per-gas `1000000000` wei; and
- max-fee-per-gas `10000000000` wei.

The test first verifies the pinned Reth base fee is not greater than the fixed maximum fee. A
different value fails the fixture; no estimator or ambient fee choice enters the command.

### Faults and recovery

Use one narrow authority decorator: it forwards the first deployment `reserve_or_compare` to the
real PostgreSQL implementation, waits for the real commit, then returns `Unavailable` once. It
proves recovery after nonce-reservation acknowledgement loss. Its consumed-once flag lives in one
test-harness `Arc` outside every reconstructed backend/decorator handle, so cold reconstruction does
not re-arm the fault.

No provider decorator belongs in the e2e. The production adapter deliberately treats a successful
raw-submission response as nonterminal, returns `Unavailable`, and settles only from the receipt on a
later cold resume. The test independently observes that Reth accepted that submission, which proves
the accepted-but-not-yet-authoritative boundary without adding an inert wrapper. Keep one focused
loopback live-provider test in `mfm-evm-live` where a server reads the complete bounded request and
drops the connection before responding; require `AdapterError::Unavailable`. Do not add a proxy
service to the managed e2e.

After each forced interruption, drop and rebuild Runtime, assembly, provider, and backend handles,
reuse the same key-bound signer owner, and resume with the caller-supplied RunId. Drive explicit
caller resumes for at most eight total start/resume progression invocations, failing the test if the
run is not terminal by then; there is no test-only scheduler or retry hidden in Runtime. Perform an
extra final resume/read to prove no third mutation.

### Independent assertions

Do not prove success solely from MFM's retained evidence. Independently scan canonical Reth blocks
and require:

- exactly two outgoing transactions from the imported wallet;
- nonces exactly 0 and 1, with no duplicate or third nonce;
- `keccak256(authority raw) == authority hash == Reth transaction hash`, while Reth's independently
  decoded type-2 RPC fields equal the authored command, reserved nonce, and sender;
- deployment created the expected deterministic contract address and code;
- configuration targeted that address with the exact calldata/value;
- exactly the expected `Configured` event/value exists on chain;
- anchored `value()` returns the configured value at the configuration receipt block hash;
- the final MFM report contains the two transaction hashes, contract address, anchor, and value;
- cold reads return byte-identical evidence/report; and
- Journal contains one prepare/conclusion pair per transaction Effect, with no raw transaction.

From the two Journal prepares, obtain the exact EffectIds and command refs and use only the public
authority `load` API to require two `Settled` records with nonces 0/1, exact hashes/evidence, and the
one expected domain/epoch. Exact table counts, catalog layout, and absence of mutable/broadcast
relations belong to the PostgreSQL authority tests, not an app-only SQL inspection backdoor.

### Nixfied task

The locked nixpkgs supplies `solc 0.8.33` on all declared systems. Add:

```nix
pinnedSolc = assert pkgs.solc.version == "0.8.33"; pkgs.solc;
```

Include `pinnedSolc` only in `effect-e2e` task tools, not global `cargoTools`. The leaf task:

- requires `postgres` and `reth`;
- sets `tools = [ "pg-psql" pkgs.coreutils pinnedSolc ];`;
- nests the existing `localPostgresRun (localEvmRun ...)` wrappers;
- drops `mfm_evm_tx`, `mfm_config`, and `public`, then recreates `public`;
- exports the existing bounded admin/runtime/Reth test locators;
- runs serially:

```sh
cargo test -p mfm-app --test evm_contract_effect_e2e -- \
  --include-ignored --test-threads=1
```

Place `effect-e2e` after `test-db` and `client-e2e` in the `ci` sequence. Update
`docs/build-and-verification.md` with the new managed task contract.

## Documentation cut

Documentation ships with the owning commit, not as follow-up work:

- `docs/design.md`: Program v3, Effect lifecycle, retained-history authority, Journal v2, authority
  epoch/nonce/raw/settlement contract, and development-only settlement boundary;
- `docs/architecture.md`: updated ownership table, dependency direction, progression diagram,
  Pure-only injection, authority port, and no production transaction composition;
- `docs/run-execution.md`: prepared Runnable state, caller-driven `Indeterminate`, retry/cancellation
  behavior, and the fact that read performs no adapter IO;
- `docs/persisted-public-surfaces.md`: exact current Program/Journal/PostgreSQL identifiers and
  explicit rejection of their superseded baselines;
- `docs/evm-rpc-routing.md`: chain-instance versus endpoint identity, separate transaction provider,
  exact RPC sequence, anchored calls, and fixed managed-Reth policy;
- `docs/evm-portfolio-contract-freeze.md`: replace its current-status claim that transaction Effect
  support and this fixture do not exist while preserving the portfolio boundary;
- `docs/known-gaps.md`: remove the generic Effect/outbox gap and retain production finality,
  persistent key recovery, and authority rollback as explicit non-goals/future cuts;
- `docs/build-and-verification.md`: `effect-e2e` task selection and contract; and
- affected IDs, Values, Capabilities, Program, Journal, Runtime, Signing, Keystore, EVM, live EVM,
  PostgreSQL, and app READMEs/rustdoc examples.

Update affected Cargo package descriptions that currently say Capabilities are Read-only, Program
is v2, or Journal has only three record families; package metadata is part of the cutover contract.

Update CLI provisioning wording only where it describes the exact managed PostgreSQL schemas. Do
not add lifecycle inventory, configuration, CLI, or REST transaction documentation because those
surfaces do not exist in this cut. Do not create a second normative EVM protocol document that can
drift from `docs/design.md` and the owning crate READMEs.

## Focused test ownership

### IDs, Values, and derive

- exact EffectId round trips and malformed prefix/algorithm/case/length rejection;
- `StringGrammar::EffectId` wire and validator vectors;
- proc-macro identity shape for fields and containers; and
- stable independent derivation vectors in Runtime.

### Program

- required v3 domain and exact re-encoding;
- v2/missing/wrong domain and unknown-field rejection;
- explicit Pure/Read/Effect sum and exact contract refs;
- Effect authoring before/after Pure injection, identity injection, and deterministic expansion;
- compile-fail proof that `InjectionWriter` cannot emit Effect/Read/Operation/Match;
- exact failure routing and association metadata; and
- global frame weight at, below, and above 65,536, including mutually exclusive branches.

### Journal

- exact frame-v2 canonical known-answer bytes/head digests;
- prepared/concluded object closure and bounds;
- valid pending suffix;
- orphan conclusion, prepare/prepare, prepare/Pure, prepare/Read, and extra conclusion rejection;
- duplicate/unsorted/missing objects and malformed EffectId rejection; and
- complete v1 rejection.

### Runtime

- no adapter before known-inserted prepare;
- preparation error and command/EffectId mismatch before adapter;
- pending hot/cold equivalence and Runnable rendering;
- retained historical Effect command/ID validation;
- evidence swap and outcome-contract mismatch rejection;
- prepared and conclusion `Inserted`/`NotInserted`/`Indeterminate` matrices;
- `Indeterminate` returns without an internal loop and converges on caller resume;
- adapter unavailable/internal/panic/cancellation leaves exactly one pending prepare;
- same-Effect concurrent progress converges on one conclusion;
- no Effect/Read adapter calls from `Runtime::read`; and
- regressions preserving the existing retained Pure/Read authority contract.

### Signing and keystore

- scalar zero/overflow rejection and known public-key/address vectors;
- deterministic repeated signature bytes, low-S, recovery IDs, and recovered key;
- key-instance content-ref derivation and duplicate import;
- wrong digest/purpose/result-shape rejection at consumers;
- capacity/backpressure, last-sender exit, explicit async shutdown/join, and panic/error redaction;
- compile-fail `Keystore: !Send + !Sync`; and
- no secret-bearing Debug/Display/serde surface.

### EVM domain

- every address/hash/U256/byte bound and serde rejection path;
- fixed type-2/empty-access-list command shape;
- fee relationship, create/call, binding, chain, epoch, and signer checks;
- identity prepare and success/revert interpretation;
- evidence binding to EffectId and exact create/call result shape; and
- anchored-call calldata/code/return exact bounds and intent/evidence qualification.

### PostgreSQL authority

- fresh epoch stability and changed epoch after fresh recreation;
- old binding rejection before signer/provider after recreation;
- exact schema/owner/index/constraint/ACL/durability/primary verification;
- missing/partial/extra/legacy installations rejected;
- atomic domain plus first reservation at the observed pending nonce;
- same-Effect insert-or-compare and different-command rejection;
- different-Effect concurrency, one-unsettled fence, and unique `(domain, nonce)`;
- signer label/key mismatch cannot fork a domain;
- same chain ID/different genesis separation and endpoint-independent domain identity;
- lower/equal/higher provider pending behavior;
- exact prepared raw/hash convergence and settled evidence byte stability;
- settlement EffectId/expected-command/nonce/hash mismatch, malformed bytes, or noncanonical bytes
  cannot release a nonce;
- success/revert nonce advancement and overflow;
- injected ambiguous commit at reservation/prepared/settlement, with no later phase in the same call;
- `NUMERIC(20,0)` zero/u64-max/out-of-range handling; and
- no database lock/transaction across simulated signer/provider waits.

### Live EVM

- exact EIP-1559 unsigned/signed known-answer vectors and decode/compare;
- deterministic signer integration, low-S parity, sender recovery, and wrong-key rejection;
- CREATE address derivation and receipt comparison from exact sender/nonce;
- local binding/epoch/route/signer/sender mismatch causes zero authority or provider calls;
- a fake loaded state with the wrong nested domain is Internal before settled return, signing, or
  provider IO;
- settled/reserved/prepared/absent phase entry with exact call counts;
- chain/genesis checks before reservation and prepared submission;
- receipt-first behavior and exact raw bytes on every repeated send;
- null receipt, timeout, disconnect, malformed/error response, success, and revert;
- no provider error-string classification and no transaction lookup;
- canonical receipt reobservation and stable evidence as chain head advances;
- anchored EIP-1898 code/call/reobservation; and
- focused real HTTP connection-drop and response-bound tests.

## Ordered logical commits

Every commit must build, pass its selected focused tests, carry its own docs, and leave one coherent
current design. Subjects are intentionally lower case.

### 1. `add durable effect execution protocol`

- EffectId plus Values/derive support and Runtime derivation vectors;
- shared preparation/adapter errors;
- Effect capability and State contracts;
- Program v3 discriminator/sum/authoring/Pure-only injection/global weight;
- Journal frame v2 prepare/conclusion/structural qualification;
- Runtime association, pending fold, execution, ambiguity, concurrency, cancellation;
- PostgreSQL run-history v2 fresh baseline and affected fixture resets;
- complete workspace consumer/golden-vector cutover; and
- authoritative kernel/runtime/storage docs.

This persisted-wire/API cut is inseparable. Do not split it into commits where Runtime writes a wire
that Journal/PostgreSQL or existing consumers cannot read.

### 2. `add thread-affine evm signing`

- checked signing types and key-instance identity;
- exact pinned `k256` dependency;
- typed secp256k1 import and key-bound signer handle;
- owner thread/channel/shutdown/zeroization;
- delete opaque overwrite/list/remove and free-form signing APIs; and
- crypto, redaction, capacity, and compile-fail tests/docs.

### 3. `add evm transaction contracts`

- shared checked EVM address/hash/quantity/byte types;
- chain instance, wallet, binding, fixed EIP-1559 command;
- settlement/output/revert values;
- one transaction Effect capability/State with identity injection; and
- generic anchored contract-call Read contracts and domain tests/docs.

### 4. `add append-only evm transaction authority`

- new authority port crate and checked internal records;
- PostgreSQL authority epoch, domain/reservation/prepared/settlement schema;
- exact admission/provisioning/ACL/durability integration on the existing backend/pool;
- existing database/Nix fixture resets for the third schema; and
- concrete ambiguity, concurrency, authority-recreation/old-epoch, and bounds tests/docs.

The authority follows EVM contracts because its requests and retained evidence depend on those
types.

### 5. `submit evm transactions durably`

- separate transaction provider facet on `JsonRpcEvmProvider`;
- exact pinned `alloy-rlp` and type-2 codec;
- signer/authority/provider orchestration and receipt settlement;
- generic anchored-call live adapter;
- no production `ComposedRuntime` registration; and
- fake/loopback/cancellation/phase tests and EVM routing docs.

### 6. `exercise effect recovery against reth`

- exact Solidity source and standard-JSON compiler input;
- fixture-local lifecycle Operation/Pure States/report;
- generated/funded ephemeral wallet;
- real authority fault decorator, submit-and-yield recovery, and cold reconstruction;
- independent chain/database/history assertions;
- pinned-solc `effect-e2e` Nixfied task and CI composition; and
- final build/verification and known-gap documentation.

## Verification workflow

Select exact focused filters while implementing, then expand at each changed public/persistence
boundary. All Cargo/Rust commands run inside the default Nix shell.

After commit 1's kernel/persistence cut, run the affected kernel suites, PostgreSQL library tests,
and workspace check. After commit 2, run signing/keystore all targets. After commit 3, run EVM domain
all targets. After commit 4, run authority/PostgreSQL focused and managed ignored tests. After commit
5, run EVM live/domain/authority suites. Representative commands are:

```sh
nix develop -c cargo test \
  -p mfm-ids -p mfm-values -p mfm-program-derive -p mfm-capabilities --all-targets
nix develop -c cargo test -p mfm-program -p mfm-journal -p mfm-runtime --all-targets
nix develop -c cargo check --workspace --all-targets
nix develop -c cargo test -p mfm-signing -p mfm-keystore --all-targets
nix develop -c cargo test -p mfm-evm --all-targets
nix develop -c cargo test -p mfm-evm-transaction-authority --all-targets
nix develop -c cargo test -p mfm-storage-postgres --lib
nix develop -c cargo test -p mfm-evm-live --all-targets
nix run .#run -- --task postgres-test
```

After the first `nixfied.nix` edit, run `nix run .#model-check` early. The plan changes no flake
output, so `nix flake check --no-build` is not selected unless implementation also changes one.

On the exact final candidate, run the managed Effect test once directly while iterating if needed:

```sh
nix run .#run -- --task effect-e2e
```

Then run one final composed gate:

```sh
nix run .#ci
```

Do not redundantly run `.#check`, `.#test`, or `.#test-db` immediately before CI. Report exact
commands/results and anything not run.

## Completion checklist

Implementation is complete only when:

- the six commits are ordered, coherent, and lower-case;
- no Program v2, Journal v1, run-history v1, dual transaction Effect, injected Effect, or old
  signing API remains;
- Store has no new semantic dependency or method;
- Runtime is still the only Program-aware fold;
- exact prepared bytes always precede provider submission;
- authority has no update/delete/broadcast path and no lock spans external IO;
- same-Effect concurrency and every ambiguous commit converge by exact retained facts;
- old authority epoch and wrong chain/genesis/signer fail before mutation IO;
- the fixed settlement policy is absent from production composition;
- the ignored Reth test independently proves exactly two wallet transactions with nonces 0/1;
- no secret/raw transaction reaches logs, public values, Journal, RunView, or failures;
- authoritative design/architecture/build docs match the implementation; and
- final CI passes on the exact candidate.

## Material uncertainties

none
