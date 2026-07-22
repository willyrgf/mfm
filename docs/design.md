# MFM Design Contract

Status: authoritative typed-core design contract

This document defines the runtime and authoring contract for the current MFM codebase. It is the
normative contributor-facing design reference.

The central rule is:

```text
typed state programs are the only semantic executable surface;
certified typed execution specs are the only runtime contract;
erased runner plans are implementation artifacts.
```

Operations may plan typed state programs, but operations are not runtime execution units after
certification. Binaries and app crates assemble stores, artifacts, registries, and capabilities;
they do not own workflow semantics.

## Non-Negotiable Invariants

- New execution uses certified typed execution specs only.
- Runtime state transitions are committed as typed kernel events through `mfm-store`.
- The typed run stream for `run:{run_id}` is append-only and authoritative.
- Store commits are atomic: a commit either appends every payload and updates derived projections,
  or appends nothing.
- Event envelopes, run-local sequence numbers, store-wide append coordinates, ordinals, event ids,
  logical keys, and projections are store-owned. `StreamSeq` orders one run; the durable
  `StoreCommitOrder` totally orders committed appends across the store and is the only authority
  for cross-run fact ordering and fact-query commit watermarks.
- Manifest, config, seed, fact, artifact, output, and spec identities are content-addressed.
- Hashed structured data uses canonical JSON and must not contain floats.
- Typed values, typed configs, public outputs, and event payloads must not contain secrets.
- Replay and resume are driven by the stored certified spec and the authoritative run stream.
- Replay adapters must answer only from recorded facts, typed artifacts, and side-effect evidence.
- Process identity and process topology are not semantic authority. Workers are interchangeable
  executors over the store, and leases/claims are liveness coordination only.
- Admission, drive, verify, and replay must not consult mutable registries or external policy
  oracles; outcome-affecting policy is resolved once into hash-defining certified spec material.
- Side effects use one state-authored typed intent plus idempotency input, required typed prepared
  invocation authority, durable ledger events, and typed submission, receipt, confirmation, or
  recovery evidence. State logic is pure; adapters alone prepare and submit external mutations.
- Side-effect verification policy is hash-defining certified config. `RunAdmitted` may record
  launch audit evidence, but it is not independent finality or verification authority.
- Certified saga decisions are derived from the certified spec plus append-only stream facts.
- Manual saga resolution requires certified schema authority and a signed authorization proof.
- Public output JSON is a render surface. Typed terminal cells plus public-output specs and events
  are the authority.
- Source scans, naming conventions, hash-only envelopes, persisted summaries, and CI summary keys
  are not typed-core architectural proof or runtime authority.

## Authority Surfaces

Typed-core code distinguishes data, evidence, authority, and implementation artifacts:

| Surface | Runtime authority? | Contract |
|---|---:|---|
| `mfm_spec::UntrustedTypedSpec` | no | Persisted/user bytes decoded into typed Rust data. Hostile until certification verifies the spec against the registry. |
| `mfm_certify::LoweredTypedSpec` | no | Program-lowered draft data. It is the certifier input for authored programs, not runtime authority. |
| `mfm_certify::ValidatedTypedExecutionSpec` | no, certifier-private | Registry-validated spec authority used inside certification to mint certified objects and certificates. It does not cross into runtime as a mutable execution surface. |
| `mfm_spec::v1::HashedSpecEnvelope` | no | Hash-only envelope for canonical spec bytes and non-semantic audit metadata. It cannot certify a spec. |
| `CertifiedSpecCertificate` bytes/evidence | no | Persisted certificate evidence. Hostile until the certifier verifier checks spec hash, certificate hash, registry digest, descriptor identities/digests, lowering/canonicalizer identity, public-output schema id, and audit metadata. |
| `mfm_certify::CertifiedTypedSpec` | yes | Non-forgeable in-memory authority minted only by registry-backed certification or verified persisted spec/certificate evidence. |
| `CertifiedDescriptorSet` / `CertifiedFrameworkLifecycle` | yes, within certified spec authority | Certified descriptor and framework lifecycle views derived from a validated spec. Runtime consumes these views instead of recertifying raw descriptor tables. |
| `mfm_spec::v1::CertifiedContextSpec` / `ContextRef` | no, spec data until certified | Hash-defining transition-context table entries and content-addressed refs. They become execution authority only through certified spec validation plus node, cell, and input context constraints. |
| `mfm_runtime::CertifiedRuntimeSpec` | yes, runtime-only | Runtime wrapper derived only from `CertifiedTypedSpec`; owns scheduler indexes and erased runner derivation. |
| `PreparedCommit<Purpose>` / `PreparedCommitPlan` | yes, store mutation | Purpose-specific commit authority built by runtime/app authority. The store rejects mismatched payload purpose, missing saga proof, and missing admitted artifact evidence. |
| `CertifiedRunStoreAuthority` | yes, store admission | Policy-bound run-start/certified run authority minted from the certified typed spec and tied to run id, certified spec hash, saga policy, and side-effect terminal policies. Store admission checks the token spec hash against the projected `RunAdmitted.spec_hash`, not only the incoming saga payload. |
| `ManualResolutionProofAuthority` / `VerifiedManualResolutionForPrefix` | yes, manual resolution | Prefix-bound proof authority over a certified manual-blocked stream prefix, retained artifacts, canonical proof bytes, and certified operator policy. |
| `CertifiedSideEffectContract` | yes, side-effect verification | Certified resource-claim and side-effect contract authority shared by live execution, resume, and replay. |
| `SideEffectLedgerState` | yes, store transition | Typed ledger state used by store/runtime to admit only legal side-effect transitions. |
| `SagaTerminalProof` | yes, terminal saga | Store-required proof object for completed, compensated, manually resolved, or failed-without-claim terminal saga outcomes. |
| `CommittedRunStream` / `EventArtifactRequirement` / `VerifiedRunArtifactStore` | yes, stream/history evidence | Store-owned committed stream authority, event-derived retained-artifact requirements, and verified retained-artifact authority tied to that stream. |
| Erased runner plans | no | Runtime implementation artifacts reproducibly derived from certified authority and runner registry. |
| `PublicOutputReadAuthority` | yes, render-only | App authority minted after certified spec/certificate verification and projection rebuild from the authoritative stream. |
| Rendered public-output JSON/artifacts | no | Output/cache material for users and integrations. They cannot authorize resume, replay, or another render. |

Persisted spec bytes, persisted certificate bytes, rendered JSON, and projection rows must be
validated or rebuilt before they influence semantic execution.

## Semantic Package Boundaries

Workspace packages declare semantic ownership through closed `package.metadata.mfm` fields. The
contract is independent of package names, paths, and counts:

| Layer | Responsibility |
|---|---|
| `kernel` | Framework-owned, domain-free contracts and runtime infrastructure. |
| `domain` | Pure domain model, capability, signing, state, and operation responsibilities. |
| `live` | Domain adapter and reusable transport implementations. |
| `signing` | Generic signing contracts. |
| `secret-provider` | Secret-bearing keystore and signer implementations. |
| `storage` | Concrete storage implementations. |
| `assembly` | Runtime configuration, implementation construction, and application services. |
| `binary` | Transport-only CLI and API executables. |
| `test` | Test-only support and cross-boundary fixtures. |

