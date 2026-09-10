# Known limitations

- Adapter error chains are not yet preserved end to end. RPC, SQLx, signer, custody and startup
  conversions discard source layers before Runtime can audit them. The new repository rule
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

## Configuration-driven workflows

The intended consumer supplies workflow/input and execution configuration to production code,
then inspects a typed report. The full problem and acceptance criteria are recorded in
[production responsibilities leaking into the E2E](../PROBLEM_LEAK_PROD_IMPLS.md).
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
  Exact-contract-checked access and bounded lossless failure reporting must replace consumer-side
  JSON decoding and context reconstruction.
- **Deferred general output references:** configuration such as “use output Y of State Z as ABI
  argument X” is not generally supported. Existing `CallCreatedAt` and `ObserveAt` recipes provide
  specific checked dependencies. General references need exact type/field checks, dependency
  order, branch-result availability, and deterministic identity. Start with static configured
  arguments and supported built-in connections; do not imply a dynamic untyped lookup facility.

## Development-node funding

- There is no reusable `DevNodeFundWallet` State/adapter. The E2E currently implements unlocked
  account discovery, submission, and its own funding RPC path. The target is a small deterministic
  Effect State with an explicitly selected dev-node adapter using shared bounded transport, not a
  prerequisite general faucet-authority subsystem.
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
- Consumer E2E coverage should configure production execution and assert final report results.
  Deliberate fault schedules can be selected in separate explicit dev/test scenario configuration,
  backed by reusable testing infrastructure. Forced interruption, teardown/reconstruction, exact
  heads, nonce/retained-wire checks, and append-boundary assertions belong in dedicated
  unit/integration tests. This separation must preserve existing guarantees and managed task
  coverage, not delete them or embed fault injection in ordinary operation semantics.
