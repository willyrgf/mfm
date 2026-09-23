# Architecture

Dependencies point inward from composition and adapters to typed domain/kernel contracts.

| Owner | Responsibility | Must not own |
| --- | --- | --- |
| IDs / Values | checked identities including `EffectId`, `ConfigName` and `DigestBytes`, schema descriptors, canonical typed values, shared checked unsigned-256 arithmetic, 32 MiB object bound | execution or IO |
| Capabilities | Read intent/evidence and Effect command/evidence contracts, typed adapter outcomes | State outcomes or retries |
| Program | typed Operation authoring and root input validation, sole private lowering draft, complete immutable v9 ordered State sequence and exact initial-value commitment, Pure/Read/Effect State contracts, typed State/adapter invocation and original classification callbacks, redaction-safe internal callback errors, `Never` | registries, ambient IO, scheduling |
| Runtime | complete current continuation and operation facts, local safety checks and typed progression | physical storage or Journal envelope wire |
| Journal | exact opaque canonical frame encoding and decoding | Program/lifecycle semantics or persistence IO |
| Store / run index ports | object-safe admission/latest/optional-probe load and atomic append; separate mechanical current-head enumeration | Program, State, capability, reducer, config semantics, or run-status derivation |
| Config repository port | immutable named revisions, exact import/load/delete, and complete listing | config-wire parsing, Program semantics, merging, defaults, or revocation |
| Signing | checked transient secp256k1 key/digest/signature contracts, public recovery, key- and purpose-bound signer port | persisted identity, secret custody, EVM encoding, provider IO |
| Keystore | bounded thread-affine secp256k1 custody and key- and purpose-bound signer handles | Program, Runtime, EVM, persistence, free-form signing |
| Domains | reusable deterministic Chain/Portfolio/EVM semantics and public value contracts | Runtime, Store, provider handles |
| Live adapters | bounded provider ingress, EVM wire codecs, stage-specific signer/custody/provider IO, checked native resources, exact typed binding, and native client config/output conversion | domain planning or finality policy configuration |
| Application | injected and live composition; typed config/run/discovery use cases; exhaustive entry-point planning and compiled component inventory; shared client RunId generation and JSON models | sockets, argv/HTTP, sessions, frame inspection, status derivation, secret administration |
| Binaries | bounded transport parsing, one Application call, transport policy, and redacted rendering | composition, domain planning, environment resolution, execution lifecycle, or run semantics |

Program construction owns executable association, exact typed value contracts, bound native
adapters, handlers and checkpoint markers. Runtime retains that complete immutable Program directly;
it has no assembly builder, registration table or second executable representation. Cold read/resume
receives a constructed Program and compares its exact identity with the admitted document before
validating the current continuation. Program's `admit` checks retained slots against their selected
contracts; native evidence and operational originals use the selected native ABI. The complete
Program shares immutable storage across invocations. Typed values materialize at selected callbacks
and are not cached in Runtime's continuation. RunView shares the admitted Object.

Program's private Inventory owns installed value, State, native and handler claims and the single
executable association path shared by compilation and loading. Fresh Draft borrows those claims
for selected-owner checks and retains only declaration ordering, public bindings, defaults,
checkpoints and depth. It cannot create callbacks or bind resources. Association checks all
available typed handler parameters and native bindings before resource attachment; bound handlers
capture decoded parameters. Full ABI and in-process TypeId checks preserve ownership without
persisting Rust identities. New groupings of installed components need no extra source publication.

Program derives component inspection from the same structural source/type discovery used by cold
association. State and Operation definitions own metadata. Application adds its entry points and
forwards that inventory without a handwritten component table, Plan invocation or live handles.
Runtime owns no descriptions or Operation registry.

Program's kernel callback boundary owns typed State evaluation, preparation, evidence binding,
interpretation, original classification and bound adapter invocation, including native decoding,
value/original encoding and panic containment. Runtime's nongeneric State runners call these
functions without passing DriverContext or transition authority across that boundary. Each outcome
wrapper captures its exact original contract; Runtime supplies the current execution position. Its invocation-only Decode/Execute/Encode failure carries
immutable diagnostic data; Runtime attaches the actual operation without recapturing it.
Read has one completion callback: project/bind native evidence once, pass the typed evidence directly
to State interpretation, then encode the outcome. Its internal Bind/Interpret result preserves the
operation alongside the existing codec phase and cause. The native Object remains the exact retained
evidence. Effect keeps separate binding and interpretation callbacks across settlement acknowledgement
and cold recovery; Read has no such intervening durable boundary.
Callback encoding and decoding use immediately awaited pure blocking jobs. Runtime admission
encoding is the explicitly approved synchronous exception for borrowed caller input. Explicit adapter IO stays
async, and Runtime alone retains append, acknowledgement, classification timing and recovery
authority. Capabilities owns the single Pending/Settled Effect adapter outcome type.