Every pure domain package declares its validated domain and whether that domain is a source or an
aggregate. All packages for a domain agree on that role. A live package declares the same domain
and is invalid without a pure-domain owner. Kernel packages explicitly declare whether domain code
may consume them; kernel and assembly packages explicitly declare whether binaries may consume
them. Cargo target kinds independently require every package with a binary target to use layer
`binary` and keep proc macros in dedicated non-binary packages.

Model, capability, signing, state, operation, adapter, and transport remain architectural roles,
not automatic package boundaries. Within one pure domain, the permitted private-module direction
is `model <- capability <- signing <- state <- operation`. Within one live domain, the public typed
transport and private adapter are siblings over the pure-domain capability contract. A new package
is justified only by a real reuse, dependency, proc-macro, process, storage, or secret-isolation
boundary.

Kernel dependency direction remains strict:

```text
ids -> canonical -> values -> capabilities
  -> program/spec -> certify/events/store -> runtime/replay
```

Kernel packages depend only on kernel packages. Pure domains consume only domain-facing kernel and
generic signing contracts; aggregate domains may also compose source domains. Live packages may
consume kernel, signing, and the pure domains allowed by their source/aggregate role, but never app,
binaries, concrete storage, or secret providers. Secret providers consume only domain-facing
kernel, signing, and their own layer. Concrete storage consumes kernel contracts, never the reverse.
Assembly is above those layers. Binaries consume only explicitly binary-facing kernel and assembly
contracts. Tests are unrestricted. Normal and build dependencies are checked from Cargo metadata;
dev-only edges do not establish production ownership.

States remain free of runtime/store implementations, binaries, live transports, secret providers,
and operation modules. Operations remain deterministic planning. Adapters bind state-owned intent
to runtime capabilities and evidence phases. Transports implement reusable capability backends.
Neither adapter nor transport code mints production authority outside app/runtime assembly.

## Typed Values And Configs

Typed configs are deterministic planning inputs. They are decoded at boundaries, validated,
canonicalized, and retained by value or content-addressed reference in certified specs.
Run-start rejects config artifacts that match certified metadata but cannot be decoded and validated
through the trusted certification registry or an exact framework-owned config reference.

Typed runtime values cross state boundaries through typed cells and handles. A runtime value cannot
change the topology of the same certified run. If produced data must select future topology, the
workflow must create a new planning boundary such as a child run or a separately certified
continuation.

Persisted values and configs must implement the typed descriptor contracts through the framework
derive path or framework-owned generic constructors. Manual descriptor implementations outside the
framework boundary are rejected by checks because they bypass schema, no-float, and no-secret policy.

Secrets remain below the typed semantic boundary. Private keys, mnemonics, passwords, decrypted
bytes, raw signing material, and signed raw transactions must not be typed values, configs, facts,
artifacts, events, public outputs, error details, or fixtures. States refer to secret-bearing systems
through non-secret labels, references, and capabilities.

Persisted and public surfaces are inventoried in `docs/persisted-public-surfaces.md`; that inventory
is the review checklist for applying this no-secret invariant to app, CLI, REST, storage, artifact,
and diagnostic boundaries.

## Fact Identity And Query Terms

Fact identity retains the complete canonical typed subject object. `FactSubjectMaterialV2` wraps
that object under `mfm.fact-subject-material.v2`; it never projects identity down to a configured
list of scalar paths. The v2 subject namespace binds the fact kind and subject schema id. `FactKey`
then binds that namespace hash to the full subject-material hash, so an undeclared nested subject
field, optional tagged-enum payload, account, asset, or contract address cannot disappear from
identity merely because it is not indexed.

Descriptor-declared subject fields validate typed scalar paths and define query-term extraction.
They are searchable projections, not identity declarations. A `FactQueryProjection` owns all
present `fact_query_terms` for its claim; optional terms may be absent, but rebuild, signed query
evidence, replay, and content-identity verification must rehydrate the retained full subject,
validate every declared required term, and derive terms again. No flattened path list,
delimiter-joined asset key, or query-term cache can substitute for canonical subject material.

Canonical fact-query v2 may carry one opaque `FactContentIdentityEvidence` narrowing value. The
compiler binds its descriptor component to the resolved descriptor, and providers compare all four
compact components against trusted query projections before ordering and limiting. This filter is
only a bounded candidate-narrowing mechanism: the consumer must still hydrate canonical subject and
response material and rederive the full content identity before trusting a returned fact.

## Current Semantic Configuration And Launch Boundary

Semantic configuration has a strict pre-admission path and a separate process-local runtime path.

Setup import is the only TOML semantic configuration surface. `mfm-app` decodes a closed
`SetupDocument { configs: Vec<SetupConfig> }`, validates every typed value, derives its stable
target from the intrinsic domain id, canonicalizes it through the typed descriptor path, rejects
prohibited secret-bearing fields, enforces size limits, and atomically upserts the complete batch.
A failed value or duplicate target leaves all current configuration unchanged. Configuration storage
persists opaque canonical bytes plus target/schema/digest; target is the primary key. It has no
history, revision selector, digest lookup, cursor, or knowledge of setup kinds or domain types.

Run start accepts one exact entry-point id and one target. The sole public entry point is
`mfm.portfolio/snapshot@2`, whose target is a `PortfolioId` such as `acme/primary`. The target
contains no collector policy, child config, runtime route, read bound, collect/reuse switch, or
report-only switch. App assembly loads that target's current row, requires the expected schema,
verifies canonical bytes and digest, revalidates semantic config, verifies the embedded
`portfolio_id` equals the selected target, and only then calls the deterministic operation builder.
The completed config, typed graph, certificate, and runtime authority contain concrete values.

`RunAdmitted` records the exact entry-point id and sorted `(target, schema_id, digest)` evidence.
The certified spec, certificate, seeds, retained artifacts, facts, outputs, and append-only run
stream are the authority after admission. Resume, status, stream, public-output, and replay paths
must not query mutable current configuration or reconstruct semantic config from current setup
files. Replay verifies retained evidence and recomputes pure behavior from certified values only.

Runtime TOML is intentionally outside this semantic boundary. It maps non-secret source and signer
references to process-local routing and capability resources, is not semantic configuration data,
and is never persisted in specs, events, artifacts, public outputs, or replay inputs. For each start
or resume, live assembly derives the distinct Bitcoin and EVM route keys required by certified
pending nodes, resolves each selected entry once into a fresh dispatch-local session set, and leaves
unrelated malformed entries unparsed. A later call creates a new dispatch and re-resolves the
current file. Evidence-only reads do not load runtime TOML. Selected EVM routes admit only HTTP(S)
URLs without userinfo and valid HTTP authorization values. Ingress validates route/session bindings
and EVM chain identity before `RunAdmitted`, or before claim acquisition on resume, and preserves
`RuntimeConfigRequired` versus `RuntimeConfigInvalid` with closed semantic diagnostics. There is no
fallback or compatibility surface.

