# Architecture

Dependencies point inward from composition and adapters to typed domain/kernel contracts.

| Owner | Responsibility | Must not own |
| --- | --- | --- |
| IDs / Values | checked identities including `EffectId`, schema descriptors, canonical typed values, 8 MiB object bound | execution or IO |
| Capabilities | Read intent/evidence and Effect command/evidence contracts | State outcomes or retries |
| Program | typed Operation authoring, sole private lowering draft, checked v3 State/Match graph, Pure/Read/Effect State contracts, `Never` | registries, IO, scheduling |
| Runtime | immutable assembly, Program association, sole fold, typed execution/progression | persisted wire or physical storage |
| Journal | exact frame encoding and complete-history qualification | domain interpretation or persistence IO |
| Store / run index ports | object-safe complete load and atomic append; separate mechanical current-head enumeration | Program, State, capability, reducer, config semantics, or run-status derivation |
| Config repository port | immutable named revisions, exact import/load/delete, and complete listing | config-wire parsing, Program semantics, merging, defaults, or revocation |
| Signing | checked transient secp256k1 key/digest/signature contracts, public recovery, key- and purpose-bound signer port | persisted identity, secret custody, EVM encoding, provider IO |
| Keystore | bounded thread-affine secp256k1 custody and key- and purpose-bound signer handles | Program, Runtime, EVM, persistence, free-form signing |
| Domains | reusable deterministic Portfolio/EVM semantics and public value contracts | Runtime, Store, provider handles |
| Live adapters | bounded provider ingress, EVM wire codecs, signer/authority/provider orchestration, and direct typed callback registration | domain planning, State registration, or finality policy configuration |
| Application | injected and live composition; typed config/run/discovery use cases; exhaustive entry-point planning and compiled component inventory; shared client RunId generation and JSON models | sockets, argv/HTTP, sessions, frame inspection, status derivation, secret administration |
| Binaries | bounded transport parsing, one Application call, transport policy, and redacted rendering | composition, domain planning, environment resolution, execution lifecycle, or run semantics |

RuntimeAssemblyBuilder registers exact value codecs, closed Pure/Read/Effect State modes, Match
descriptors, and separate Read/Effect callbacks. Multiple exact schemas may share one semantic type
identity; exact content refs select codecs, and the exact implementation/input/output/failure ABI
selects a State registration. Neither lookup falls back to semantic identity. `finish` freezes one
immutable assembly. Program association converts each registered mode into one mode-specific
executable carrying only its valid functions, codecs, validators, and exact callback. Fold state
retains declaration identity and qualified values only; hot execution and cold qualification index
the already-associated executable Program. The fold performs no public registry lookup and exposes
no erased value workflow.

Application's private compiled State table couples each State's domain-owned inspection metadata to
the same monomorphized Runtime registration function used by live composition. The component
inventory adds the domain-owned entry-point definition and explicitly admitted public reusable
Operations without expanding a Program. Runtime owns no descriptions or Operation registry.

The progression sequence is:

```text
caller -> Application -> Runtime -> Journal frame -> Store append
                                  -> Pure State
                                  -> Read adapter -> provider
                                  -> Effect prepare -> Store append -> Effect adapter
                                                     -> Effect conclusion -> Store append
Store load -> Journal qualify -> Runtime fold -> RunView
```

Concrete storage backends may implement both `Store` and the separate `RunIndex`, but Runtime
receives only `dyn Store`. Config custody and run enumeration therefore cannot widen Runtime's
append-only storage authority. Config listing returns all retained revisions as one unpaginated
aggregate; each document remains bounded, but the collection has no count limit. Run enumeration
uses ascending `RunId` keyset pages and makes no cross-request snapshot claim.

