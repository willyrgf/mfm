# Architecture

Dependencies point inward from composition and adapters to typed domain/kernel contracts.

| Owner | Responsibility | Must not own |
| --- | --- | --- |
| IDs / Values | checked identities including `EffectId`, `ConfigName` and `DigestBytes`, schema descriptors, canonical typed values, mechanical typed context slots, 32 MiB object bound | execution or IO |
| Capabilities | Read intent/evidence and Effect command/evidence contracts | State outcomes or retries |
| Program | typed Operation authoring and root input validation, sole private lowering draft, checked v8 ordered State sequence and exact initial-value commitment, Pure/Read/Effect State contracts, redaction-safe internal callback errors, `Never` | registries, IO, scheduling |
| Runtime | immutable assembly, Program association, complete current continuation and operation facts, local safety checks and typed progression | physical storage or Journal envelope wire |
| Journal | exact opaque canonical frame encoding and decoding | Program/lifecycle semantics or persistence IO |
| Store / run index ports | object-safe admission/latest/optional-probe load and atomic append; separate mechanical current-head enumeration | Program, State, capability, reducer, config semantics, or run-status derivation |
| Config repository port | immutable named revisions, exact import/load/delete, and complete listing | config-wire parsing, Program semantics, merging, defaults, or revocation |
| Signing | checked transient secp256k1 key/digest/signature contracts, public recovery, key- and purpose-bound signer port | persisted identity, secret custody, EVM encoding, provider IO |
| Keystore | bounded thread-affine secp256k1 custody and key- and purpose-bound signer handles | Program, Runtime, EVM, persistence, free-form signing |
| Domains | reusable deterministic Portfolio/EVM semantics and public value contracts | Runtime, Store, provider handles |
| Live adapters | bounded provider ingress, EVM wire codecs, stage-specific signer/custody/provider IO, direct typed callback registration, and pure transaction State-family registration | domain planning or finality policy configuration |
| Application | injected and live composition; typed config/run/discovery use cases; exhaustive entry-point planning and compiled component inventory; shared client RunId generation and JSON models | sockets, argv/HTTP, sessions, frame inspection, status derivation, secret administration |
| Binaries | bounded transport parsing, one Application call, transport policy, and redacted rendering | composition, domain planning, environment resolution, execution lifecycle, or run semantics |

`RuntimeAssemblyBuilder::new` fallibly installs the framework codec, while infallible `finish`
freezes the accumulated valid registrations. One private capability sum correlates each distinct
public Read or Effect protocol with its exact codecs and binding callbacks. Multiple exact schemas
may share one semantic type identity; exact content refs select codecs, and the exact
implementation/input/output/failure ABI selects a State registration. Neither lookup falls back to
semantic identity. Program association converts each registered mode into one mode-specific
executable carrying only its valid functions, codecs, validators, and exact callback. Read
callbacks and hot/cold binding receive the qualified intent value ref; Effect callbacks receive the
qualified command value ref. Current state retains checked declaration identity and canonical Objects; typed native values are
materialized at selected callbacks and are not cached in the continuation. Association marks
checkpoint boundaries once in the private executable State; entering a State does not rescan the
Program. Canonical byte wrappers and Object content identities share immutable storage. RunView
shares the admitted Object, so transitions do not copy its complete input or content identity.

Application's private compiled State table couples each State's domain-owned inspection metadata to
the same monomorphized Runtime registration function used by live composition. The component
inventory adds the domain-owned entry-point definition and explicitly admitted public reusable
Operations without expanding a Program. Runtime owns no descriptions or Operation registry.

The progression sequence is:

```text
caller -> Application -> Runtime current transition -> Journal opaque frame -> Store append
                         -> Pure State or Read adapter
                         -> original failure append -> recovery callback -> recovery append
                         -> Effect prepare append -> Effect adapter
                         -> settlement append -> deterministic interpretation
Store admission/latest/probe snapshot -> Runtime local validation -> RunView
```