Portfolio selection consumes family completion authority without a generic fan-in value. The
snapshot operation passes typed Bitcoin and EVM receipt vectors into one report operation. That
operation's structured input binding is passed unchanged into `SelectHoldingsState`; the
collector receipt input edges are the readiness barrier. The snapshot operation constructs only child
operations, while the report operation constructs only selection, snapshot assembly, and report
projection. The state validates exact portfolio demand, receipt family/chain/source coverage, and
receipt uniqueness before it authors any query. There is no portfolio manifest identity, generic
receipt entry, count/readiness value, app-only fan-in runner, or separate replay verifier. This
explicit downstream state-to-state contract is why the portfolio state role depends on the
Bitcoin and EVM pure-domain contracts; source domains remain independent of portfolio, adapters,
transports, app assembly, and runtime config.

## Typed Program Authoring

State outputs are represented by branded typed handles. Handles carry the produced Rust value type,
program brand, scope brand, cell identity, schema identity, semantic type identity, and value
lineage. Handles cannot be forged from raw ids.

Scopes are part of certified semantics. Cross-scope same-value movement requires framework bridge
nodes with persisted bridge evidence. Transforming movement is modeled as an ordinary state.

Optional runtime paths use `MaybeValue<T>` cells, not absent context keys. Artifact references use
`ArtifactRef<T>` and must verify digest, schema id, semantic type id, role, and producer evidence
before materialization.

Operations are typed planners. They receive typed config and typed handles, assemble a typed state
program, bind public outputs, and return a draft for certification. Operation lineage and stable
domain keys are recorded in the certified spec so fanout/fanin ordering and semantic identity are
auditable.

## State And Operation Registration

A state type is executable only after framework registration validates:

- state kind and version
- config, input, output, and public descriptor identities
- effect class
- capability set
- hash-defining effect contract for every external read and side effect
- runner kind and executable identity

Planning requires state and operation membership in the builder registry. Runtime requires the
certified descriptor identity and the registered runner identity to match the stored spec.
Parent operation registries compose child state, operation, and certification registries as one
unit, then register only parent-owned types. Repeating a child's concrete inventory in its parent
is forbidden because it creates a second topology authority that can drift independently.

One live process has one `ExecutableIdentityTemplate`. The app computes it once by streaming the
opened current executable through raw SHA-256, then content-addressing exactly the canonical object
`{"contract":"mfm.executable-bytes.v1","sha256":"<64 lower-case hex>"}`. Linux hashes the
opened `/proc/self/exe` inode and verifies stable file metadata; macOS verifies the opened immutable
path and file metadata before and after the read. Blocking file work runs before scheduler work on
a blocking worker, and every failure is the redacted `ExecutableIdentityUnavailable` startup
error. Every runner, framework handler, and adapter factory retains its distinct `factory_id` but
uses that one template `binary_digest`; runtime and domain crates provide no default or
label-derived identity. Live resume under different executable bytes fails binding validation
before an attempt or live capability call. Read-only observation and evidence-only replay never
read or hash the current executable.

## Effects And Capabilities

Effect classes are framework-owned:

- pure: no external capabilities
- read: read/support capabilities only
- apply side effect: exactly one external mutation authority plus allowed support capabilities

The runtime injects only capabilities certified for the current node. State code must not create its
own live network, filesystem, clock, process, or signer access when that access is part of semantic
execution. Domain helpers may compute deterministic values, parse data, or validate typed inputs,
but side effects and replayable observations must pass through typed capabilities.

Every `ReadExternal` state owns canonical `Plan` and `Evidence` value contracts plus a pure reducer.
The plan is derived only from certified config, input, and context; the reducer consumes the same
input/context, one primary evidence value, and any kernel-owned fact-query evidence. Plan and
evidence schema and semantic identities are included in the state descriptor's effect-contract
digest, so either identity changing produces different descriptor authority.

Runtime uses one generic external-read runner to load arbitrary certified input trees and context,
invoke the state planner, let an adapter execute only the resulting plan, retain exactly one primary
external-read evidence artifact plus any fact-query evidence, and invoke the reducer. Replay loads
the same config/input/context and retained evidence and calls the same reducer. It constructs no
live capability implementation and rejects missing, duplicate, or wrong-schema primary evidence.

## State Capability Boundary

States declare authority. Transports implement authority. Adapters bind the two at runtime.

Capability contract crates are part of the typed state-facing contract. They may define capability
specs, request types, response/evidence types, redacted error contracts, and traits that represent
external authority. They must not perform live IO, route endpoints, resolve signer material, or own
workflow topology.

States may depend on capability contract crates. States must not depend on live transport
implementation crates. For example, a contract validation state may depend on the coherent
`EvmReadCapability` and its source-bound session types. It must not depend on the live JSON-RPC
transport that chooses an endpoint, attaches authorization, or uses a concrete client library.

Runtime/app assembly supplies concrete capability implementations for live execution. Replay
reconstructs state input and context from certified authority and consumes only recorded facts,
typed artifacts, and side-effect evidence. Adapters translate state-owned plans or mutation intent
into capability calls and evidence phases without moving protocol IO or signer material into state
code.

The reusable EVM surface is intentionally narrow. `SubmitEvmTransactionState` is the sole EIP-1559
mutation state, and its closed action is direct `Create` or ordinary `Call`; one side-effect node
always represents one transaction. Operation crates own constructor/call encoding and any
dependency-ordered domain composition. The independent `ValidateEvmContractState` is the one
exact-anchor code/call validation
read state; neither primitive creates a public operation or application entry point by itself.
The production application does not automatically register either primitive for certification,
execution, or replay. Explicit library consumers and tests opt into their separate adapter
registration functions; portfolio production assembly registers only EVM balance collection.

Contract validation accepts an explicitly anchored address and a mandatory expected runtime-code
hash other than the empty-code digest. Its bounded ordered checks retain full caller, target, value,
calldata, gas, access-list, and exact-return context. One bound `EvmReadSession` performs code and
calls at the same EIP-1898 hash selector with `requireCanonical = true`, then re-reads the authored
number and requires the same hash. The state-owned reducer rejects empty code, code-hash drift,
call-context/result drift, session-implementation/network/chain drift, evidence
omission/reordering, and reorgs.
The capability contract owns the 128-KiB deployed-code maximum and every `EvmCall` owns its exact
decoded-result maximum. The HTTP transport derives method body limits from those values and checks
content length and streamed chunks before JSON decoding; its one-MiB limit is only an outer defense
for methods without a smaller semantic response contract.
The compact output contains only the address, anchor, observed code hash, and validation-plan
digest. Evidence-only replay invokes that reducer without a route, transport, signer, or runtime
configuration.

Transaction idempotency is the full schema- and semantic-bound immutable authored intent: semantic
network and chain, expected sender, signer ref, deterministic signing profile, action bytes/value,
access list, and the one checked gas/fee policy. The later prepared invocation is separate authority.
Under the exclusive `(network_id, chain_id, expected_sender)` lane, it fixes the pending nonce, fee
observations, gas estimate, unsigned envelope, signing digest, expected signed hash, derived CREATE
address when applicable, and redacted checked-session evidence. Preparation contains no signature,
raw signed envelope, endpoint, credential, keystore path, or provider body.