Runtime also owns the read-only `program_document` bootstrap because it owns the admission-record
wire. Bootstrap and restore share the private RunRecord decoder and admission qualification; no
Application parser or second wire representation is introduced. It returns the retained Object
without requiring installed executable code, and makes no checked-current-continuation claim.
Program owns canonical validation and the eventual typed cold reconstruction from that document.

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

Execution authority preserves unavailable diagnostic data or one final internal invocation value.
PostgreSQL alone extracts SQLx and selected retained-fact causes; Live moves the result into the
existing operational owner or invariant route. EVM owns the v4 classifiable transaction-error
payloads and has no SQLx, signing or keystore dependency. AuthorityError has no persistence schema
or serializer. Executing keystore replies carry SigningError directly; the signer port supplies
SignFailed evidence without a KeystoreError roundtrip. Live moves it into SignerUnavailable.
Thread-affine custody and channel authority are unchanged. No new recovery, capture or reporting
layer is introduced.

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

`EvmResources<Sources>` owns checked native resources and derives exact bindings and public views
through existing Program environment contracts. It has no duplicate cache or registration loop.
The same concrete backend supplies Store, RunIndex and configuration custody. Application compiles
or cold-loads a complete Program and coordinates Runtime and repositories.

CLI and REST render one typed Application use-case surface and install no user authentication or
authorization layer. Binaries own bounded transport parsing/rendering and transport policy only. A
shared Application request contains only bounded, secret-free data and stable selectors for
pre-bound capabilities. It cannot introduce environment resolution, a filesystem or network
locator, secret custody, schema authority, or an unbounded durable effect.

Portfolio owns candidate selection and its checked enrichment output, pairing each collection
with semantic route/point envelopes and checked product metadata. Native clients retain and decode
public native descriptors, render product output and construct publication configuration without
source configuration or live handles. Application alone publishes
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
State contracts, selected recovery ABIs and exact native associations before returning Program.

The source-authoring sequence is separate from progression:

```text
typed source + borrowed local Plan + explicit resources
    -> one private lowering draft -> complete immutable Program v9 -> Runtime
       native typed prefix/designated/suffix and exact resource binding
```

Maintained Operation defaults inherit through typed scopes; handler replacement changes parameters
and target markers together. One nesting guard runs before planning/injection. Error classification
belongs to the exact original error contract. Checkpoint markers lower to qualified declaration
boundaries. Runtime receives only the completed Program; Journal and Store receive no authoring
scopes or executable policies. Runtime owns continuation transitions, recovery usage, checkpoint
retention and irreversible Effect barriers.

EVM owns native incident classification and exact retained native decoding. Owning native State
definitions derive decoder dispatch. Portfolio validates collection/context agreement and constructs
its public failure projection. Shared semantic failures own their closed codes. Application invokes
these pure client functions with retained contracts; it has no native interpreter or presentation
registry. The exact original/report remains separate from product presentation. Snapshot and
enrichment use framework Stop with zero allowances unless an enclosing caller selects another policy.

Pure work and byte-heavy validation run in immediately awaited pure blocking jobs. Effect identity
derivation belongs to Runtime and binds RunId, Program ref, execution position (State and visit), and exact command
ref. Connections, transactions, Store mutation, and provider IO remain async and outside those
jobs. Dropping an operation is safe: no candidate exists yet, or the one in-flight append commits
atomically and a later selected-row snapshot observes its outcome without claiming that the interrupted caller
received an acknowledgement.