Journal's privately constructed `EncodedRunFrame` proves exact canonical envelope bytes and hash,
not a lifecycle or complete-history qualification. Runtime owns one `RunRecord` containing its current operation, checkpoints, usage and Effect
barrier. A private borrowed selector derives continuation for dispatch, validation and public views;
there is no stored phase, second current input or phase/facts agreement validator. Values Object owns the exact value ref and canonical bytes. Store owns the
head and bounded selected rows in `LoadedRun`; it does not decode Program or reconstruct state.
Runtime validates selected rows and current facts without folding earlier frames. Checked Object Deserialize and derived Runtime decoding own stored-payload rejection, with complete
input consumption. Stored nested constructor rejection reports parser category/location/reason,
without structured nested ancestry or size facts. Direct construction and postdecode slot admission
retain their concrete fields. Ordinary Serde structural forms are accepted; no parallel decoder
grammar or re-encoding comparison exists. Every physical
append still enforces exact predecessor, sequence and all-or-nothing immutable insertion.

Store's three physical failure dispositions carry Values-owned diagnostic data. PostgreSQL owns
one private SQLx extraction recipe at selected run producers; MemoryStore owns its local check,
allocation and task facts. Runtime and Application forward the existing concrete Store error;
no new reporting owner, source registry or mutation authority is introduced.

Concrete storage backends may implement both `Store` and the separate `RunIndex`, but Runtime
receives only `dyn Store`. Config custody and run enumeration therefore cannot widen Runtime's
append-only storage authority. Config listing returns all retained revisions as one unpaginated
aggregate; each document remains bounded, but the collection has no count limit. Run enumeration
uses ascending `RunId` keyset pages and makes no cross-request snapshot claim.

PostgreSQL owns its raw private locator grammar, target equivalence, SQLx wiring, split-role
provisioner, loopback-only plaintext policy, and ambient-input exclusion. `PostgresBackend` owns one
pool gated only for Store, RunIndex, and config custody. Optional
`PostgresEvmTransactionAuthority` owns a separate pool gated only for its port. The authority schema
owns only its epoch marker and append-only reservation and exact prepared-wire
facts; the reservation contains the complete nonce domain. It does not own commands,
transaction action semantics, provider
truth, signing, broadcast, Runtime progression, or Program association. PostgreSQL uses stock SQLx
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

Portfolio owns candidate selection and its checked enrichment output, pairing each collection
with its route and anchor and consuming it into the snapshot configuration/selector. Application alone publishes
that output through configuration custody and verifies RunId/head/output linkage before a new
dependent admission. Runtime exposes already-qualified genesis input and entry-point identity for
matching start recovery; it acquires no configuration lookup or write authority.

The CLI owns argv, bounded file/stdin input, exit status, schema provisioning outside listener-held
Application state, and the deployment-free compiled-component inspection rendering. REST owns
liveness, bounded HTTP admission, and an unauthenticated Unix socket; it exposes neither schema,
developer inspection, nor secret administration. Both accept an optional identity and use the same
Application client primitive to generate one before their one use-case call. REST returns HTTP 200
for a durably failed run, while the CLI uses exit 1 for Runnable, EffectPending, AwaitingRecovery,
AwaitingInterpretation, or Failed. These are named transport
asymmetries, not second use-case implementations.

Program authoring and wire decoding share one private State data representation. Whole-sequence
validation constructs the public immutable declarations; those declarations have no unchecked
deserializer. Serialization borrows the validated data.

Values owns qualification against exact inline/generic value descriptors. Program checks adjacent
State contracts and selected policy/mapping ABIs; Runtime associates the exact typed implementations.

The source-authoring sequence is separate from progression:

```text
domain Operation -> OperationExpansion -> one private symbolic draft -> immutable Program v8
                         |-> typed before/designated/after capability injection
```