The adapter signs once during ordinary preparation and holds the resulting bearer envelope only in
a bounded process-local one-shot cache. Submission consumes those exact bytes. A provider
acknowledgement is accepted only when its hash equals the locally computed prepared hash; that
acknowledgement immediately becomes submission evidence without a visibility lookup. If the submit
exchange does not return a trusted acknowledgement, the adapter performs exact-hash lookup once and
persists `SubmissionUnknown` when the lookup is absent or unavailable. Recovery of that durable
uncertainty is observation-only: it never reconstructs a signature or invokes submission again.
EVM nonce observations never mint a non-submission proof. The sender lane serializes MFM attempts
only and cannot reserve a nonce against another wallet, operator, or process.

The mutation runner binds a `DeterministicSigningProvider` and records the concrete signing
capability implementation independently from the transaction-session implementation. The keystore
provider identity is `mfm.signing.keystore.rfc6979.v1`; the EVM signing boundary rejects a provider
whose deterministic profile is not `secp256k1.rfc6979.recoverable.low_s.v1` before requesting a
signature. Implementing only the unconstrained generic signing-provider contract is insufficient.

Transaction lookup must match every prepared public field. Receipts retain strict status, optional
contract address, and complete coherent successful-execution logs with a lossless immutable read
surface for downstream operations. Only a successful direct `Create` may carry a contract address,
and it must be the sender/nonce-derived address; reverted creation and every `Call` forbid one.
Because top-level revert rolls logs back, reverted receipts must have no logs at capability and
persisted/replay boundaries. Revert is a terminal external effect. Finalized evidence re-reads the
unchanged receipt, checks
its number/hash against a block-by-number result, and proves the certified depth against a fresh
head. Provider, route, transport, HTTP/RPC, response, and source-binding failures after submission
block the open attempt and resume observation from durable ledger evidence; they never terminalize
trusted on-chain success or authorize another broadcast. Deterministic request, retained-evidence,
signed-hash, reducer, and certified-authority violations remain terminal. Replay decodes the same
typed intent, preparation, transaction, receipt, and confirmation artifacts and recomputes their
relations without network, signer, keystore, or current runtime config.

The retained transaction-signing foundation is one canonical path in `mfm-evm`. It admits
one opaque Alloy `TxEip1559`, obtains its signing digest from Alloy, builds one generic digest-sign
request, verifies the exact deterministic profile, low-s signature, recoverable parity, public
identity, and expected sender, then asks Alloy to finalize and EIP-2718 encode the envelope. The
signed transaction hash is Keccak-256 of those exact transient bytes and is cross-checked against
Alloy's hash. There is no legacy/style enum, custom RLP, alternate encoder, normalization fallback,
or second normal-path signing call.

Transaction quantity ingress uses `U256`. Alloy 0.8 represents chain id, nonce, and gas limit as
`u64` and fee fields as `u128`. The one pre-gas type-2 description checked-converts chain id, nonce,
and fees before estimation IO; the exact complete object is sent to `eth_estimateGas` against
`pending`, then the checked gas result is added to that same description to form signing authority.
Envelope admission also checks the gas limit. Unrepresentable values are rejected without
truncation, clamping, or fallback, while transaction value remains full `U256`. This fail-closed
representability boundary is the only production path. Supporting wider fee fields would require
upstream Alloy support, not a parallel MFM envelope implementation.

`mfm-signing` carries the protocol-neutral algorithm and explicit signing-profile ids on every
transient request/result. The admitted EVM profile is deterministic RFC 6979 recoverable
secp256k1 with canonical low-s output. `mfm-keystore` owns encrypted key storage and binds exactly
one runtime signer ref to one keystore entry without exposing a raw-key API. It enforces the generic
algorithm/profile and expected identity, and leaves domain/purpose authorization to the caller.
Each signing call reads at most 64 KiB plus one byte from its unlock file into zeroizing storage on
a blocking worker, rejects oversize or invalid content, strips at most one LF or CRLF, and drops the
unlock value before returning. The app layer also owns keystore selection, profile resolution, and
all import/list/delete implementation work. A binary may capture a one-shot secret only into the
app's consuming `SecretInput`, which has no cloning, formatting, serialization, deserialization,
borrowing, or public accessor contract; app services consume it on a blocking worker. REST exposes
no secret-bearing keystore ingress. App assembly admits the signer/keystore support families without
loading EVM routes, then selects the requested `[signers]` entry and its referenced `[keystores]`
profile.

App assembly keeps evidence-only services separate from live driver services. Status, stream
inspection, list/watch, replay, and public-output rendering construct only store, artifact, and
certification/replay authority; they do not parse live runtime config, construct live EVM transports,
or construct signer providers. Malformed or missing live capability wiring can block live
start/resume when that run needs it, but it must not affect evidence-only reads.

CLI and REST receive one opaque app facade over a shared production store. The app owns concrete
storage, generic store bounds, registry construction, evidence/live service construction, and live
implementation selection. Process transports pass connection and explicit runtime-config inputs,
invoke facade operations, and render results; they cannot construct or inspect those
implementations directly.

Initial admission validates ingress for every domain node. Resume revalidates process-local live
capability only for nonterminal domain nodes that can still execute in the verified stream; a
terminal source read must not make downstream deterministic work depend on an unavailable runtime
configuration.

Runtime admission binds each certified capability descriptor to a registered non-secret
implementation identity before the run can start or resume. Missing or mismatched implementation
bindings are deployment/ingress failures, not semantic attempt outcomes.

Replay and resume semantics follow the effect class:

- Ordinary pure states execute through the runtime-owned generic `PureState` runner, which loads
  certified config, the complete input tree, and certified context, then stages the canonical
  output and validates any declared context output. They replay through the one generic replay
  verifier by reconstructing the same retained values and recomputing exact canonical output.
- Read states replay from recorded read evidence. Replay must not call live transports.
- Side-effect states resume from durable phase evidence such as intent, idempotency, required
  prepared invocation authority, submission, receipt, confirmation, or recovery evidence. Resume
  must not duplicate external mutations or infer mutation status from unstored state. The kernel
  derives the full schema-bound idempotency key; adapters cannot supply or truncate it.
- Replay never constructs live transports or signer providers.
- Replay never resolves current configuration or current-executable identity.

Portfolio holding intent is direct and aggregate-validated: each symbol is a `Native` source or an
EVM `Erc20` source with a normalized non-zero contract address, and EVM native scale is owned by
the semantic network. Admission rejects excessive networks, wallets, symbols, wallet-symbol
relations, or distinct sources for one EVM network before graph expansion or provider work.

Each demanded EVM network is one child call to `EvmBalanceCollectionOperation`. The reusable
operation expands to exactly one fact-producing `CollectEvmBalancesState` and exports only an
`EvmBalanceCollectionReceipt`. The read state owns the sorted native/ERC-20 source
plan and deterministic reducer. One checked source-bound session resolves latest once, reads
deduplicated token metadata and every balance at the exact EIP-1898 hash with canonicality
required, and finishes with one number-to-hash recheck. Reads use bounded concurrency. The
reducer emits one `evm.balance_snapshot` fact per source and the checked receipt; runtime settles
the ordered fact batch, receipt, evidence, and completion in one atomic append.