PostgreSQL owns its raw private locator grammar, target equivalence, SQLx wiring, split-role
provisioner, loopback-only plaintext policy, and ambient-input exclusion. `PostgresBackend` owns one
pool gated only for Store, RunIndex, and config custody. Optional
`PostgresEvmTransactionAuthority` owns a separate pool gated only for its port. The authority schema owns only append-only nonce-domain, reservation, exact prepared-wire,
and canonical settlement facts. It does not own commands, transaction action semantics, provider
truth, signing, broadcast, Runtime folding, or Program association. PostgreSQL uses stock SQLx
directly.
Administrative database authority exists only in short-lived provisioning paths and is never
retained by Application. Production CLI provisioning installs only the base persistence surfaces.

`ComposedRuntime` is the only live Portfolio assembly constructor. One opaque binding set supplies
typed EVM targets and provider handles in stable order; composition derives both adapter
registrations and public binding views from it. The same concrete backend is coerced to `Store`,
`RunIndex`, and config custody, so production use cases cannot observe different repositories.

CLI and REST render one typed Application use-case surface and install no user authentication or
authorization layer. Binaries own bounded transport parsing/rendering and transport policy only. A
shared Application request contains only bounded, secret-free data and stable selectors for
pre-bound capabilities. It cannot introduce environment resolution, a filesystem or network
locator, secret custody, schema authority, or an unbounded durable effect.

The CLI owns argv, bounded file/stdin input, exit status, schema provisioning outside listener-held
Application state, and the deployment-free compiled-component inspection rendering. REST owns
liveness, bounded HTTP admission, and an unauthenticated Unix socket; it exposes neither schema,
developer inspection, nor secret administration. Both accept an optional identity and use the same
Application client primitive to generate one before their one use-case call. REST returns HTTP 200
for a durably failed run, while the CLI uses exit 1 for Runnable or Failed. These are named transport
asymmetries, not second use-case implementations.

The source-authoring sequence is separate from progression:

```text
domain Operation -> OperationExpansion -> one private flat draft -> immutable Program v3
                         |-> exact capability/State injection policy
```

`match_join` owns both ordinary value-selected topology and recovery selected by a Pure failure
classifier. `with_failure_handler` routes only the named typed State failures; it does not catch
compiler, Runtime, adapter, Store, cancellation, or panic failures. Runtime, Journal, and Store
never receive Operations, authoring setup, callbacks, or scope metadata.

Operation implementations compose children only through `OperationExpansion`, and capability
policies emit support States only through `InjectionWriter`. Direct trait callback calls bypass
kernel callback accounting and are forbidden in reviewed production code. This trusted-code rule
is not a security or authorization boundary; checked Program construction remains the persisted
graph boundary.

Pure work and byte-heavy validation run in immediately awaited pure blocking jobs. Effect identity
derivation belongs to Runtime and binds RunId, Program ref, declaration index, and exact command
ref. Connections, transactions, Store mutation, and provider IO remain async and outside those
jobs. Dropping an operation is safe: no candidate exists yet, or the one in-flight append commits
atomically and the next complete reload resolves it.

EVM owns one shared checked address, hash, and U256 vocabulary across existing balance Reads and
transaction contracts. Independent Create and Call Effect States project action-specific inputs,
successes, and failures through the same deterministic `EvmTransactionEffect`; it owns no nonce
reservation, signing, provider, or settlement loop. EVM also owns the checked creation-to-call
binding/target transition and call-to-observation route/target/anchor transition, but no product
lifecycle Operation. `EvmAnchoredContractCallRead` owns only the exact Program-visible
intent/evidence and context-preserving State. Its route reference is the content ref of
`EvmTransactionRoute`, while the transaction Effect binds the complete
route/authority-epoch/sender value. Live registration and IO remain downstream adapters. Live EVM
alone owns exact type-2 RLP/Keccak, the separate transaction provider facet, append-only authority
orchestration, and the anchored-call RPC sequence. Its version 1 transaction settlement is
intentionally limited to the pinned non-reorging development fixture; `ComposedRuntime` registers
neither transaction Effects nor anchored transaction-route Reads.

The domain graph is one-way: Portfolio depends on EVM domain contracts; EVM depends on foundations
and Program; live EVM depends on EVM plus Runtime and captures concrete
signer handles. Neither domain depends on Runtime, Store, live IO, keystore custody, or Application.

## Material uncertainties

none
