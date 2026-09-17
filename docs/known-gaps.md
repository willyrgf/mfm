# Known limitations

- Adapter error chains are not yet preserved end to end. SQLx, signer, custody and application startup
  conversions still discard source layers. EVM transport capture now retains reviewed causes, but
  internal State failures and recovery-before-commit still require the RFC lifecycle cutover. The new repository rule
  requires preservation; [the adapter error audit](adapter-error-audit.md) records concrete gaps,
  secret-free retention constraints and coherent remediation scope. Durable typed failure records
  must not be described as retaining raw client errors already discarded upstream.
- PostgreSQL claims primary crash/restart durability only. It does not claim safe writable rollback,
  host-loss failover, quorum, replica, or multi-primary authority.
- Trusted Rust State implementations, assembly, and adapters are in the process trust base.
- Runtime has caller-driven progression only; it owns no background scheduler or timeout policy.
- Current product entry points are `mfm.portfolio/snapshot@1` and bounded candidate enrichment
  `mfm.portfolio/enrich@1`. Enrichment does not discover assets outside the supplied candidate list.
- EVM transaction settlement version 1 is limited to the pinned non-reorging development fixture.
  Production submission remains unsupported until a product defines its finality, confirmation,
  reorg, authorization, and operational policy under a separately reviewed capability identity.
- The in-process keystore has no encrypted persistent custody or key recovery across host-process
  termination. The bounded managed cold-recovery loop rebuilds Runtime and every IO handle between
  caller invocations while retaining the same ephemeral signer owner.
- The append-only EVM transaction authority has no writable rollback, snapshot restoration, nonce
  release/reuse, replacement, or fee-bump contract. Loss of acknowledged authority requires a new
  authority epoch and fresh runs rather than reconstruction of its old writable timeline.
- The standalone CLI exposes no token or contract deployment, configuration effects, transaction
  submission, or keystore administration.
- Sequential fresh RunIds can grow retained history without a product quota.
- Effect progression is caller-driven and permits duplicate adapter entry for the same exact
  `EffectId` and command. Each mutating adapter must supply its own convergent durable authority.

## Public framework interfaces

MFM supports three public activities: selecting an existing operation, composing Operations/States,
and implementing a new State. Their caller responsibilities and proposed improvements are recorded
in the [public interfaces and tests RFC](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md).
Current authoring requires separately maintained executable registrations, and ordinary EVM callers
still supply execution/result-handling glue. Improving configured products alone does not establish
usable framework composition or extension. Custom Operations, States, and intentional product maps
are legitimate in tests of those public authoring contracts.

### Downstream component discovery in the DSL refactor

The [DSL refactor RFC](../RFC_REFACTOR_DSL.md) accepts one temporary limitation: authors introducing
new executable semantics must publish the new source once in their integration's installed public
component set. Stored implementation identities cannot instantiate Rust code absent from that set.
Automatic discovery of arbitrary downstream State implementations is deferred; this is a target
design limitation, not a claim that the new construction API is implemented.

Consumers selecting or recomposing installed components need no registration-list changes. The
framework must derive each published source's State, codec, handler and injected-support requirements
instead of requiring separate executable lists. The original Operation/composition type must not be
required at each cold load to compensate for missing discovery infrastructure.

During implementation, place a comment explaining this constraint at the source-publication site
and link it here. Revisit automatic discovery later without weakening exact implementation matching,
inward dependencies or explicit failure when required code is unavailable.

## Configuration-driven workflows

A configured EVM product is one use of the framework, alongside direct Rust authoring. Its consumers
should supply supported workflow/input and execution options and inspect checked results. The RFC
separates this product capability from improvements to the three public usage paths.
The following are missing capabilities, not current API guarantees:

- A production composition/execution entry point for supported EVM workflows that owns dependency
  lifecycle, exact State/adapter assembly, and use of the existing Runtime. The consumer E2E still
  implements assembly and progression around production primitives; it does not contain a second
  Runtime engine. The current Portfolio composition does not provide this EVM product surface.