The generic source contract is `EvmBalanceSource { account, asset }`, where `EvmBalanceAsset` is
`Native` or `Erc20` with a non-zero contract address. Typed constructors accept checked EVM
addresses and persisted values retain their canonical lowercase representation. The fact subject,
collection plan, evidence, fact batch, and receipt use this one EVM-domain algebra; they
contain no portfolio, wallet, symbol, route, endpoint, schedule, or invocation identity. Their
shared EVM block anchor persists the full U256 number as canonical decimal plus canonical hash. The
receipt retains only network/chain, anchor, sorted sources, and verified content identities; it
does not duplicate response material.

`PortfolioSnapshotOperation` constructs collector operation calls and one
`PortfolioReportOperation` call; it constructs no state directly. The report operation receives
the typed Bitcoin and EVM receipt vectors and passes the same structured binding to selection,
which compiles one exact query per demanded holding. The same store that owns retained artifacts
evaluates the complete batch over one snapshot and applies receipt content identity before the
one-row limit. The
production Postgres provider reconstructs append-only fact authority once for that batch, not once
per holding. Selection rehydrates the returned response and rederives full
descriptor/subject/response identity and fact refs before accepting it. Content identity
intentionally treats any number of byte-identical append occurrences as equivalent; deterministic
last-write ordering selects one occurrence without scaling hydration with history. Missing receipt
content, mixed frontiers, malformed cardinality, tampering, unexpected receipts, or incomplete
coverage fail closed. The
report operation assembles only the selected store material, rechecks exact config-derived
coverage, and projects the structured report. EVM collection replay belongs only to the private
adapter in `mfm-evm-live`; portfolio replay verifies selection, snapshot, and report. Both use
retained evidence without a live route. Public-facts CLI/REST is not portfolio selection authority.

## Certified Saga Semantics

Certified saga behavior is part of the typed runtime contract. The detailed saga and scoped AC/DC
contract is maintained in `docs/saga.md`; this section summarizes the authority rules that every
runtime, store, replay, CLI, REST, and app change must preserve.

MFM implements certified saga semantics for external side effects. It does not claim full AC/DC
semantics for arbitrary external systems. Stronger AC/DC-style claims require MFM-owned
transactional resources or replay-verifiable domain proof from typed evidence. Core saga outcomes
therefore mean exactly what the certified stream can prove: forward success, certified
compensation, authorized manual decision, or failure without an AC/DC-equivalence claim.

`TypedExecutionSpec` carries hash-defining saga policy. `mfm-store` derives saga engagement,
obligations, run mode, manual-block state, resource lanes, and terminal agreement from the
certified policy plus the append-only stream. Directive selection, obligation open/close,
run-mode changes, and manual-block requests are not separate event families.

Saga handling engages at the first non-retryable failure or forward side-effect ambiguity. After
engagement, no new forward side-effect boundary crossings may be admitted. Runtime drives
past-boundary forward ledgers to quiescence before resolving obligations or terminal outcomes.
Remediation ledgers use the same side-effect protocol as forward ledgers and carry
`SideEffectLedgerPurpose::Remediation { forward_pair_id }`.

Public status reports semantic `RunMode`: `forward`, `remediating`, `manual_blocked`,
`completed`, `compensated`, `manually_resolved`, or `failed_without_acdc_claim`.
Attempt lifecycle is reported separately as committed attempt dispositions: `started`, `completed`,
`failed`, or `interrupted`. Interruption is retryable attempt bookkeeping, not a run mode, saga
engagement, compensated outcome, AC/DC claim, or manual-resolution authority.
`Compensated` is never a vacuous outcome; it requires owed obligations closed by certified remedial
evidence. `FailedWithoutAcdcClaim` is the honest terminal result when policy permits failure
without a compensation or AC/DC-equivalence proof.

Manual resolution is a signed authorization protocol. Certification uses the registry as live
authority for manual evidence schema roles, manual authorization verifier identities, operator
authority snapshots, supported signing scheme, and quorum. The certified spec and certificate then
carry replay authority. Runtime and replay verify manual resolution from certified
spec/certificate, stream prefix, retained artifacts, and canonical proof bytes only; they must not
call live signer, registry, keystore, environment, or runtime signer sources.

`ManuallyResolved` means an authorized manual decision was recorded. It does not mean MFM
independently proved external domain truth.

## Certified Spec

`mfm-spec::v1::TypedExecutionSpec` is the persisted execution contract. It includes:

- spec version and canonicalization identity
- certified transition context table entries, context refs, and node/cell/input context constraints
- certified descriptors and executable identity requirements
- scopes, seeds, configs, nodes, cells, bridge nodes, and public outputs
- input binding trees and value lineage
- effect and capability evidence
- hash-defining external-read and side-effect contracts
- retained config and seed artifact refs
- public-output render nodes and output evidence

The spec hash is computed from canonical bytes. The erased runner plan must be reproducibly derived
from the certified spec and runner registry. It must not carry semantics missing from the certified
spec.

Certification validates transition-context authority before a spec can become runtime authority:
context refs must be content-derived from the certified context table, no-context descriptors cannot
run under a semantic context, context-required nodes must match the registered context descriptor,
context-bound user outputs and inputs must match their descriptor resource kind, stage, and approved
producer contract. The producer contract may list multiple approved state descriptors for a stage,
and raw seeds cannot produce context-bound resources unless the certified producer constraint
explicitly permits seed producers. Framework same-value bridges and
side-effect verify nodes may only preserve an existing context binding; framework receipt nodes must
remain no-context.

Runtime preserves that authority after certification. `CertifiedRuntimeSpec` indexes certified
context table entries by `ContextRef`, sealed runner invocations materialize typed
`CertifiedContext<C>` values only from the certified node context, and no-context states receive only
explicit no-context authority. Input materialization rechecks that each input binding context matches
the certified source cell and that required context-bound inputs are consumed under the same node
context ref. Context-bound state-output cells also require a registered runner extractor; terminal
output admission checks the `CellProduced` context against the certified cell and validates the
staged state-output artifact bytes against the certified context ref, resource kind, and stage.

`mfm_spec::v1::HashedSpecEnvelope` is not certification authority. Persisted spec bytes, hash-only
envelopes, and persisted certificate bytes are hostile data until `mfm-certify` verifies them
against a registry and returns `CertifiedTypedSpec`.

## Store And Events

`mfm-store` is the only semantic commit contract for certified typed runs. Production execution
callers submit purpose-specific `PreparedCommit<Purpose>` authority through `PreparedCommitPlan`.
Each plan carries typed event payloads, commit preconditions, and the artifact evidence that becomes
run authority in the same atomic append. The store constructs envelopes and maintains projections.
Synthetic direct mutation is confined to explicitly named non-execution test, migration, repair,
corruption, or low-level storage contract fixtures.

Events carry typed artifact-reference facts but do not define storage requirements or artifact-read
capabilities. `mfm-store` alone derives exact retained-artifact requirements from those facts and
owns `RetainedArtifactReadProvider` plus `VerifiedRetainedArtifactBytes`. A retained read verifies
artifact id, evidence hash, digest against the actual bytes, byte length, media type, schema id,
semantic type id, producer binding, and role before the bytes can contribute to run-history
authority.

The authoritative event stream contains:

- run start and attempt lifecycle events, including interrupted-attempt terminal bookkeeping
- seed/config/fact/artifact evidence
- cell terminal events
- side-effect ledger events
- public-output render and produced events
- retention refs and manifests
- run completion or terminal failure