Operation implementations compose children through `OperationExpansion`. One selected handler binding inherits through nested scopes; occurrence overrides replace its
parameters and targets together. Error classification belongs to the exact typed error contract. Root failure maps
replace failure-routing States. Scoped checkpoints lower to permitted declaration boundaries.
Runtime receives only the completed Program and associated implementations; Journal and Store
receive neither authoring scopes nor policies to execute. Runtime owns continuation transitions
and local validation, including recovery usage, checkpoint retention and irreversible Effect barriers.

EVM owns its original incident components and their intrinsic error classifications. Portfolio owns its
original public failure and explicit EVM failure map. The shipping Portfolio operation uses the
framework's stop defaults and zero recovery allowances. A caller selects any different policy;
collection authoring does not override it implicitly.

Pure work and byte-heavy validation run in immediately awaited pure blocking jobs. Effect identity
derivation belongs to Runtime and binds RunId, Program ref, execution position (State and visit), and exact command
ref. Connections, transactions, Store mutation, and provider IO remain async and outside those
jobs. Dropping an operation is safe: no candidate exists yet, or the one in-flight append commits
atomically and a later selected-row snapshot observes its outcome without claiming that the interrupted caller
received an acknowledgement.

EVM owns one shared checked address, hash, and U256 vocabulary across existing balance Reads and
transaction contracts. Capability injection expands the generic
`ExecuteEvmTransaction<C, R>` into reservation and preparation Effects, the designated execution
Effect, and a Pure outcome projection. Domain-owned `mfm_evm::custody` defines atomic reservation
and exact-byte retention; PostgreSQL implements that port mechanically. Runtime retains accepted settlement in the Journal payload before interpretation. Runtime schedules the expanded sequence with no EVM or signer knowledge. EVM owns checked plans, cumulative transaction/observation facts, and slot-selected creation,
creation-dependent call, ordinary call, and anchored observation recipes. Products select context
field names and recipe connections, and own ABI decoding and terminal report/failure policy.
`EvmTransaction<C, R>` is the reusable domain Operation; live EVM owns its pure four-State
registration helper, separately from IO adapter registration. Outer authoring and registration
bounds name the existing executable contracts; slot reconstruction equalities belong to their
State implementations. Read subject-family classification belongs to EVM and is reused by live
preflight, which captures immutable binding references at registration.
`EvmAnchoredContractCallRead` owns only the exact Program-visible
intent/evidence and context-preserving State. Its route reference is the content ref of
`EvmTransactionRoute`, while the transaction Effect binds the complete
route/authority-epoch/sender value. Live registration and IO remain downstream adapters. Live EVM
receives checked Read intents with Runtime's exact intent value ref and returns evidence bound to
that same ref; it owns no duplicate serialized-intent transport. Live EVM alone owns the checked command-to-consensus mapping, retained-wire and signer validation, the
separate transaction provider facet, stage-specific custody access, and the anchored-call RPC
sequence. Its narrow pinned Alloy dependency owns EIP-1559/EIP-2718 consensus encoding, decoding,
hashing, and CREATE-address derivation. Transaction settlement is intentionally limited
to the pinned non-reorging development fixture; `ComposedRuntime` registers neither transaction
Effects nor anchored transaction-route Reads.

Values owns `DiagnosticEvidence`, a nested persisted-field JSON contract, and invocation-only
`InvocationDiagnostic` data. Neither has a standalone executable value identity, source walker,
classification role or separate quota. EVM owns the provider kind/source carrier and ObservedSize;
live EVM and CLI output own the two explicit source-extraction recipes. Live EVM also owns the
development funding calls through the same bounded RPC decoder.

The domain graph is one-way: Portfolio depends on EVM domain contracts; EVM depends on foundations
and Program; live EVM depends on EVM plus Runtime and captures concrete
signer handles. Neither domain depends on Runtime, Store, live IO, keystore custody, or Application.

## Material uncertainties

none
