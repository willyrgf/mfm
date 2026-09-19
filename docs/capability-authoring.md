# Authoring Operations, States and native capabilities

The [design](design.md) owns execution and persistence contracts; the
[architecture](architecture.md) owns placement. This guide describes the implemented authoring
path. The [Phase B ledger](dsl-phase-b.md) records which production acceptance checks have passed
and which remain outstanding.

## Reuse semantics before introducing a contract

Reuse a capability when its intent or command, evidence and binding guarantees express the needed
operation. Different endpoints, providers or native encodings belong to implementations of that
capability. A new externally meaningful operation or evidence guarantee can require a new
capability contract. A different Rust caller context alone does not.

`ContractRead` has one fixed semantic intent, `ReadContractValue`, and outcome-bearing
`ContractValueEvidence`. The caller supplies the expected route and observation point; the native
owner qualifies these against its binding and converts its native observation. Rejection, safe
failure and authenticated integrity blocking retain their distinct evidence meanings. Local route
or codec errors remain invocation errors before IO or append.

`TransactionEffect<R>` specializes its semantic command and evidence by `TransactionRequest`.
`R::Applied` describes the successful request-specific result. Deployment establishes a contract
locator; configuration establishes `ConfigurationApplied`. Both carry a common transaction
identity and inclusion point independently of their applied result. Request specialization avoids
an untyped command envelope or a growing enum of unrelated operations.

Prepared values are necessary when later execution must retain facts established by earlier
States. Supporting native States are necessary when preparation or confirmation requires explicit
IO or acknowledged Effects. Do not compute future State outputs during planning. For example, EVM
transaction injection reserves a nonce and prepares a transaction before the designated Effect;
the signed wire remains under native transaction-authority custody. Balance injection observes
chain/anchor facts and confirms the anchor before a candidate becomes a confirmed balance.

See the current [transaction contracts](../crates/domains/chain/src/transaction/capability.rs),
[scalar Read](../crates/domains/chain/src/transaction/read.rs), and
[balance contracts](../crates/domains/chain/src/balance/read.rs).

## Follow one complete construction path

A consumer selects a maintained Operation or composes typed `Pure`, `Read`, `Effect`, nested
Operations and homogeneous endomorphic vectors. Adjacent expanded endpoints must match. `Plan`
borrows the parent configuration and returns a local configuration plus a finite typed body;
planning neither performs IO nor simulates execution. `Operation::from(definition)` supports
explicit non-Default definitions with maintained defaults. `None` in policy values inherits the
parent choice; an explicit zero replaces an allowance. One nesting guard covers Operations and
native injection and is checked before planning or injection construction.

`ProgramEnvironment::Sources` publishes installed source types. Existing discovery derives their
State codecs, handlers, native ABIs and recursively injected support. `CapabilityFamily` describes
available native implementations; `Resolve` selects one from admitted configuration. Native
`ResolveReadBinding` or `ResolveEffectBinding` qualifies that configuration. `BindRead` and
`BindEffect` bind the selected public facts to explicit handles. A consumer recomposing installed
components does not write registration loops or another binding cache.

`compile` produces a complete immutable Program: a canonical document, exact public bindings and
mandatory executable occurrences. The entire serialized document, including aggregate bindings,
is bounded. Runtime receives this Program explicitly with the borrowed typed input. It performs
its existing admission, continuation, recovery and local safety checks; it does not assemble code.

For cold execution, obtain the retained Program document and `load` it against an environment
containing the exact installed implementations and required resources, then pass the resulting
Program to Runtime `read` or `resume`. Cold loading needs no source configuration and never calls
`Plan`, defaults resolution or injection construction. Public binding objects persist; provider,
signer and Store handles do not enter canonical data. Publication retains the endpoint's public
name as well as its derived identity, so a native client can reconstruct configuration from a
semantic enrichment output without live handles.

The executable examples are [maintained lifecycle composition](../crates/domains/chain/src/transaction/authoring.rs),
[Portfolio planning](../crates/domains/portfolio/src/planning.rs),
[explicit defaults](../crates/kernel/program/tests/recovery_scopes.rs), and
[native Portfolio cold execution](../crates/live/evm/tests/portfolio_runtime.rs).
The accepted [source-publication limitation](known-gaps.md#downstream-component-discovery-in-the-dsl-refactor)
is distinct from recomposing already installed code.

## Trace deployment through native execution

1. `DeploymentRequest` retains the artifact, configuration value and caller-owned execution
   expectations. `Deploy` is the shared designated State.
2. EVM injection expands it with `ReserveEvmNonce<R>` and `PrepareEvmTransaction<R>`. The native
   recipe qualifies the request and creates its nonce-free command; reservation and preparation
   run through the same Runtime Effect machinery as other Effects.
3. `PreparedTransaction<R>` retains the request, selected implementation/binding identities and
   exact public native preparation. Native execution reconciles the retained transaction, checking
   receipt and transaction-known status before any exact-byte rebroadcast. Pending pacing stays
   in the awaited adapter future; cancellation creates no background task or Runtime deadline.
4. Native settlement projection produces `TransactionEvidence<R::Applied>`, retaining the exact
   native original, common transaction identity and inclusion point. `Deploy` interprets that
   evidence into `DeployedContract`.
5. `Configure` consumes that actual deployed result. `Observe` reads at the configuration point;
   `Validate` checks the observed value and `Report` produces the shared lifecycle report.

The native owner is implemented in [recipes](../crates/domains/evm/src/transaction/recipes.rs),
[injection and projection](../crates/domains/evm/src/transaction/native.rs), and
[Live transaction reconciliation](../crates/live/evm/src/transaction.rs). The
[lifecycle Runtime test](../crates/domains/evm/tests/lifecycle_runtime.rs) exercises maintained 42
and composed 84 with scripted custody/provider responses; managed acceptance is separately tracked.

## Keep author responsibilities explicit

A State author defines typed deterministic semantics and its declared domain failures. A Read or
Effect State prepares semantic work and interprets admitted capability evidence. It performs no
ambient IO and does not decode another owner's native configuration.

A native implementation author owns configuration qualification, codec phases, native protocols,
supporting States, evidence validation and semantic projection. Use Capabilities' existing
`CallbackFailure` and codec scopes to retain Decode, Execute and Encode through nested hooks.
Forward actual provider errors unchanged. Preserve each available cause and reviewed diagnostic
field; a local binding failure cannot become authenticated integrity evidence or a fabricated
durable domain incident.

Native clients admit native product configuration and decode exact retained contracts for product
presentation. Portfolio checks collection, continuation, route, correlation and source agreement
and constructs its checked product projection. Native decoder dispatch derives from owning State
declarations. Application coordinates these existing owners and keeps the exact original report
separately; it does not interpret native payloads or install presentation callbacks in Runtime.

Use the [native State declarations](../crates/domains/evm/src/balance/stages.rs),
[client failure projection](../crates/live/evm/src/client/portfolio/failure.rs), and
[independent native Portfolio test](../crates/domains/portfolio/tests/native_boundary.rs) as
consuming examples. Extend boundary tests for new semantics, including exact cold decoding,
local rejection without IO/append, cancellation and original-cause preservation. The independent
native test is a fixture, not support for another shipping network.