Projection corruption is repairable by rebuilding from the run stream. Projection data must never
be the sole authority for resume, replay, public output, retention, or side-effect status.

The first certified persistent storage path is:

```text
crates/storages/postgres
```

Postgres is the only production persistence backend. It stores append-only `commits`, canonical
`run_events`, artifact blobs/evidence, resource-lane claim/release/transition rows,
commit cursor authority, store metadata, and mutable operational admission-lane coordination rows.
`store_metadata.store_scope_id` is store-owned, non-secret identity material for the deployment's
trust boundary; callers cannot supply or update it. Observation list/watch rows are derived from
strict authority at read time. Artifact bytes live in Postgres; production app, CLI, and REST paths
do not stage, read, or migrate workflow artifacts through filesystem artifact roots. The schema and
migrations are owned by `crates/storages/postgres`; runtime callers validate schema
contract shape and must not run startup auto-DDL. Because MFM is pre-production, replacing a
persisted contract shape is a destructive schema change that updates the baseline directly.
Existing-run detection folds authoritative `run_events`; there is no separate run-admission index.
Admission-lane rows are operational coordination only. They may select who tries next or who is the
current active driver, but they never grant replay, resume, public-output, side-effect, resource
ownership, or terminal-state authority. Observation rows and list/watch cursors have the same limit:
they are read models, not authority.

Production deployments must give the Postgres run store a dedicated MFM database tenancy. List/watch
cursors order committed `commits` rows by the durable store-owned `store_commit_order` coordinate;
the coordinate is assigned within the append transaction and advances only with a successful commit.
Run-local validation, artifact verification, and projection rebuild complete before the process
acquires the global `store_commit_order` row lock (`SELECT … FOR UPDATE`). That lock is then held
until commit so order assignment and durable append materialization stay atomic. Cross-run contention
on the remainder of the transaction is accepted for correctness; multi-transaction order allocation
is not used. Cursor epochs and artifact cleanup have no public v1 maintenance entry points; any
future maintenance role must first specify Postgres roles, ownership, credentials, and restore/clone
runbooks.

v1 has two operational lane uses:

- execution lanes: `nowait_skip` leases keyed by base work identity
  (`certified_spec_hash` + `store_scope_id`) with the concrete holder `run_id` stored separately;
- resource-admission waiters: FIFO waiters for one certified exclusive side-effect resource claim.

Execution-lane acquire, renew, release, and reap require the holder `run_id` and token to match the
current row. Acquire does not auto-reap expired holders. Resource-admission checks run under the
store transaction and may use Postgres advisory transaction locks as an implementation detail. A
resource claim commits only when the certified lane has no active authoritative holder and no earlier
live waiter. `AdmissionBlocked` persists no run event, commit, resource-lane claim/release,
lane-transition, or other domain authority row; it may insert or refresh one mutable waiter row.
Expired or admitted waiters no longer block later attempts, and retries after expiry receive a fresh
lane-local ticket. Release notifications are wake hints only.

## Process-Fungible Execution

MFM runs are durable certified work, not process-owned work. A process can start, drive, stop, crash,
or resume, but the run's meaning comes only from the certified spec, `RunAdmitted`, the append-only
stream, and store validation.

The model has three identities:

- run identity: `certified_spec_hash` + store-owned `store_scope_id` +
  `invocation_key_digest`; this derives `run_id` and is recorded in `RunAdmitted`;
- execution lane: `certified_spec_hash` + `store_scope_id`; this allows at most one live driver for
  the same base work in one store scope;
- resource lane: the certified side-effect resource key resolved by state preflight; this protects
  external mutation.

A worker may drive a run only after all execution validations pass:

- the stored stream validates against the certified runtime spec and recorded run identity;
- the worker's executable and capability bindings match the stored admission evidence;
- the worker holds the live execution-claim token for the run's execution lane and holder `run_id`.

Public start uses the same rules. If the requested `run_id` already exists, compatible callers attach
or observe `already_active`. If a different invocation of the same base work is already driving, the
execution lane returns `already_active` with the holder `run_id` and no contender `RunAdmitted` event
is appended.

Execution lanes reduce duplicate live work; resource lanes protect side effects. Neither replaces the
other. Execution-claim expiry is not permission to repeat an external mutation. It is permission to
reload the stream and either continue from recorded side-effect evidence, record defended terminal
evidence, or block for certified manual/resolution policy.

v1 is invoker-driven and manual-resumable. Automatic dead-driver takeover, background worker-pool
dispatch, feed-driven dispatch, `due_at` re-wake, long-wait tenure release, pipelined nonces, and
multi-lane admission remain deferred unless this document and `docs/saga.md` define a new certified
contract.

## Runtime

The runtime is the authority boundary for an event-sourced typed state-machine workflow. Its input
model is deliberately small:

```text
static certified transition graph + verified run history
  -> deterministic frontier scheduler
  -> sealed runner invocation
  -> guarded commit
```

The runtime authority contract starts from `CertifiedTypedSpec`, not from parsed spec JSON or a
hash-only envelope. Authoring and persistence move through explicit stages:
`UntrustedTypedSpec`, `LoweredTypedSpec`, certifier-private `ValidatedTypedExecutionSpec`, and then
non-forgeable `CertifiedTypedSpec`. Persisted nodes and renderers refer to descriptor authority by
`DescriptorRef`; `CertifiedDescriptorSet` and `CertifiedFrameworkLifecycle` are the certified views
runtime consumes. `CertifiedRuntimeSpec` is the runtime view of the static certified transition
graph. Before `RunAdmitted`, the assembly/runtime boundary verifies:

- spec hash and schema/version fields
- staged spec/certificate/config/seed artifact bytes and typed evidence
- descriptor identities and executable identity requirements
- runner registry availability
- capability registry availability
- context-bound output extractor availability

Context-bound output value traits live in `mfm-values`. Runtime owns extractor registration,
artifact decoding, and admission checks against certified context-bound cell constraints.

After launch, runtime advances only from the append-only run stream authority. It loads the stream,
delegates spec-independent ordering and projection checks to `mfm-store`, then performs
runtime-owned spec-aware validation of seeds, configs, artifacts, completed cells, side-effect
ledger evidence, public-output events, retention events, and terminal run state. The rebuilt
projection is derived from the stream; it is not independent semantic authority. Store-owned stream
validation requires attempt-bound payloads to be preceded by a separate `StateAttemptStarted`
commit before runtime, replay, or Postgres-backed loads trust them.

Read, resume, replay, and status paths must construct a `VerifiedRunHistoryView` from a
`CommittedRunStream` plus `VerifiedRunArtifactStore` before trusting history. Replay authority is
minted from the verified view and certified runtime authority; raw event vectors or retained
artifact bytes without committed evidence do not cross the runtime/replay boundary. Scheduler drive
paths construct `VerifiedRunContext` through
`VerifiedRunContextLoader`, which combines the same committed stream/fold authority with
`BoundRuntimeContext` runner, capability, and framework-handler authority before any transition is
selected.