EVM owns one shared checked address, hash, and U256 vocabulary across existing balance Reads and
transaction contracts. The request-typed supporting sequence is ReserveEvmNonce<R> then
PrepareEvmTransaction<R>, surrounding the shared Deploy/Configure State with an identity suffix.
ReservedRequest<R> retains its exact request and ReservedEvmTransaction. Deterministic EVM recipes
own request-to-command and successful settlement-to-domain projection. Program owns exact native
implementation identity; preparation uses that helper rather than an EVM-local hashing recipe.
The designated native implementation validates binding, native command and request correspondence,
then projects settlement while retaining the original Object. Shared States own semantic rejection
and successful context construction. The old slot-based execute/project family is removed.
Domain-owned `mfm_evm::custody` still defines reservation and exact-byte retention; PostgreSQL owns
its mechanical storage. [Build and verification](build-and-verification.md) defines focused and
managed acceptance; unit or scripted execution does not imply acceptance against live custody and
chain services.
Chain owns the shared balance source/request/scale and semantic evidence contracts. Request admission
preserves order and checks unique public IDs, nonempty bounded source count and common ledger.
Balance Read binding checks intent, native-original provenance and successful observation point;
native protocol, account/asset and route qualification remain with EVM. The current shared boundary
supports both maintained Portfolio continuations. Chain's CandidateBalance retains only prepared
facts and raw units. The native
confirmation State owns anchor comparison before calling shared append. Append's outer invocation
error and inner arithmetic failure preserve their separate ownership; no classification is attached
to local context-construction errors. Confirmed entries carry no copied Read evidence history.
Chain also owns private bounded decimal arithmetic and exact collection arithmetic failures.
Values owns reusable checked size-violation facts. Arithmetic helpers do not confer native
confirmation or collection-completion authority.
Chain's ConsolidateBalanceCollection requires the entire confirmed prefix and owns aggregate
arithmetic. Its checked completion nests the original context and total, retaining the typed caller
without duplicating request, metadata or confirmations. Caller resumption owns checked product projection. Portfolio stores semantic metadata, confirmed
balances, execution descriptors and totals; native clients own native JSON admission and output
conversion. Shared constructors own source/request validation and Portfolio owns cross-collection
agreement. Public route descriptors retain the endpoint name needed by configuration-deleted
publication. Maintained snapshot/enrichment Operations derive vectors from checked input; Application
calls compilation with explicit native resources. Their distinct checked continuations share private
collection validation. The evidence ledger records the exact revisions and scope of focused and
managed acceptance.
Native balance support uses EvmBalanceLedger's chain ID, EvmBalanceTarget's account/asset and a
per-source binding. The qualified native intent carries ordinal, scale, chain, target and public route
once, with a stage-only subject. Supporting identity translation checks those retained execution
facts against the selected binding before IO. Balance qualification preserves the existing chain-ID
protocol; the stronger lifecycle ledger's genesis identity is not fabricated for balance collection.
EVM supporting States retain shared context through checked chain and token-anchor stages. Native
preparation uses the requested collection scale; token preparation observes decimals at the initial
anchor. Confirmation re-observes the committed block number and compares its hash before shared amount
admission. EVM no longer owns a parallel collection context, completion representation or decimal
arithmetic. Native and token balance implementations inject their fixed supporting sequences around
the shared ObserveBalance and project native evidence while retaining the original Object. Explicit
resolved sources compile into a ten-State native/token collection Program, including shared
consolidation, and cold-load without configuration.
The designated balance intent retains the independently expected caller route, checked by native
translation against the selected binding before IO, including cold execution.
Live EVM alone owns the checked command-to-consensus mapping, retained-wire and signer validation, the
separate transaction provider facet, stage-specific custody access, and the anchored-call RPC
sequence. Its narrow pinned Alloy dependency owns EIP-1559/EIP-2718 consensus encoding, decoding,
hashing, and CREATE-address derivation. Transaction settlement is intentionally limited
to the pinned non-reorging development fixture. Native transaction resources bind typed request
specializations and anchored contract Reads through the same environment/discovery contracts.

Values owns `DiagnosticEvidence`, a nested persisted-field JSON contract, and invocation-only
`InvocationDiagnostic` data. Neither has a standalone executable value identity, source walker,
classification role or separate quota. EVM owns the provider kind/source carrier and ObservedSize;
live EVM and CLI output own the two explicit source-extraction recipes. Live EVM also owns the
development funding calls through the same bounded RPC decoder.

The shared Chain domain owns network-independent scalar and contract lifecycle State semantics, typed
transaction requests and semantic settlement binding. It depends only on inward Program,
Capabilities, Values and IDs contracts. Checked success-context construction and decoding belong
here; native protocol authentication, preparation and receipt interpretation remain with the native
owner. Chain retains exact native Objects without native caches, network or Runtime dependencies.
Shared lifecycle contexts and flattened reports use the same borrowed semantic consistency checks.
Report consumes validated context and moves each retained fact once. Intent identity reconstruction
connects observations to their retained target and configuration anchor; it does not reinterpret
native evidence or reconstruct prepared transaction commands.

The production domain graph points inward: Portfolio depends on Chain; EVM depends on Chain,
foundations and Program. Live EVM depends on Portfolio, Chain, EVM and Program/Capabilities for
native admission, binding and presentation, and captures concrete signer handles. Runtime is a
Live EVM test dependency, not a production dependency. Application coordinates Runtime and the
native client. No domain depends on Runtime, Store, live IO, keystore custody, or Application.

## Material uncertainties

none