- Bounded configuration of supported operation order, ABI/function selection, and static typed
  arguments, with production-owned validation, encoding/decoding, sequence construction, and
  executable registration. ABI choices and values should be configuration rather than custom
  test State implementations. The initial supported ABI/type catalogue remains to be specified.
- RPC fee-discovery Read States and production fee-selection logic. Observation evidence must be
  retained before deterministic transaction construction, with configured bounds/rules. A resumed
  acknowledged prepare must reuse its original command and fees rather than refresh them.
- Simple configuration of supported terminal failure classes and production-owned classification,
  propagation, and report construction. Consumers should not write typed maps merely
  to select terminal behavior. Configuring a class does not make an infrastructure error into an
  authenticated durable domain failure.
- Production typed success/failure reports containing configured names/order, declared public
  inputs and execution facts, available outputs, and supported consistency/expectation results.
  Result delivery should reuse existing exact-contract decoding and retain declared facts within
  report bounds, replacing repeated consumer-side JSON decoding and context reconstruction.
- **Deferred general output references:** configuration such as “use output Y of State Z as ABI
  argument X” is not generally supported. Existing `CallCreatedAt` and `ObserveAt` recipes provide
  specific checked dependencies. General references need exact type/field checks, dependency
  order, branch-result availability, and deterministic identity. Start with static configured
  arguments and supported built-in connections; do not imply a dynamic untyped lookup facility.

## Development-node funding

- There is no reusable `DevNodeFundWallet` State. The E2E uses
  `JsonRpcEvmProvider::fund_development_sender` for unlocked account discovery and one submission
  through the shared bounded causal RPC path. A recoverable funding Effect still needs an explicit
  convergent protocol; the provider helper alone does not establish that contract.
- The funding completion/readiness and duplicate-entry contract remains undefined. The current
  helper's returned submission hash does not itself establish funded readiness, and blindly
  repeating `eth_sendTransaction` after lost acknowledgement can fund twice. Select and test the
  smallest convergent dev-node protocol before treating funding as a recoverable Effect.

## General execution and recovery policy

Runtime now supplies intrinsic error classification and common handler selection, bounded retry/checkpoint restart, Effect
barriers, canonical failure reports, and stopped-invocation observations. The remaining gaps below
concern configurable product orchestration around that caller-driven contract.

- There is no complete reusable production driver for configured deadlines, polling, retries,
  exact-admission recovery, dependency reconnection, and structured result delivery. Consumers
  should configure this policy rather than implement `drive_to_success` or resume-before-start
  loops. Operational policy must not rewrite admitted Program/input or retained pending commands.
- Product configuration for Absent, admission conflict, dependency reconnection, ambiguous
  acknowledgement, cancellation, and caller deadlines remains undefined. Any orchestration must
  preserve the existing distinction between invocation failure and durable terminal failure,
  including retained pending Effect authority.
- Recovery after client/process loss, pending transaction reconciliation, key/authority availability,
  and interactions with the existing nonce/replacement/finality limitations are not a complete
  configurable product. Existing cancellation, acknowledgement-loss, prepared-wire, and cold-fold
  tests establish specific boundaries; they do not establish a complete configurable orchestration product. Ephemeral
  keystore survival during Runtime reconstruction is not host-process restart recovery.
- The [three public-usage E2Es](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md#three-public-usage-e2es)
  are the agreed replacement target: selection through both CLI/REST, composition of existing
  Operations/States, and new-State implementation plus composition. Each can grow named recovery,
  durability, and error cases. Framework cases may author Operations/States and drive Runtime
  directly; fault cases may assert intermediate authority/history. Focused boundary tests remain
  valid. Map existing guarantees and managed selection before deleting old coverage; planned future
  cases are not replacements for exercised guarantees. No new scenario DSL is required.