The deterministic frontier scheduler is pure. Given the static certified transition graph and
verified run history, it returns one closed transition decision: start a node, continue an open
attempt, start remediation, wait for manual resolution, resolve saga terminal state, or report
blocked. Public-output, retention, and completion work are ordinary certified
framework node selections; an already projected public output is a scheduler facade status, not a
frontier transition decision. Open-attempt recovery, including legal interruption, is handled by the
attempt recovery lifecycle when the continued attempt is dispatched. The frontier decision does not
write the store, stage artifacts, construct live capabilities, or call runners. Open-attempt recovery
classifies verified open attempts into continue, retry terminalization, interrupt, side-effect
recovery, or operational block dispositions. Operational blocks are reserved runtime recovery states
for malformed evidence such as terminal side-effect ledger evidence without matching
attempt-terminal evidence; normal store/history validation rejects those malformed streams before
scheduler recovery, and public status collapses any surviving operational block to blocked without
minting semantic terminal events.

The runtime exposes one async drive path. Lifecycle authority belongs in pure helpers for
transition/recovery classification, attempt planning, invocation build, output validation, and
commit planning.

For a new ordinary runnable node attempt, runtime first appends `StateAttemptStarted` from
certified attempt authority. It then materializes state inputs from certified binding trees and
prior typed cell evidence, materializes the certified node context, checks runner identity and
capability availability, and constructs a sealed runner invocation. If post-start materialization,
runner-output validation, or runtime validation fails inside a valid started attempt and no
side-effect authority has been acquired, runtime stages a redacted diagnostic artifact and records a
failure-safe `StateAttemptFailed` plus runtime-evidence retention from minimal trusted attempt
authority. Corrupt history before a valid
attempt context, missing deployment bindings, storage/artifact outages before terminal evidence
commits, and side-effect attempts with acquired ledger authority remain non-semantic runtime or
recovery concerns. Runners receive only scoped typed inputs, allowed capabilities, and erased
context surfaces. They return typed payload intent, staged artifacts, side-effect evidence, or
sealed handles but cannot append to the run stream. Context-bound output artifacts are accepted only
when the runner's registered extractor can recover the certified context metadata from the staged
bytes and it matches the output cell's certified context binding.

For `ReadExternal`, the shared runner performs config, arbitrary input-tree, and context
materialization; state planning; adapter plan execution; state reduction; and typed output/evidence
staging in that order. Adapters do not own a second reducer or recorded-provider implementation.

The commit planner owns all production execution appends. `RunAdmissionLifecycle` verifies and
stages launch material, then commits exactly one `RunAdmitted` root event with certified spec,
certificate, config, seed, executable, binding-digest, framework, source, and caller launch-time
evidence. Root artifact retention is projection-derived from `RunAdmitted`; launch does not append
attempt, cell, artifact-reference, or retention-ref payloads. Ordinary states, `PublicOutputRender`,
`ProjectRetentionManifest`, `CompleteRun`, and `ResolveSagaTerminal` append
`StateAttemptStarted` before sealed invocation construction, then use the same guarded terminal
commit path: output/reference bindings are checked against the certified graph, side-effect
protocol rules are enforced, commit preconditions are built, and artifact bytes/evidence are
admitted only by the commit that first references them. Production callers submit
`PreparedCommitBundle` values built from purpose-specific `PreparedCommit<Purpose>` authority;
stores do not expose or accept a raw typed-batch or plan-only append escape hatch. Purpose
constructors reject purpose mismatches, missing `CertifiedRunStoreAuthority`, missing `SagaTerminalProof`, or
artifact evidence that was not admitted in the same commit. Artifact blobs are admitted inside the
append transaction; failed appends leave no authoritative run-store evidence.

Normal launch identity is content-addressed from `RunIdentityMaterialV1` using canonical JSON:
`certified_spec_hash`, store-owned `store_scope_id`, and required `invocation_key_digest`. Public
entry-point starts accept an optional raw `invocation_key`; when omitted, the app mints a fresh opaque
key before deriving identity. The raw key is not persisted. The run id must equal the digest of the
recorded identity material, and `RunAdmitted` records the material so attach, resume, replay, status,
and public-output authority can fail closed on identity mismatch. Raw caller-supplied run ids are not
a normal launch surface.

Run admission and first execution-claim acquire are one store operation. The app validates the
store-owned scope, the certified spec hash, and the derived `run_id`; the store validates that
the prepared execution claim matches the `RunAdmitted` identity material before admission. If the
execution lane is already held, the store returns `ExecutionClaimBusy`, appends no contender run
event, and public start reports `already_active` with the holder `run_id`.

Resume reads the recorded identity from `RunAdmitted`, rebuilds the execution lane from that identity,
validates stored launch evidence and executable/capability bindings, then acquires or renews the
claim. Runtime drive entry points require the current token for the execution lane and holder
`run_id`; callers without it cannot drive through the public scheduler API. The v1 claim lease has a
60 second TTL and a 20 second heartbeat interval. While short receipt-level waits are active, the
invoker loop keeps heartbeating instead of releasing tenure.

Framework lifecycle work is represented by certified graph nodes, not ad hoc runtime side effects.
Run admission is the sole pre-attempt root authority and is not represented by a certified graph
node. `PublicOutputRender`, `ProjectRetentionManifest`, `CompleteRun`, and
`ResolveSagaTerminal` are sealed framework runners with the same append-only stream, rebuilt
projection, deterministic scheduler, started-before-run attempt lifecycle, and guarded commit rules
as domain states.

Resume loads the stored certified spec, rebuilds the verified history and projection from the run
stream, verifies completed cell and side-effect evidence against the spec, then advances only from a
type-valid frontier.

Replay loads the stored certified spec and certificate artifacts, verifies them against the compiled
certification registry, compares the hashes to `RunAdmitted`, and rebuilds stream evidence.
External reads are recomputed by the generic replay driver through the state-owned reducer;
side-effect protocols use their evidence-only domain verifier. Live capability construction during
replay is a contract violation.
Replay service construction itself is evidence-only app assembly: it must not construct the live
runner registry, live transports, signer providers, keystores, or live capability runtime config.

Live provider identity is enforced by bound session implementations. Runners derive a certified
semantic binding, such as EVM `network_id` plus expected chain id or Bitcoin `network_id`,
`source_identity`, and expected network tag, before issuing operation-only capability requests. An
endpoint-bound Bitcoin session is a public reusable transport under
`mfm_bitcoin_live::transport`. App assembly constructs those sessions and supplies one app-private
routed implementation of the same pure `BitcoinBalanceSession` trait to the private live adapter;
the adapter cannot import or construct the concrete transport. A route is selected by certified
semantic source identity. Its checked session is fixed for one start/resume dispatch and discarded
afterward.

An EVM collection attempt binds one direct route, probes its chain once, and persists one redacted
session identity: `network_id`, chain id, `source_ref`, and transport `implementation_id`. Every
native balance, ERC-20 metadata, ERC-20 balance, and number-to-hash re-verification uses that same
session. The canonical capability evidence is carried directly in the aggregate EVM collection
evidence; the shared block anchor and typed address/asset algebra are likewise retained without
portfolio-specific mirrors. The checked collection receipt carries the semantic network, chain,
anchor, sorted sources, and verified fact content identities, while source provenance remains only
in read evidence. Balance responses remain solely in retained facts and are reread before portfolio
assembly. One live attempt keeps one fixed source-bound session. Its checked `source_ref` is audit
provenance explaining which
process-local route served that attempt; it may differ on a later attempt or resume after runtime
routing changes. Replay never resolves it against current runtime config or compares it with a
current route. Post-commit changes remain detectable through ordinary artifact digest and stream
integrity. If provider identity must influence semantic trust, the author must certify an explicit
oracle/source identity instead of relying on process-local routing. An EVM route whose bind-time
chain probe observes an incompatible chain fails before admission, or before resume claim
acquisition, with a closed redacted provider diagnostic. Bitcoin Core chain identity is necessarily
observed by the operation-time `getblockchaininfo` call; its mismatch fails the admitted attempt.
Provider diagnostics carry a
provider family, closed diagnostic code, optional redaction-safe operation id, and closed
boolean/integer/id fields only. Examples include HTTP status, JSON-RPC numeric code,
response-shape failure, unsupported operation, operation incomplete, and source mismatch. They must
not carry RPC URLs, authorization headers, file paths, provider messages, request/response bodies,
signer material, or signed transactions. Replay decodes the checked session evidence and verifies
it against the certified binding without resolving source refs through current runtime config.
For EVM reads and pre-submission work, the phase-aware classifier treats HTTP 408, 425, 429, 500,
502, 503, 504, and 507 plus JSON-RPC `-32603`, `-32001`, `-32002`, and `-32005` as operational
availability/resource failures. Other numeric HTTP and JSON-RPC rejections are terminal contract
failures, as are missing numeric classification fields. After submission, every provider failure is
operational because it cannot prove non-mutation or authorize another broadcast.

Manual-resolution replay additionally verifies that the stream prefix derives `ManualBlocked`, the
event matches certified policy, evidence and authorization artifacts match certified roles and
digests, the canonical proof claim matches the event and prefix exactly, signatures verify, signers
belong to the certified authority snapshot, and quorum is satisfied. Runtime and replay build this
through `ManualResolutionPrefixAuthority` and `ManualResolutionProofAuthority`; only a
`VerifiedManualResolutionForPrefix` can authorize the corresponding manual-resolution commit or
terminal saga proof. Terminal saga proof construction re-verifies that authority from the current
verified prefix and retained artifacts; scheduler-local proof caches are not authority.

## Side Effects

Side-effect states are multi-commit protocols. The scheduler/store own:

- logical ledger key derivation
- idempotency key stability
- claim generation and fencing
- invocation epoch
- durable transition ordering
- ambiguous recovery blocking
- terminal output binding

The durable uncertainty boundary is the invocation-started event. After that boundary, resume must
recover or block using typed evidence; it must not duplicate an external mutation or guess from
unstored state.

`CertifiedSideEffectContract` is the single side-effect resource-claim and verification authority
shared by live execution, resume, and replay. Verification is a hash-defining contract value:
`Receipt` terminalizes from receipt evidence, while `Finalized(depth)` terminalizes only from
confirmation evidence at the certified depth. The depth is lowered into the certified spec, never
resolved from a registry at admission or from worker-local policy. `RunAdmitted` may echo launch
diagnostics, but it cannot override or duplicate this verification authority. Receipt-level
terminalization is final-at-risk by explicit operation design: a later reorg can invalidate the
published output or wedge the next nonce, and that recovery belongs to the deferred stuck-transaction
or nonce-reclaim workflow.

`CertifiedSideEffectContract` exposes domain output construction for the configured terminal
evidence level. Runtime invokes the domain-owned receipt or confirmation output contract and never
constructs domain output values itself. `NotSubmittedProven` is a defended investigation outcome,
not a terminal result from one transient RPC miss.

The store exposes legal phase information through
`SideEffectLedgerState`, so transition admission is a typed state-machine check instead of an
optional-field projection heuristic. Forward side-effect ambiguity is admissible only when paired in
the same commit with the non-retryable attempt failure that engages saga handling.
The store is the source of truth for forward-fence admission after saga engagement; runtime
early-rejects are scheduling convenience and cannot substitute for store validation.
Resource-lane scheduling remains conservative. A lane-blocked attempt is parked before invocation
and may be retried after bounded backoff or a lane-release wakeup; ordinary contention is not
terminal evidence. Runtime resolves concrete exclusive lane keys in pure preflight, then asks the
store to materialize `ResourceLaneClaimed` from `ResourceLaneClaimIntent`. The store assigns the
lane-local fencing token and transition sequence and records the lane mirror/transition rows in the
same append transaction. `ResourceLaneClaimed`, not the waiter row, is held-lane authority. Recovery
must either reuse that committed held lane for the same invocation or release it through certified
cleanup authority before interruption. `SideEffectInvocationPrepared` and every later phase are
owned by `SideEffectLifecycle`; recovery either resumes from the concrete ledger phase, records
evidence-backed terminal side-effect outcome, or reports an operational block.

Prepared-invocation artifacts are required before the invocation-started boundary and may retain
unsigned mutation plans, expected hashes, and non-secret signer references. Preparation runs only
under a committed claim. Signed raw transactions are bearer mutation material and remain transient
submit-time bytes inside the mutation adapter. They are neither serializable typed values nor
cloneable service results. The explicit user-selected `keystore tx-sign --out` file is the only
non-run bearer-output boundary; CLI output reports distinct `signing_digest` and
`transaction_hash` metadata and never the raw bytes, signature, or local path.

Process-local authority that becomes usable only after a runner commit uses a non-cloneable
one-shot `RunnerOutputSettlement`. Runtime executes its callback only for
`CommitOutcome::Appended`. Idempotent, admission-blocked, stale, failed, or dropped outputs destroy
the captured authority without promotion, including the case where an append returned an error but
another read later observes that the commit exists. Durable events remain the only recovery
authority.

Submission observed, submission unknown, and not-submitted-proven evidence share one logical
submission-result slot for an invocation epoch. Unknown submission can be superseded only by the
legal recovery transitions enforced by the store.

## Public Outputs

Public-output specs are part of the certified spec. Runtime public output is produced by typed
render states and events. Rendered JSON artifacts are cache material and must be checked against
typed output evidence before use.

Rendering helpers require `PublicOutputReadAuthority`, which `mfm-app` mints only after verifying
stored certified authority and rebuilding the projection from the authoritative run stream.
Rendered JSON cannot be used as resume, replay, certification, or render authority.

CLI and REST outputs are public API surfaces, but they are not semantic execution authority.

## Architecture Placement

This design contract defines runtime authority. Crate placement, taxonomy, operation/state/adapter/
transport/signer/config boundaries, public naming rules, and reviewer checks are maintained in
`docs/architecture.md`.

The semantic package metadata, responsibility taxonomy, dependency matrix, and placement rules are
maintained in `docs/architecture.md#semantic-package-metadata`.

## Documentation Update Rules

Update this document when a change alters:

- certified spec semantics
- typed event schemas
- store commit or projection authority
- resume or replay semantics
- certified saga or manual-resolution authority
- effect/capability rules
- public-output authority
- crate ownership boundaries
- CLI or REST runtime contracts

Use `docs/architecture.md` for the short contributor map.

The former typed-core source-scan gates and summary-key CI scripts have been deleted. Real
guarantees now live in typed APIs, private constructors, crate dependency boundaries, Rust tests,
trybuild fixtures, cargo-metadata checks, and production-path integration tests.
