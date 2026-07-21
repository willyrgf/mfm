# Implementation Plan: RFC Repository Reorganization

- Status: Accepted
- RFC: RFC_REPO_REORG.md
- Date: 2026-07-21
- Delivery: one buildable, reviewable commit per numbered step

## Binding outcome

Implement the RFC with these decisions fixed:

1. Use one pure crate per current domain as a deliberate concentration experiment. Preserve model,
   capability/signing, state, and operation responsibilities with private modules, one-way source
   dependencies, narrow root exports, and architecture tests.
2. Colocate each source-domain adapter and transport in one live crate, but keep transport public,
   typed, and independently reusable. Adapter modules stay private and expose only narrow
   registration functions.
3. Facts and retained artifacts are platform-wide. mfm-facts owns domain-free fact semantics,
   mfm-store owns store IO and retained artifact authority, and concrete stores implement those
   traits directly. mfm-portfolio-live owns only portfolio-specific query/hydration behavior.
4. Do not add ManagedFactWrite or another generic write runner. A ReadState returns its deterministic
   fact batch; the external-read runner settles evidence, facts, output, and terminal events in one
   atomic append.
5. Remove speculative fact provenance, visibility, request, and observation fields which have no
   production producer or consumer. The current facts surface contains indexed platform facts.
6. Use the published portfolio operation's independently declared authoring catalog as production
   registry authority. Representative drafts test behavior, not topology completeness or semantic
   reachability.
7. Replace Bitcoin per-address reads with one bounded multi-descriptor scan and no process-local
   scan coordinator.
8. Put runtime document parsing/resolution and keystore application services in mfm-app. Live crates
   receive resolved inputs. Binaries may name presentation-safe ID/canonical contracts, but app is
   their only execution and implementation dependency.
9. Preserve no backward compatibility. Delete replaced code and reset persisted store authority
   where necessary; do not create migration, fallback, facade, alias, or legacy reader paths.
10. Treat the exact live worker executable bytes as execution authority. Keep only factory ID and
    binary digest, construct every live registry from one app-supplied template, and deliberately
    reject cross-binary/cross-build live resume; evidence-only replay performs no executable IO.

If implementation evidence disproves a decision, stop at the relevant commit boundary and obtain an
architecture decision. Do not silently introduce another crate, public role module, alternate
execution path, or compatibility layer.

## Resolution of reviewed concerns

| Concern | Final choice | Proof required during implementation |
|---|---|---|
| Fact/artifact providers in portfolio live | Rejected. Contracts and providers move to facts/store and concrete stores. | No generic provider implementation or concrete-store dependency in mfm-portfolio-live. |
| One pure crate per domain | Adopt as the current experiment, not a package-count invariant. | Private role modules, source-direction checks, reduced public surface, and no forbidden dependency. |
| Adapter and transport in one crate | Adopt colocation only. Responsibilities stay distinct. | Public typed transport works without a registry; private adapter works with a fake session. |
| Private transport modules | Rejected. Bitcoin and EVM transport modules are public. | Positive external-use tests and compile-fail/private raw-RPC tests. |
| Generic managed fact runner | Rejected after tracing current producers. | Read settlement atomicity/replay tests and complete deletion of managed-write branches. |
| Version choreography | Rejected as unnecessary compatibility work. | One strict current schema/authority; old stores and shapes fail closed. |
| Label-derived executable identity | Rejected. Live assembly hashes exact worker bytes; redundant package/Nix fields are deleted. | One required registry template, no label/default path, cross-digest resume rejection, and replay with no executable IO. |
| App-only binary type boundary | Narrowed. App is the execution boundary; safe ID/canonical value types may be used directly. | No app mirror DTOs and no direct implementation/runtime/domain behavior dependencies in binaries. |

## Material uncertainties

The target decisions are accepted. These remaining uncertainties are empirical implementation
questions, not permission to introduce parallel designs:

1. The concentrated pure-domain layout may expose a Cargo firebreak that modules, visibility, and
   tests cannot express cleanly. If that happens, the consequence would be an unjustified public
   surface or forbidden dependency inside a consolidated crate. Validate Bitcoin first by measuring
   its public-item/bridge-type diff and source-role checks; stop at that commit boundary and obtain a
   new architecture decision before copying the pattern.
2. The atomic external-read/final-Bitcoin cut assumes both stores can admit and recover the complete
   fact/output/completion settlement without a fact-prefix state while the product moves directly
   from per-address reads to one aggregate read. A failure would invalidate the single execution-
   path design or force an even larger store/runtime redesign. Resolve it with injected append
   failures, uncertain-result recovery, exact graph, and aggregate transport tests against both
   in-memory and Postgres stores before domain consolidation.
3. The independent authoring catalog may still miss a domain descriptor that graph construction can
   produce. That would make production assembly reject a valid published branch or silently omit a
   runner. Generate catalog and typed registries from one declaration, reject undeclared children
   in the authoring builder, and require the union of canonical Bitcoin-only, EVM-only, and mixed
   expansions to equal every declared domain operation/state. Do not infer either direction from
   the app registry being compared.
4. The repository-pinned Bitcoin Core version and regtest behavior may expose a consumed response
   field not covered by the reviewed Core 28+ contract. The consequence would be a decoder or parity
   failure. Resolve it with raw-envelope fixtures and the pinned live parity task, updating the one
   strict additive decoder if evidence requires it; do not add version branches.
5. Selective configuration decoding must coexist with whole-document duplicate-key and secret-name
   rejection for both JSON and TOML. If the current parser stack cannot prove those properties, add
   the smallest parser support justified by focused adversarial tests; do not weaken the closed
   security scan or deserialize every unused entry.
6. Exact executable hashing must prove stable running-byte authority on Linux and the immutable
   packaged-path contract on macOS while evidence-only services perform no executable IO. Validate
   both targets where CI supports them, streaming/error redaction, and cross-digest live resume. A
   failure requires a new execution-identity architecture decision; do not restore a package,
   runner, environment, or shared-label digest.

No backward-compatibility, migration, fallback, version-negotiation, provider-ownership, or private-
transport question remains open.

## Target workspace and dependencies

    crates/
    |-- kernel/
    |   |-- ids
    |   |-- canonical
    |   |-- values
    |   |-- capabilities
    |   |-- facts
    |   |-- program
    |   |-- program-derive
    |   |-- spec
    |   |-- certify
    |   |-- events
    |   |-- store
    |   |-- manual-auth
    |   |-- runtime
    |   +-- replay
    |-- domains/
    |   |-- bitcoin
    |   |-- evm
    |   +-- portfolio
    |-- live/
    |   |-- bitcoin
    |   |-- evm
    |   +-- portfolio
    |-- signing
    |-- keystore
    |-- storages/
    |   +-- postgres
    +-- app

    bin/
    |-- cli
    +-- rest-api

    tests/
    +-- integration

Final crate names:

- mfm-bitcoin, mfm-evm, and mfm-portfolio;
- mfm-bitcoin-live, mfm-evm-live, and mfm-portfolio-live;
- mfm-signing, mfm-keystore, mfm-storage-postgres, and mfm-app;
- the fourteen kernel crates shown above;
- the CLI, REST API, and integration-test crates.

The approximate count is descriptive only. No test may assert the total or an exact package-name
inventory.

Allowed high-level direction:

    domain-facing kernel and mfm-signing
                    ^
                    |
          source pure domains
             ^             ^
             |             |
      portfolio pure   same-domain live
             ^             ^
             |             |
             +---- mfm-app -+---- mfm-keystore
                       |
                       +---------- mfm-storage-postgres
                       |
                    binaries

Additional rules:

- mfm-bitcoin and mfm-evm never depend on mfm-portfolio.
- mfm-portfolio may depend on the source pure domains and never on live crates.
- A source live crate depends on its corresponding pure domain and platform contracts, never on
  another live crate, app, Postgres, keystore, or a binary.
- mfm-portfolio-live may use mfm-portfolio and pure Bitcoin/EVM decoding contracts. It never depends
  on Bitcoin/EVM live crates or concrete stores.
- mfm-keystore may depend on mfm-signing; the reverse edge is forbidden.
- mfm-storage-postgres depends on mfm-store; the reverse edge is forbidden.
- Binaries call app services for execution and implementation assembly. Direct mfm-ids and
  mfm-canonical use is allowed only where their values are part of input/output presentation.

## Internal crate contracts

### Pure domains

Use these private role modules:

    mfm-bitcoin
    |-- model
    |-- capability
    |-- state
    +-- operation

    mfm-evm
    |-- model
    |-- capability
    |-- signing
    |-- state
    +-- operation

    mfm-portfolio
    |-- model
    |-- state
    +-- operation

The only allowed internal direction is:

    model  <-  capability  <-  signing  <-  state  <-  operation

Interpret the arrow as may-be-used-by. Model imports no domain role; capability may import model;
signing may import model/capability; state may import all lower roles; operation may import all lower
roles. No role imports a role above it. Operation expansion remains deterministic and contains no
ambient IO.

Role modules are private. The crate root re-exports only types that a final consumer must name.
Moving code into one crate must reduce visibility for package-crossing helpers; do not preserve old
module paths or root glob exports.

The pure crates cannot import runtime execution, replay, store IO, app, live integration, Postgres,
keystore, filesystem clients, environment readers, or network clients.

### Source live crates

The final source-live shape is:

    pub mod transport;
    mod adapter;

The crate root exposes narrow registration/binding functions from adapter; it does not glob re-export
transport. Canonical transport paths are mfm_bitcoin_live::transport and
mfm_evm_live::transport.

Independent reuse means the consumer API and transport source do not require adapter construction;
the package still builds as one crate. Do not add adapter/transport feature flags or recreate a
second transport package for compilation isolation.

The transport module:

- exposes a checked typed session, resolved endpoint/auth input, and redacted typed error;
- implements session/provider traits owned by the corresponding pure domain;
- is constructible and usable by a library consumer without runtime/replay/registry construction;
- does not name adapter, runner identity, runtime, replay, store, app, or another domain;
- keeps raw request/response envelopes, arbitrary JSON-RPC calls, and HTTP clients private; and
- has no transport-level automatic retry which would escape runtime attempt accounting.

The private adapter module:

- binds pure-domain intent to a supplied session and runtime registration;
- depends on the pure-domain session trait and must not import the concrete transport;
- can be tested with a fake session and without concrete HTTP; and
- owns no domain reduction or operation topology.

App assembly constructs public endpoint-bound transport sessions and supplies only the pure session
trait to the root registration function. There is no second concrete-adapter convenience path.

Multiple configured routes are instances of one capability implementation, not multiple registry
bindings. For each start/resume, mfm-app extracts the distinct route keys required by pending
certified live nodes, selectively resolves exactly those config entries, constructs one public
endpoint-bound session per key, and stores them in one app-private routed session set. Bitcoin keys
are `semantic_source_identity`; each selected session still checks the requested network/chain.
EVM keys are `(network_id, source_ref)`. Duplicate/missing keys or a session whose checked binding
does not equal its map key fail before admission.

The routed set implements the same pure-domain session trait as one public transport session and is
passed as one Arc to live registration. Registration reads the implementation ID from that same Arc
and installs one capability binding; the executor uses the same Arc for every request. The adapter
therefore imports neither the app router nor concrete transport, while a library consumer may pass
one public session or its own trait implementation without app.

Replace brittle source-text assertions in crates/app/tests/transport_boundaries.rs with public API,
privacy, and behavior tests. Preserve the prohibition on a raw public JSON-RPC bypass.

### Portfolio live

mfm-portfolio-live owns SelectHoldingsState execution: issue a shared-snapshot fact query, read the
exact retained response artifacts selected by those facts, decode with pure-domain canonical
decoders, and produce portfolio query evidence.

Its registration API takes one store object:

    pub fn register_portfolio_live<S>(..., store: Arc<S>) -> Result<...>
    where
        S: FactQueryStore + RetainedArtifactReadProvider + 'static

Internally coerce clones of the same Arc to the two least-authority trait objects. Delete
PortfolioRunnerCapabilities. This prevents app assembly from accidentally supplying separate query
and artifact provider objects; retained artifact identity/bytes remain independently verified.

mfm-portfolio-live has no transport, concrete Postgres/in-memory implementation, generic provider
facade, generic JSON hydration layer, or copied Bitcoin/EVM response decoder.

## Shared execution contracts

### Pure states

mfm-runtime owns one generic PureStateRunner. It:

1. loads and validates certified config, input trees, and context;
2. constructs the typed state;
3. invokes PureState::run;
4. stages the one canonical output and any existing context output through RunnerOutputBuilder; and
5. returns the runtime-owned settlement.

The logical pure-runner factory ID remains exactly `pure` and is runtime-owned. Runtime exposes one
narrow helper with this shape (plus the required `StateSpec`/decode bounds):

    pub fn register_pure_state<S>(
        registrations: &mut RunnerRegistrationBuilder<'_>,
        factory: &RunnerFactoryBinding,
        artifacts: Arc<dyn RetainedArtifactReadProvider>,
        context_output: Option<Arc<dyn ContextOutputExtractor>>,
    ) -> Result<StateDescriptorIdentity>

The helper rejects a factory whose ID is not `pure`, constructs the private `PureStateRunner<S>`,
installs the optional context-output extractor, and registers it with the caller-supplied executable
identity. It never manufactures an executable identity from mfm-runtime.

MFM live-service assembly computes executable identity once before constructing production runner
registries. On Linux, a private app helper opens `/proc/self/exe` so it hashes the running inode
rather than reopening a raceable pathname. On macOS, it opens `std::env::current_exe()` (the
packaged path is an immutable Nix-store path) and rejects the read unless opened-file and path
device/inode/size metadata agree and remain unchanged through EOF. These are target-specific
implementations, not a try-one-then-fallback path. Stream bytes through SHA-256 with a fixed-size
buffer; do not load the executable into one allocation or log its path/bytes. Always perform this
blocking work through `spawn_blocking` before scheduler work starts. Render the raw SHA-256 as 64
lower-case hexadecimal characters, then build the persisted `binary_digest` from this exact
JCS-style canonical value and no other fields:

    {
      "contract": "mfm.executable-bytes.v1",
      "sha256": "<raw executable sha256>"
    }

Use `CanonicalValue`/`CanonicalJsonBytes::content_digest`; do not feed raw executable bytes directly
to a `Sha256JcsV1` `ContentDigest`, JSON-wrap bytes themselves, hash hexadecimal text a second time
outside the canonical object, or add a second digest-algorithm variant. Assembly failure to
identify/open/read/hash a stable executable, canonicalize its identity, or join the blocking task
is the stable redacted `ExecutableIdentityUnavailable` startup error.

Reduce `ExecutableIdentity` to exactly `{ factory_id, binary_digest }`. Delete
`cargo_package_digest`, `nix_derivation_hash`, `nix_output_hash`, and the unused public Nix hash
types/codecs/schemas. Rename `RunnerExecutableIdentityTemplate` to `ExecutableIdentityTemplate`,
with no alias, and give it one constructor taking the precomputed canonical executable-identity
`ContentDigest`; it preserves that digest verbatim for every factory. Make
`ErasedRunnerRegistry::new(template)` require and retain the template, with no zero-arg or default
constructor; registry/builders mint and validate every factory binding from it, and framework
resolution uses the same template. Delete runtime's `framework_executable` label hasher and every
domain-local template construction. A root passes the registry-minted `pure` binding unchanged to
the helper. A non-MFM library assembler supplies an explicitly attested template or checked
`RunnerFactoryBinding`; there is no runtime/domain default or fallback. Tests use explicit fixture
digests. Production constructors expose no digest-injection seam. Domain live adapters no longer
implement pure execution or replay.

Different executable bytes are intentionally different live execution authority. A run admitted by
CLI cannot be live-resumed by REST, and a rebuilt executable cannot resume an unfinished run
admitted by the previous build; byte-identical copies can. Read-only observation and evidence-only
replay do not hash or read the current executable: replay checks recorded identity consistency and
recomputes outputs without granting live execution authority. Add an integration test which admits
with one fixture digest and requires live resume under a second digest to fail with the existing
redacted executable-identity mismatch before live IO. Add a separate evidence-only replay test with
panicking executable-file access to prove no current-executable IO. Do not introduce a shared-label
compatibility mode.

mfm-replay owns one verify_pure_state implementation. It reconstructs config/input/context from
retained artifacts, calls the same state, and compares exact canonical output. It never resolves
current configuration or calls a live/provider interface.

The introducing commit migrates every surviving ordinary pure state and deletes all copied runner
and replay code.

### Fact-producing external reads

Change the existing ReadState contract rather than adding a new effect:

    pub trait ReadState: StateSpec<Effect = ReadExternal> {
        type Plan: MfmValue;
        type Evidence: MfmValue;
        type Facts: ReadFactBatch;

        fn plan(...) -> StateResult<Self::Plan>;

        fn reduce(
            &self,
            input: &Self::Input,
            evidence: &ExternalReadEvidenceSet<Self::Evidence>,
            context: &CertifiedContext<Self::Context>,
        ) -> StateResult<(Self::Output, Self::Facts)>;
    }

ReadFactBatch is a public-but-sealed trait in mfm-program because it appears in ReadState's public
associated bound. Its only operation has this fallible shape:

    fn fact_descriptor() -> mfm_facts::Result<Option<FactDescriptor>>;

It has exactly two framework implementations:

- () returns Ok(None) and means no emitted facts;
- NonEmpty<F>, where F implements MfmFactType, returns F::descriptor().map(Some) and means one
  homogeneous non-empty batch.

Runtime defines a private staging companion trait for those same two types; replay defines a private
comparison companion trait. Those private traits may iterate the concrete NonEmpty<F>. Do not add a
public visitor, erased prepared-fact value, NoFacts, MaybeFacts, heterogeneous batch, or extension
hook. MfmFactType moves from mfm-program to mfm-facts; derive output uses the new path.

ReadExternal's sealed EffectRunner implementation derives the emitted fact descriptor from
S::Facts. State registration installs that descriptor automatically. Delete manual
StateSpec::emitted_fact_descriptors hooks, public register_fact_type/register_fact_descriptor paths,
and domain after-registration callbacks. There is one descriptor authority.

Replace external_read_contract_digest contract version 1 with a private version 2 helper whose
generic inputs are Plan, Evidence, and Facts. It hashes this exact canonical object:

    {
      "contract_domain": "mfm.external_read",
      "contract_version": 2,
      "effect_class": "read_external",
      "plan_schema_id": <Plan schema ID>,
      "plan_semantic_id": <Plan semantic ID>,
      "evidence_schema_id": <Evidence schema ID>,
      "evidence_semantic_id": <Evidence semantic ID>,
      "fact_mode": {"kind":"none"}
    }

For NonEmpty<F>, fact_mode is instead:

    {"kind":"non_empty","fact_descriptor_hash":<canonical digest string>}

EffectRunner owns emitted_fact_descriptors, defaulting to empty. ReadExternal derives the one
descriptor from S::Facts, registration installs it, and launch artifact authority derives it from
registered states. There is no v1 digest function or alternate path.

On successful execution, the runner semantic payload order is:

1. zero or more FactRecorded payloads in reducer order;
2. one CellProduced payload for the output/receipt; and
3. one StateAttemptCompleted payload.

Primary and optional query evidence artifacts, fact response artifacts, staged artifact admissions,
derived ArtifactReferenced events, and retention records join those payloads in the same settlement
append. The read plan/request is certified input, not a retained evidence artifact. A failed
settlement append leaves the already-committed attempt-start prefix visible. On an uncertain append
result, runtime reloads and verifies the stream before any live IO; it either recognizes the
committed settlement or follows verified recovery authority. It never blindly repeats live IO.

Store admission enforces:

- any FactRecorded shares its commit with exactly one matching CellProduced and
  StateAttemptCompleted;
- fact-only prefixes and facts in failed, interrupted, skipped, side-effect, or run-level commits
  are invalid;
- every fact response is in the same settlement bundle as either PreparedArtifactBytes or an exact
  ExistingArtifactAdmission for bytes/evidence already retained; and
- FactKey is unique within that commit.

Runtime, certification, and history validation additionally enforce:

- only a certified external-read state whose ReadFactBatch is non-empty may emit facts;
- every fact descriptor equals the automatically certified descriptor;
- node, attempt, sequence, response artifact, and content identity bindings match;
- the batch is non-empty and homogeneous when facts are declared;
- a () read emits no fact event or fact artifact; and
- FactRecorded and ArtifactReferenced do not count as terminal attempt dispositions;
- returned FactKey values are unique within the batch and preserve reducer order; and
- an empty certified descriptor list has zero facts, while a non-empty list has at least one fact of
  its one certified descriptor.

Replay runs the same reducer over retained evidence and compares exact output plus fact count, order,
descriptor, subject, response, content identity, response artifact, node, attempt, and commit
binding. Missing, extra, duplicate, or reordered facts fail replay.

Repeated FactKey values across different attempts or runs remain valid; uniqueness is only within
one returned batch.

Delete all of the following rather than adapting them:

- ManagedWriteState;
- ManagedPlatformWrite and its runner/effect/role/certification/recovery/replay branches;
- any proposed ManagedFactWrite type or runner;
- FactRecordCapability and all adapter bindings/implementation IDs;
- RecordBtcAddressBalanceFactState and RecordEvmBalanceFactsState;
- FactRecordInput, RecordEvmBalanceFactsInput, and their handles/schemas;
- intermediate inputs/outputs used only between read and record states;
- RecordedFacts, RecordedFact, and same-attempt fact-prefix recovery; and
- RunnerOutputBuilder::record_fact, public RunnerFactRecorded, and the public
  RunnerEventPayload::FactRecorded authoring variant;
- StateSpec::emitted_fact_descriptors, public register_fact_type/register_fact_descriptor, and
  domain/app descriptor callbacks; and
- domain-specific managed-write replay helpers.

### Fact model and query ownership

mfm-facts owns:

- MfmFactType and fact descriptors;
- subjects, typed responses, claims, refs, content identity, and canonical hashing;
- query expressions, plans, ordering, results, receipts, and evidence; and
- the state-facing FactQueryReadCapability with name mfm.fact.query.read, kind
  (mfm.fact, query.read), identity
  sha256_digest_bytes(b"mfm.fact.capability:query.read"), and version
  mfm.fact.query.read.v1. Semantic compile/plan errors remain here; backend IO errors do not.

mfm-store owns:

- async FactQueryStore;
- batched single-snapshot/frontier semantics;
- one public backend-facing FactQueryProjection with private fields and narrow
  constructors/accessors for in-memory and Postgres projection/rebuild;
- RetainedArtifactReadProvider and VerifiedRetainedArtifactBytes;
- one free event-to-requirements derivation plus fact/config/cell/diagnostic/seed/output requirement
  constructors; and
- validation that retained bytes match the referenced event/artifact.

FactQueryStore has this object-safe shape, with no future alias, GAT, or async-trait method:

    type Error: std::error::Error + Send + Sync + 'static;

    fn fact_query_implementation_id(&self) -> &'static str;

    fn execute_fact_queries<'a>(
        &'a self,
        plans: &'a [CanonicalFactQueryPlan],
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<Vec<FactQueryResult>, Self::Error>,
                > + Send
                + 'a,
        >,
    >;

Results align one-for-one with plans. Empty input returns immediately without IO; non-empty input
uses one snapshot/frontier. Delete the old index-read identity, FactIndexReadRequest, provider
future aliases, and provider error families. Store backend errors are typed at their owner. The
private portfolio executor maps every S::Error to the constant redacted RuntimeError::Blocked so
the run follows the resume/retry path; it never formats backend diagnostics. Wrong result count,
frontier, ordering, or evidence maps to InvalidRunnerOutput.

PostgresStore and AsyncInMemoryRunStore implement FactQueryStore and
RetainedArtifactReadProvider directly. Their exact implementation identities are
mfm.storage.postgres.fact-query.v1 and mfm.store.memory.fact-query.v1. Do not preserve mfm.app
provider identities. Registration reads fact_query_implementation_id from the same Arc<S> used for
queries/artifacts and binds it as the FactQueryReadCapability CapabilityImplementationId. Catalog
comparison covers the semantic capability key, not this concrete ID. Registration accepts no
separate implementation-ID parameter, and permanent assembly validation checks that the installed
ID equals the value reported by that same Arc<S>.

Query compiler inputs and CanonicalFactQueryPlan contain no authored store scope, query scope, or
scope-decision evidence. Each receipt from a non-empty batch carries one StoreReadFrontier with the
actual store-owned StoreScopeId loaded from that same S and StoreCommitOrder. Every descriptor
admission and fact append advances that store-wide order, so the pair identifies the complete
append-only snapshot without a redundant descriptor-count watermark. Every aligned result carries
that exact same frontier; runtime and replay compare it exactly. Delete DescriptorCatalogWatermark,
StoreReadFrontierType, and both Prefix/Snapshot variants: FactQueryStore always returns one complete
snapshot, so the extra watermark and type tag are dead generality.

Prove that claim with barrier-controlled tests for both AsyncInMemoryRunStore and PostgresStore. In
separate cases, pause a multi-plan query after its snapshot/frontier is established, then commit (a)
a fact-producing append and (b) a descriptor-only admission. The query must observe either the
complete before state/frontier or complete after state/frontier, never mixed rows or aligned results
with different frontiers. Assert that descriptor-only admission advances StoreCommitOrder and that
rows/catalog authority at each returned order contains exactly the commits at or below that order.

Delete PostgresFactIndexReadProvider, ProjectionFactIndexProvider, app provider factories, and
mfm-fact-capabilities. Also delete PublicFactQueryExecutor, PublicFactQueryFuture,
PublicFactQueryExecution, and their aliases/factories. Domain-specific canonical response decoding
stays in mfm-bitcoin and mfm-evm.

Move EventArtifactRequirement, EventArtifactReferenceSource, and event-to-requirement derivation out
of mfm-events and into mfm-store; mfm-events retains event payload/reference facts only. Store also
owns run_artifact_requirement, artifact_referenced_artifact_requirement,
config_ref_artifact_requirement, seed_artifact_requirement, seed_cell_artifact_requirement,
public_output_cell_artifact_requirement, public_output_rendered_artifact_requirement,
diagnostic_artifact_requirement, fact_descriptor_artifact_requirement, and
fact_response_artifact_requirement. Runtime-private side-effect/staged requirement derivation stays
in runtime because it binds attempt-local runner artifacts and capability roles; it constructs the
store-owned requirement and is not a retained-read/provider facade. Delete
KernelEventPayload::artifact_requirements, the empty RuntimeArtifactStore marker, and every duplicate
artifact request/provider/future/error/evidence type. Rename VerifiedRunArtifactBytes to
VerifiedRetainedArtifactBytes with no alias.

VerifiedRetainedArtifactBytes validates artifact ID, evidence hash, digest against actual bytes,
length, media/schema/semantic IDs, producer, and role. Requirement construction and all of its tests
move from mfm-events to mfm-store.

Rename index-facing public vocabulary to query-facing vocabulary where it describes the contract,
not the projection implementation. Do the rename once; leave no old aliases.

Apply the same-store assembly rule across app services, not only portfolio registration. Replace
RunServices<S, A> and public fact query/hydration entry points that accept a separate artifact
provider with one shared Arc<S> carrying the required store plus retained-artifact traits.
Adversarial tests wrap one store and override its artifact method rather than injecting a second
provider object.

Final pure-domain retained fact-response decoders first require canonical JSON bytes, then strictly
decode the exact typed response with unknown-field/trailing-data rejection and domain validation.
They do not rederive selection identity without its external reference. Portfolio separately
rederives descriptor, subject, response content identity, and artifact evidence against the
selected fact ref. This closed retained-response contract is distinct from additive-field Bitcoin
Core JSON-RPC decoding. Cover noncanonical bytes, unknown fields, wrong schema, trailing data,
tamper, and swapped responses.

### Fact schema simplification

After managed writers and checkpoint facts are deleted, every production fact is an indexed
platform fact emitted from external-read evidence. Delete unused generality:

- FactProducerProvenance and producer fields;
- request and observed_at claim/ref/query fields;
- FactVisibility, FactAudience, FactVisibilityScope, and FactQueryScope;
- Control, RunPrivate, and default-scope branches;
- StoreScopeRef and ScopeDecisionEvidence (the former does not bind the real StoreScopeId and both
  are constant/ignored after the platform-only query cut);
- FactMetadataField::ObservedAt, FactExtractionMetadata.observed_at, derive metadata support, and
  their SQL terms/constraints; and
- FactRecordProjection/map and FactIndexProjection, merging useful fields into the one
  backend-facing, field-private FactQueryProjection built from FactRecorded and retained bytes;
- projection columns, query parameters, codecs, public DTO fields, filters, tests, and fixtures for
  those concepts.

FactRecorded already binds run, node, attempt, sequence, ordinal, descriptor, subject, response,
content identity, and response artifact. recorded_at remains store-envelope authority. Observation
time/source semantics belong in typed protocol evidence/response types. The real StoreScopeId stays
in run/store authority and does not need a second authored label in each fact query.

The public fact API returns the descriptor-approved platform catalog and contains no visibility
fixture or audience input. A future private/control fact requires a new concrete access-control
design; do not preserve speculative branches now.

This is a persisted authority cut. Update canonical codecs, content identities, schemas,
projections, SQLx metadata, replay fixtures, REST/CLI JSON, and docs together. Changed hashes and IDs
are accepted. Reset the one current Postgres baseline/authority fingerprint; old stores fail one
redacted incompatible-store/schema error and must be recreated.

The cut's replacement ledger includes the external-read contract digest, affected state/spec/
certificate hashes, FactRecorded schema, fact claim/ref codecs, query compiler/plan/frontier/
receipt/evidence identities, fact-kernel contract identity, REST/CLI schemas, Postgres catalog and
baseline checksum, SQLx metadata, and store authority mfm.postgres.store.v4. Use exactly
mfm.facts.v2, mfm.facts.query.v3, mfm.fact-query.v3, mfm.fact-query-plan.v2,
mfm.fact-query-receipt.v3, mfm.fact-query-result-set.v2, and mfm.fact-query-evidence.v3. Keep
mfm.fact-query-selected-summaries.v1 because its canonical fields do not change. Keep the
fact-content-identity, fact-key, fact-claim-id, and subject namespace/material versions because
their components do not change. Keep every `mfm.events.v1.*` schema name, set the one shared
`EVENT_SCHEMA_VERSION` to `2`, and regenerate all event schema hashes/goldens. This deliberately
uses one event-schema epoch for the changed FactRecorded and RunAdmitted/ExecutableIdentity shapes
instead of adding per-event version branches. Map both connection authority mismatch and SQLx migration-checksum
mismatch to one public IncompatibleStoreSchema code. Keep their internal typed causes redacted.
Later content-addressed
domain schema changes do not need another physical store epoch; old run material fails ordinary
certification.

## Bitcoin implementation contract

### Domain input and authority

BitcoinBalanceCollectionConfig contains only semantic demand:

- network_id;
- bitcoin_network;
- semantic_source_identity; and
- addresses.

Each batch key is exactly (network_id, bitcoin_network, semantic_source_identity), and its addresses
contain 1 through 1,024 strictly sorted entries. Add rust-bitcoin as the Bitcoin primitive authority.
Parse Address<NetworkUnchecked>, require encoding compatibility with the configured network, require
canonical rendering, derive script_pubkey, and reject duplicates by canonical address and script.
The one ordering comparator is lexicographic order of canonical rendered address UTF-8 bytes; use it
for request validation, scan descriptors, evidence, reduction, facts, and receipt entries. The later
getblockchaininfo chain check, not an address encoding, proves the actual chain.

Support main, test, testnet4, signet, and regtest chain tags. Delete the portfolio-owned
bech32/bs58 validators, the weaker Bitcoin address-envelope validator, and direct dependencies made
unnecessary by rust-bitcoin. New MFM-owned types use Bitcoin in names; BTC remains only for units or
protocol terminology.

There is no authored read count, retry count, status, coverage, or constant output field. Limits are
derived from the validated request and explicit supported resource ceilings.

The aggregate read uses one pure-domain `BitcoinBalanceCollectionReadCapability` and one
route-aware `BitcoinBalanceSession` trait. Pin the capability contract exactly:

- name: `mfm.bitcoin.balance_collection.read`;
- kind namespace/name: `(mfm.bitcoin, balance_collection.read)`;
- kind digest: `sha256_digest_bytes(b"mfm.bitcoin.capability:balance_collection.read")`;
- version: `mfm.bitcoin.balance_collection.read.v1`; and
- JSON-RPC implementation ID: `mfm.bitcoin.jsonrpc.balance_collection.v1`.

Both one public endpoint-bound `BitcoinRpcSession` and the app-private routed session set implement
that trait and report the exact implementation ID. The routed set selects by the certified
`semantic_source_identity`, verifies the selected session binding, and is registered once for the
capability. Old chain-head/balance capabilities and `mfm.bitcoin.jsonrpc.runtime.v1` are deleted,
not aliased.

Pin the new certified/domain identities exactly:

- operation kind `(mfm.bitcoin, balance_collection)`, kind digest
  `sha256_digest_bytes(b"mfm.bitcoin.operation:balance_collection")`, public name
  `mfm.bitcoin.balance_collection`, and version
  `mfm.bitcoin.operation.balance_collection.v1`;
- state kind `(mfm.bitcoin, collect_balances)`, kind digest
  `sha256_digest_bytes(b"mfm.bitcoin.state:collect_balances")`, public name
  `mfm.bitcoin.collect_balances`, and version `mfm.bitcoin.state.collect_balances.v1`;
- adapter kind `(mfm.bitcoin, jsonrpc)` and its existing kind digest
  `sha256_digest_bytes(b"mfm.bitcoin.adapter:jsonrpc")`, but replacement behavior version
  `mfm.bitcoin.jsonrpc.adapter.v2`; and
- fact kind `bitcoin.balance_snapshot`.

The `BitcoinBalanceSnapshotSubject`, `BitcoinBalanceSnapshotResponse`, and
`BitcoinBalanceSnapshotFact` schema names are respectively
`mfm.bitcoin.fact.balance_snapshot.subject`, `mfm.bitcoin.fact.balance_snapshot.response`, and
`mfm.bitcoin.fact.balance_snapshot`, each at semantic version 1. The subject contains only
network_id, bitcoin_network, semantic_source_identity, and canonical address; the response contains
only anchor_height, anchor_hash, and balance_sats.

Pin every other new persisted Bitcoin value to semantic version 1 with no alternate spelling:

- `BitcoinBalanceCollectionConfig`: `mfm.bitcoin.balance_collection.config`;
- external-read plan: `mfm.bitcoin.balance_collection.plan`;
- primary evidence: `mfm.bitcoin.balance_collection.evidence`;
- minimal collection receipt: `mfm.bitcoin.balance_collection.receipt`; and
- operation output: `mfm.bitcoin.operation_outputs.balance_collection`.

EVM retains its one `EvmReadCapability`: name `mfm.evm.read`, kind `(mfm.evm, read)`, digest
`sha256_digest_bytes(b"mfm.evm.capability:read")`, version `mfm.evm.read.v1`, and implementation ID
`mfm.evm.jsonrpc.session.v1`. Its app-private routed set selects a public endpoint-bound
`EvmRpcSession` by `(network_id, source_ref)` and is likewise the single registered implementation;
reusable validation/transaction APIs remain unregistered in production. Keep adapter kind
`(mfm.evm, jsonrpc)` with digest `sha256_digest_bytes(b"mfm.evm.adapter:jsonrpc")` and replace the
behavior version with `mfm.evm.jsonrpc.adapter.v2`. Keep portfolio adapter kind
`(mfm.portfolio, typed-portfolio)` with digest
`sha256_digest_bytes(b"mfm.portfolio.adapter:typed-portfolio")` and replace its behavior version
with `mfm.portfolio.adapter.typed.v2`. Delete all three adapter-v1 identities; do not register or
decode them beside v2.

### One live attempt

The final graph is:

    BitcoinBalanceCollectionOperation
      -> CollectBitcoinBalancesState

A successful attempt performs exactly three logical calls:

1. getblockchaininfo: require the selected chain and initialblockdownload == false;
2. one scantxoutset start request containing every sorted addr(address) descriptor; and
3. getblockhash(scan.height): require the hash to equal the scan anchor at that height.

Tip advancement after the scan is valid. A changed hash at scan.height means reorganization and
fails the attempt. Do not call getblockheader, scantxoutset status, or scantxoutset abort.

There is no local scan coordinator. It cannot enforce the Bitcoin Core global scan exclusion across
processes and would advertise a false guarantee. A scan-busy response is a redacted retriable
operational error. After a proven uncommitted attempt, runtime retry repeats the entire read; an
uncertain settlement first reloads history. Transport performs no hidden automatic retry.

Transport uses a private 10-second connect timeout and 60-second overall timeout for
getblockchaininfo/getblockhash. The selected Bitcoin route must provide scan_timeout_seconds in
1..=86,400; BitcoinRpcSession construction accepts the resolved Duration and rejects values outside
that range. That duration is the overall scantxoutset start deadline. Timeout is a redacted
retriable operational error. Cancellation or timeout drops MFM's HTTP request but does not abort
Core's global scan; a later retry may receive scan-busy until Core completes. MFM still never calls
status or abort.

Endpoint validation accepts HTTP(S) with a host and rejects URL userinfo, query, and fragment.
Authentication is a separate resolved secret. The client disables redirects and ambient/system
proxy discovery. Any 3xx response is a bounded redacted non-200 failure; it does not produce an
extra request or forward authentication to another endpoint.

MAX_BITCOIN_JSON_RPC_BODY_BYTES is a private 16 MiB supported ceiling for both success and error
bodies. Reject an oversized Content-Length before allocation and cap chunked accumulation at
limit + 1 before deserialization. Never use an unbounded response text/bytes path.

### Strict response reduction

Use one decoder for the common Bitcoin Core 28+ fields MFM consumes. Core 28 is the minimum because
it is the first release with both JSON-RPC 2.0 and testnet4. Validate known fields and
semantics strictly; ignore additive unknown object fields. Do not inspect a Core version, negotiate a
decoder, retry with another request shape, or keep alternate schema paths.

The bounded JSON parser rejects a duplicate member at every object nesting level before typed
decoding; additive unknown members are allowed only when unique. Do not deserialize first into
serde_json::Value or another last-wins map.

Every request includes jsonrpc = "2.0". A JSON-RPC success or error envelope must arrive with HTTP
200 and contain jsonrpc = "2.0", exact request-ID equality, and exactly one of result or error.
Non-200 responses are bounded redacted HTTP transport failures and are never decoded through a
legacy JSON-RPC error shape. Error code must be an integer and message a bounded string; data is
never rendered. Classify scan busy only when code is -8 and the message starts exactly with Scan
already in progress; other -8 errors are non-retriable request/protocol failures. Do not accept the
pre-28 JSON-RPC 1.1 envelope.

Require success == true and mandatory height, bestblock, txouts, unspents, and total_amount fields.
Require txouts to be a nonnegative integer fitting u64; it is validated but not used as balance or
match-count authority. Parse bestblock and transaction IDs with rust-bitcoin. Decode bounded script
hex. Require vout to fit u32, reject duplicate (txid, vout), and require every matched UTXO height to
be no greater than scan height.

Map each returned UTXO by exact scriptPubKey bytes derived from the requested checked addresses.
Reject malformed, unknown, or ambiguous scripts. Require desc to be a bounded string because it is a
known Core field, but do not interpret it or use it for authority; exact script bytes are sufficient.

Parse raw JSON amount tokens directly to integer satoshis with checked decimal conversion, at most
eight fractional digits, no exponent, no sign, no f32/f64 intermediary, and no value above Bitcoin
MAX_MONEY (21,000,000 BTC). Convert the result to rust-bitcoin Amount. Preseed zero balances for
addresses with no UTXO, checked-add balances in requested-address order, require every subtotal and
the total to stay within MAX_MONEY, and require the sum to equal total_amount. txouts is the number
scanned, not the number matched, and is not compared with the result count.

Primary evidence contains one redacted source identity, the shared scan anchor, the ordered
address/satoshi vector, and final canonicality observation. The reducer rejects missing, extra,
reordered, duplicate, wrong-source, or mismatched-anchor evidence and returns:

- one BitcoinBalanceSnapshotFact per address in canonical order; and
- one minimal collection receipt containing source/network binding, shared anchor, address keys,
  and aligned fact content identities.

Do not repeat fact payloads, anchors, counts, status, or coverage in the receipt.

Required fixtures:

- a Bitcoin Core 28-shaped response;
- a later/additive response including currently known additional fields;
- main, test, testnet4, signet, and regtest source/address cases;
- shared test-family address encodings which are accepted for compatible networks and then bound by
  the RPC chain tag;
- an exact canonical-address UTF-8 ordering fixture, including shared test-family encodings;
- zero balance, multiple UTXOs, duplicate outpoint, unknown script, excessive decimals, exponent,
  negative amount, over-MAX_MONEY, overflow, total mismatch, missing/malformed/negative txouts,
  wrong chain, IBD, reorg, exact and
  near-miss scan-busy errors, JSON-RPC version/ID/result-error violations, oversized Content-Length,
  oversized chunked body, duplicate envelope members, duplicate nested bestblock/scriptPubKey/
  amount members, and redaction;
- a raw-envelope request assertion proving exactly one multi-descriptor scan; and
- live parity against the repository-pinned Bitcoin Core service before final closure.

The dedicated parity commit adds a Nixfied regtest Bitcoin Core service from the root pinned
nixpkgs, records/asserts its exact bitcoind version (which must be at least 28), creates deterministic
RPC credentials and isolated data, mines a fixture UTXO, and adds a parity task for the checked batch
scan. Add that task to final CI and update docs/build-and-verification.md. The flake lock is the
version pin; do not rely on a host bitcoind.

Use the official contracts as test references:

- https://github.com/bitcoin/bitcoin/blob/v28.0/doc/JSON-RPC-interface.md
- https://github.com/bitcoin/bitcoin/blob/v28.0/doc/release-notes.md
- https://bitcoincore.org/en/doc/28.0.0/rpc/blockchain/scantxoutset/
- https://bitcoincore.org/en/doc/28.0.0/rpc/blockchain/getblockchaininfo/
- https://bitcoincore.org/en/doc/28.0.0/rpc/blockchain/getblockhash/
- https://bitcoincore.org/en/doc/31.0.0/rpc/blockchain/scantxoutset/
- https://bitcoincore.org/en/doc/31.0.0/rpc/blockchain/getblockchaininfo/

## Production authoring-catalog contract

mfm.portfolio/snapshot@2 is the only final published run entry point. Commit 7 deletes
mfm.portfolio/snapshot@1 when the public snapshot and graph contract change; there is no alias or
dual registration.

The portfolio operation declares a sealed authoring catalog, independently from app registry
construction, containing its allowed child operations, states, capability descriptors, and emitted
facts. It contains no process/build executable, configured route, store implementation, or live
adapter implementation ID. This is the production semantic universe; it is not described as static
proof that every branch is semantically reachable. The catalog and typed registries come from the
same macro declaration; authoring rejects a child absent from those registries. Canonical branch
tests then require their observed union to cover every declared domain item.

Extend `define_program_descriptor_registry!` once with an `authoring_catalog` function and require
each included child registry to name its generated child-catalog function beside its existing
state/operation/certification functions. The macro's existing `states`, `operations`, and `includes`
lists are the single declaration source; do not add parallel capability/fact arrays. For the final
portfolio declaration it generates:

    pub fn portfolio_snapshot_authoring_catalog()
        -> mfm_certify::Result<ProgramAuthoringCatalog>

`ProgramAuthoringCatalog` is the one public cross-crate catalog type, owned by mfm-certify, with
private fields and read-only validation/union behavior. The macro deterministically builds these
sorted, duplicate-rejecting sets:

- full operation descriptors keyed by DescriptorId;
- full state descriptors keyed by DescriptorId;
- capability descriptors derived from those states and keyed by (CapabilityKind,
  CapabilityVersion);
- emitted FactDescriptorRef values derived from those states and keyed by descriptor digest;
- AdapterBindingSpec values returned by those state types and keyed by (AdapterKind,
  AdapterVersion); and
- the subset of state keys whose effect is ApplySideEffect.

Included child catalogs are unioned with conflict rejection. Emitted facts come only from the
automatic EffectRunner/ReadFactBatch descriptor path established in commit 7. The builder/insert
surface used by macro expansion is doc-hidden; do not add a domain-specific catalog type or a
second app-owned expected-set structure.

The same macro declaration continues to generate typed authoring and certification registries.
OperationExpansion can plan only types in those generated registries, so an operation cannot emit
an operation/state key absent from the catalog generated from the same lists. Add a negative fixture
whose operation body tries to plan an undeclared child and require authoring to reject it before a
draft is produced. This proves graph possibilities are a subset of the catalog.
Canonical Bitcoin-only, EVM-only, and mixed expansions record normal typed operation lineage; the
union of their observed operation/state descriptor IDs must equal the catalog's declared domain
sets. This demonstrates that every declared item has a valid published branch without defining the
catalog from sample reachability.

`mfm-runtime` exposes one sealed `framework_authoring_catalog()` built from its closed framework
descriptor list. App unions it with `portfolio_snapshot_authoring_catalog()` and passes that one
expected catalog to narrow validation methods on CertificationRegistry and ErasedRunnerRegistry;
the app-owned replay registry validates itself against the same value. Require semantic-key
equality for:

- operation and state descriptors installed in certification;
- emitted fact descriptors and artifacts;
- state keys in runner and executable bindings;
- capability keys in capability and side-effect-verification bindings; and
- state/intent keys in replay verifier coverage.

Every declared state has exactly one runner/executable binding. Every declared capability has
exactly one implementation binding in a process. Concrete implementation IDs and process/build
provenance are not pure-catalog values: assembly obtains an ID from the same selected store/session
object used for execution and validates the binding separately. Configured routes are instances,
not new catalog identities. No extra domain registration is allowed.

There is no production ApplySideEffect state after disconnected EVM transaction registration is
removed, so the final side-effect-verifier and replay-intent key sets are both exactly empty. Do not
invent catalog metadata for an unused future side-effect mapping.

The framework bootstrap is one crate-private function in mfm-runtime backed by a closed list of
built-in effect/runtime descriptors. It cannot call a domain registration function or contain a
package carrying domain metadata.

Keep BTC-only, EVM-only, and mixed valid draft tests as deterministic behavioral smoke tests.
Reusable EVM exact-anchor validation and transaction submission have explicit narrow library
registration functions. Production balance-only assembly does not create their signer/session or
register them.

## Runtime configuration contract

mfm-app privately owns document loading, parsing, selection, value resolution, and public-safe error
mapping. Live crates accept resolved typed endpoint/auth/signer inputs and do not open files or read
environment variables.

The config document path has one source: an explicit path argument passed into mfm-app. CLI and the
REST server expose `--runtime-config <PATH>` and pass it through; there is no
`MFM_RUNTIME_CONFIG_FILE`, environment fallback, or precedence rule. Absence is allowed for
read-only/evidence-only work and becomes RuntimeConfigRequired only when a pending live node or
explicit keystore/signer operation requires configuration. ValueSource may still deliberately read
an environment variable named inside the selected document; that is not config-path selection.

The final top-level sections are bitcoin, evm, signers, and keystores. Delete btc and every old field
spelling. The exact selected-entry schema is:

    [bitcoin.routes.<semantic_source_identity>]
    rpc_url = <ValueSource>
    rpc_user = <optional ValueSource>
    rpc_password = <optional non-direct ValueSource>
    scan_timeout_seconds = <integer 1..=86400>

    [evm.routes.<network_id>]
    source_ref = <LocalPublicId string>
    rpc_url = <ValueSource>
    auth_header = <optional non-direct ValueSource>

    [keystores.<keystore_ref>]
    keystore_path = <ValueSource>
    unlock_file_path = <ValueSource>

    [signers.<signer_ref>]
    provider = keystore
    keystore_ref = <keystore_ref string>
    entry_id = <UUID string>

Bitcoin basic-auth fields are either both absent or both present. Runtime network/chain and EVM
expected chain IDs stay in certified domain demand, not process configuration. A keystore profile
stores paths only; unlock secret bytes never appear in the document. JSON uses the identical object
keys/nesting. Every selected object rejects unknown fields.

The parser:

- reads at most 1 MiB using a limit + 1 strategy on a blocking worker;
- parses the complete JSON or TOML syntax;
- detects duplicate JSON keys at every nesting level;
- rejects unknown top-level keys;
- applies the closed secret-field policy below even inside unselected entries;
- keeps entry bodies raw until selection; and
- strictly deserializes only the selected entry set and each selected signer's referenced keystore.

An unrelated syntactically valid but schema-invalid entry does not block the selected path. A
syntactically invalid document always fails. Selected structures use deny-unknown-fields behavior.

ValueSource uses one exact-one-key shape:

    { direct = value }
    { env = variable_name }
    { file = path }
    { file_env = variable_name_containing_path }

There are no sibling option fields or precedence rules. The only reviewed secret slots are
bitcoin.routes.*.rpc_password and evm.routes.*.auth_header; they accept env, file, or file_env and
reject direct even when unselected. Normalize field names with ASCII lowercase and hyphen-to-
underscore. Outside those exact paths, reject a key containing any exact marker from this closed
list: password, passphrase, mnemonic, seed_phrase, private_key, secret, credential, api_key,
access_key, token, authorization, signed_material,
signed_transaction, raw_transaction, raw_tx, or signature_scalar. Continue to reject
expected_chain_id anywhere. Unselected entries otherwise receive no schema/semantic validation.

Format is selected solely by a case-sensitive .toml or .json config-path extension. The document,
direct/env values, indirection-file content, and paths encoded inside the document must be UTF-8;
non-Unicode environment values fail closed. RuntimeEnvName retains its 256-byte uppercase/digit/
underscore grammar. Config and indirection paths are non-empty and at most 4,096 UTF-8 bytes.

The document is capped at 1 MiB. Only selected values are resolved and subjected to the 64 KiB
value/file-content cap; unselected direct strings are bounded by the document cap. Direct and env
values are preserved byte-for-byte and must be non-empty. File/file_env contents have at most one
terminal LF or CRLF removed and all other whitespace is preserved; the result must be non-empty.
file_env first resolves a bounded path value and then performs a bounded blocking read. Every read
uses limit + 1. Errors redact config/indirection paths, environment names, and values.

Secret-slot resolution owns protected storage on every path. Allocate file-read buffers as
Zeroizing<Vec<u8>> before the first read, retain limit + 1 bytes so oversize input is rejected rather
than truncated, and let every early error drop that buffer. Move environment results into protected
bytes immediately after acquisition. Validate UTF-8 by borrow, copy once directly into a
Zeroizing<String>, then drop the protected byte buffer; never create an ordinary secret String.
Cover success, empty, invalid UTF-8, limit + 1, and injected partial-read failure with an
instrumented zeroize/drop witness.

Each public live transport owns its resolved endpoint/auth input. Secret fields wrap
Zeroizing<String>, have consuming constructors, no Clone/Copy/Debug/Display/Serialize/Deserialize or
borrowed public accessor, and move into an Arc-backed session so session clones do not copy secrets.
App resolves a selected secret directly into that input and retains no second copy. Endpoint and
public source IDs use separate non-secret checked types.

Evidence-only commands and replay do not open the runtime document.

## Keystore and binary contract

In one commit, consolidate the two keystore crates and delete dangerous-secret-export,
allow_secret_exports, export_private_key, related config/feature/audit/error/tests, and the
cross-crate get_private_key/SecureKey bridge. Raw key retrieval and SecureKey become crate-private in
that same cut. Public mfm-keystore supports import, list, metadata, delete, and signing only.

The binary-to-app secret boundary is one type:

    pub struct SecretInput(Zeroizing<String>);

It has no Clone, Copy, Debug, Display, Serialize, Deserialize, or string-reference accessor. Its
constructor consumes a String. The app service consumes SecretInput and moves it into spawn_blocking
keystore work. No secret is retained in service state, error details, or output.

Runtime SigningProvider is repeatable but does not retain an unlock value: it owns redacted checked
keystore/unlock-file paths. Each sign call allocates a Zeroizing<Vec<u8>> before reading, reads at
most 64 KiB plus one byte on its blocking worker, rejects oversize rather than truncating, removes at
most one terminal LF/CRLF, and rejects empty/non-UTF-8 content. It validates by borrow and copies
directly into a Zeroizing<String>; no ordinary secret String or Vec exists on success or any error
path. It opens/unlocks/signs and drops both protected buffers before returning. CRUD/import unlock
values are one-shot SecretInput values. Instrumented tests cover zeroization on success, empty,
invalid UTF-8, limit + 1, and injected partial-read error.

The REST API exposes no secret-bearing keystore/config ingress. CLI password/stdin readers construct
SecretInput directly into zeroizing storage; request/support types containing secret input cannot
derive Clone, Debug, Display, Serialize, or Deserialize, and parse-error buffers are zeroized.

CLI and REST:

- parse transport-level input;
- call mfm-app services;
- render stable current output;
- do not construct live sessions, registries, Postgres stores, runtime configuration, signers, or
  keystores;
- do not depend on runtime, replay, live, concrete storage, keystore, signing implementation, or
  pure-domain execution crates; and
- may use mfm-ids/mfm-canonical only for genuine presentation values.

Do not create app DTOs or traits that merely copy a lower safe value type. During final review,
delete any one-to-one facade introduced solely to satisfy dependency tests.
Application DTOs are justified when they aggregate an application command/result or redact lower
authority. A wrapper around one safe scalar with identical invariants is a prohibited mirror.

## Architecture enforcement

Package metadata describes semantics from commit 1 onward:

    [package.metadata.mfm]
    layer = "domain"
    domain = "bitcoin"
    domain-role = "source"

Layers are kernel, domain, live, signing, secret-provider, storage, assembly, binary, and test.
Domain is a validated identifier, not a hardcoded list. Every domain package requires domain-role =
source or aggregate; all packages for one domain must agree. Live packages require domain and derive
source/aggregate from the matching pure-domain metadata. Kernel crates require domain-facing.
Kernel and assembly crates require binary-facing.

Cargo target kinds constrain rather than trust that metadata: every non-test workspace package with
a binary target must declare layer = binary. A mixed library/binary package is allowed only as layer
binary and its entire normal/build dependency set must satisfy the binary row. A proc-macro target
must be a dedicated non-binary package and retain the proc-macro boundary. Add positive and negative
synthetic fixtures for a mislabeled bin target, a compliant mixed binary package, and a mixed package
whose library dependency would evade the binary row.

Final domain-facing kernel packages are canonical, capabilities, certify, facts, ids, program,
program-derive, and values. Final platform-only kernel packages are events, manual-auth, replay,
runtime, spec, and store. Only app, canonical, and ids are binary-facing. The old effects and
fact-capabilities packages are honestly domain-facing until deleted at their semantic cut; this is
not an exception to dependency validation. Spec is platform-only from commit 1.

Validate normal/build internal edges with this complete matrix:

- kernel -> kernel;
- signing -> domain-facing kernel;
- source domain -> domain-facing kernel, signing, and same-domain domain packages;
- aggregate domain -> domain-facing kernel, signing, same-domain domain packages, and source-domain
  packages;
- source live -> kernel, signing, its source-domain packages, and same-domain live packages;
- aggregate live -> kernel, signing, its aggregate-domain packages, source-domain packages, and
  same-domain live packages, never source live;
- secret-provider -> domain-facing kernel, signing, and same-layer secret-provider packages;
- storage -> kernel;
- assembly -> lower layers and assembly support;
- binary -> binary-facing kernel or binary-facing assembly only; and
- test -> unrestricted.

The same-domain package allowances let the current split graph and a future evidence-backed split
use one permanent rule; they are not named exceptions. Enforce source-domain -> aggregate-domain,
cross-domain live, live -> concrete storage/secret/app, signing -> secret-provider, and store ->
storage prohibitions. A live package's domain must have at least one matching pure package.

The metadata validator checks key presence/types and dependency edges, but never exact package
names, directory suffixes, or counts. Synthetic fixtures cover every forbidden edge and positive
aggregate composition.

Add source/API guardrails:

- In concentrated pure-domain crates, State implementations live only under state modules.
- In concentrated pure-domain crates, Operation implementations live only under operation modules.
- Lower pure modules do not import higher modules under the exact role order above.
- Pure source has no IO/runtime/replay/store/app/keystore/concrete-storage identifiers.
- Source live crates have public transport and private adapter modules.
- Transport source has no runtime/replay/store/adapter/runner identifiers.
- Adapter is the only live module allowed to name runner/replay registration, and it does not import
  the concrete transport.
- Portfolio live has no transport module, concrete store, or source-live dependency.
- Binaries have no prohibited dependencies and no implementation construction.

Source scans are targeted guardrails, not a Rust parser substitute. Pair them with:

- external compile/use tests for the public typed transport;
- compile-fail/privacy tests for the private adapter module and prohibited public rpc_call escape
  hatch, without naming arbitrary private implementation types;
- fake-session adapter tests;
- Cargo metadata dependency fixtures; and
- production authoring-catalog equality tests.

Do not add a broad custom static analyzer or a new dependency solely for scans.

## Commit plan

Each commit leaves one buildable current design. Update affected rustdoc, README files,
docs/architecture.md, docs/design.md, schemas, fixtures, generated metadata, and Cargo.lock in the
same commit. A replacement deletes its old package/type/path in the same cut; no shell, re-export
crate, compatibility reader, or dual registration survives.

The order is semantic-first: delete unused product surfaces, establish platform authority and the
single execution paths, replace Bitcoin behavior vertically, then move final code into concentrated
crates. This avoids polishing or relocating code that a later commit deletes.

### 1. replace topology snapshots with semantic boundary contracts

Changes:

- Replace category metadata in every current manifest with the final metadata vocabulary. Domain
  packages declare domain and domain-role; kernel packages declare domain-facing; kernel and
  assembly packages declare binary-facing.
- Classify current split packages by responsibility, not path. The current Bitcoin/EVM/proof
  packages are source domain/live, portfolio is aggregate domain/live, app/runtime-config are
  assembly, and mfm_core/mfm-signers-keystore are secret-provider.
- Mark current mfm-effects and mfm-fact-capabilities domain-facing; mark mfm-spec and
  mfm-artifact-capabilities platform-only. Mark only mfm-app, mfm-canonical, and mfm-ids
  binary-facing; mfm-runtime-config is assembly but not binary-facing.
- Replace exact inventories, path inference, named overrides, and category exceptions with metadata
  shape validation and the semantic dependency evaluator.
- Add synthetic positive/negative fixtures for every final matrix row. Apply workspace enforcement
  immediately to permanent rules the current graph satisfies, then add remaining rules
  monotonically at their cutovers. Do not encode debt with a phase field, disabled mode, package
  allowlist, or expected-violation list.
- Derive bin/proc-macro target coherence from Cargo metadata so a package cannot self-label around
  the binary row; cover mislabeled and mixed-target fixtures.
- Retain proc-macro separation and durable source checks that do not depend on package names/counts.

Focused verification:

- Run only the Nixfied cargo-metadata-contract leaf; it already invokes the focused target in the
  pinned environment.
- Run git diff --check.

Exit condition: every package has honest semantic metadata, all forbidden synthetic edges fail, and
a valid crate rename/count change needs no test edit.

### 2. delete unused proof workflow

Changes:

- Delete mfm-collectors-proof, mfm-op-proof, and mfm-transports-proof.
- Delete proof certification, runtime/replay registration, app/CLI wiring, manifests, docs,
  fixtures, features, IDs, and Cargo.lock entries.
- Delete define_program_descriptor_registry!'s now-unused after_registration arm together with the
  proof-only manual-authority callback; do not retain a generic post-registration hook.
- Preserve genuinely unique kernel coverage only as a minimal test-local state/operation; do not
  recreate a proof framework or production descriptor.
- Add and retain the domain-to-platform-only-kernel prohibition after deleting proof's invalid
  mfm-spec dependency.

Focused verification:

- Check mfm-app, mfm-runtime, mfm-replay, and mfm-integration-tests.
- Run focused replay/certification tests formerly reached through proof.
- Run the cargo-metadata-contract leaf.

Exit condition: no production proof package, ID, command, registry branch, or current document
remains.

### 3. delete bitcoin checkpoint and standalone collector

Changes:

- Delete checkpoint query/record states, CollectorCheckpointFact, standalone chain-head
  fact/publication, BtcChainHeadCollectorCycleOperation, drafts, launch helpers, receipt assembly,
  and production registration.
- Keep the active portfolio balance path, including its shared coverage/status fields, buildable
  until commit 7 replaces its facts, selection, and public snapshot atomically. Do not relocate or
  polish those doomed fields.
- Keep only chain/source evidence internally required by that active collection until commit 7
  replaces the entire path.
- Delete app, CLI, REST, schema, and fixture surfaces reachable only from the standalone cycle.

Focused verification:

- Check/test current Bitcoin state, operation, adapter, app, and affected binary packages.
- Run Bitcoin portfolio-snapshot, certification, replay, and cargo-metadata-contract tests.

Exit condition: there is no checkpoint/standalone Bitcoin entry point; the sole remaining temporary
Bitcoin-to-portfolio status dependency is explicitly deleted in commit 7.

### 4. delete evm standalone collector wrapper

Changes:

- Delete the standalone EVM balance-cycle draft/launch wrapper and app/CLI/REST registration.
- Retain balance collection because portfolio snapshot uses it.
- Retain exact-anchor validation and transaction submission as narrow library/test foundations,
  never automatic production registration.
- Split the current bundled EVM runner registration into separate balance, validation, and
  transaction functions now. Production invokes balance registration only.
- Delete wrapper-only schemas, IDs, outputs, documents, and fixtures.

Focused verification:

- Check/test current EVM state, operation, adapter, and app packages.
- Run EVM balance, validation, transaction, portfolio snapshot, and registration-absence tests.

Exit condition: no standalone EVM product entry point or disconnected registration remains.

### 5. make store the retained artifact authority

Changes:

- Move EventArtifactRequirement, EventArtifactReferenceSource, and event-to-requirement derivation
  from mfm-events to mfm-store. Events retain event payload/reference facts only.
- Move fact-response requirement construction to mfm-store; delete
  KernelEventPayload::artifact_requirements and RuntimeArtifactStore.
- Keep one RetainedArtifactReadProvider. Rename VerifiedRunArtifactBytes to
  VerifiedRetainedArtifactBytes with no alias.
- Delete mfm-artifact-capabilities; ArtifactReadCapability/Provider/Request/Future/Evidence,
  VerifiedArtifactBytes, and their error families; app artifact_read_provider_from_retained and
  RetainedArtifactReadAdapter; duplicate constructors; and generic hydrate_fact_response_json.
- Move the exact run/artifact-reference/config-ref/seed/seed-cell/public-output-cell/public-output-
  rendered/diagnostic/fact-descriptor/fact-response constructors listed in the binding contract to
  mfm-store. Move their tests from mfm-events/app/artifact-capabilities. Keep only the stated
  attempt-local side-effect/staging derivation private in runtime.
- Verify retained artifact ID, evidence hash, digest against actual bytes, length, media/schema/
  semantic IDs, producer, and role.
- Update in-memory/Postgres retention, replay, portfolio hydration, and test stores to one contract.
  Retain the current portfolio canonical response envelope until the final typed domain decoders
  exist; do not create a decoder for doomed Bitcoin response material.

Focused verification:

- Test missing retention, every mismatched metadata/binding field, wrong digest/length, tamper,
  swap, valid pre-existing admission, and valid prepared bytes.
- Check store, events, replay, app, current portfolio adapter, Postgres, and integration dependents.
- Run cargo-metadata-contract after crate deletion.

Exit condition: mfm-store is the sole retained-artifact authority and one verified-byte type/provider
trait remains.

### 6. make rust-bitcoin the bitcoin identity authority

Changes:

- Add one workspace rust-bitcoin dependency to current pure Bitcoin owners.
- Replace active address/network/canonical rendering, script, hash, outpoint, and amount logic with
  rust-bitcoin.
- Support main, test, testnet4, signet, and regtest. Shared test-family encodings are compatible; the
  checked RPC chain tag will establish the actual chain.
- Delete portfolio bech32/bs58 validation, weaker duplicate validators/dependencies, and handwritten
  Bitcoin primitive logic.
- Rename only reusable primitive/domain Btc types which survive commit 7 to Bitcoin and delete
  their aliases. Make the minimum compile adaptation in doomed per-address types and delete them in
  commit 7. Keep BTC only for units/protocol terms.
- Add no unused aggregate API; commit 7 introduces the final batch behavior directly.

Focused verification:

- Cover canonical/malformed/wrong-family/shared-test-family addresses, duplicate address/script, all
  chains, hashes/outpoints, MAX_MONEY bounds, and redacted errors.
- Check portfolio expansion and current Bitcoin live dependents.

Exit condition: rust-bitcoin is the one active primitive authority and no portfolio validator copy
remains.

### 7. settle read facts and replace bitcoin collection

This is one inseparable contract and product cut. It activates the final aggregate Bitcoin read
directly; it must not teach doomed per-address states to emit facts or retain the managed writer
beside the replacement.

Fact execution and schema changes:

- Add public-but-sealed ReadFactBatch in mfm-program with exactly () and NonEmpty<F>
  implementations. Its only method is a fallible static descriptor returning
  mfm_facts::Result<Option<FactDescriptor>>. Runtime staging and replay comparison use private
  companion traits; add no public visitor, erasure, extension hook, or heterogeneous/optional batch.
- Move MfmFactType to mfm-facts and update derive expansion, rustdoc, and UI tests.
- Change ReadState::reduce to return output plus Facts. Replace external-read contract digest v1
  with a private v2 helper generic over Plan/Evidence/Facts and binding the exact canonical
  none/non-empty fact mode. Delete the v1 function/path.
- EffectRunner defaults to zero emitted descriptors; ReadExternal derives exactly one from
  S::Facts; state registration installs it; launch artifact authority derives it from registered
  states. Delete every manual app/domain descriptor hook.
- Set Facts = () and return (output, ()) for SelectHoldingsState, ValidateEvmContractState, and every
  retained framework/test ReadState fixture. EVM returns NonEmpty<EvmBalanceSnapshotFact>; final
  aggregate Bitcoin returns NonEmpty<BitcoinBalanceSnapshotFact>.
- Make the final EVM graph exactly EvmBalanceCollectionOperation -> CollectEvmBalancesState. Its
  output handle is the final receipt; it returns facts by converting the validated source-derived
  Vec with NonEmpty::try_from_vec and propagating the typed error, never unwrap. Delete
  RecordEvmBalanceFactsInput/handles/schema, record node/runner, and the public graph-only
  observation-batch type; use only a private local Vec before the fallible conversion.
- Set the changed EVM semantic versions exactly to mfm.evm.state.collect_balances.v2,
  mfm.evm.operation.balance_collection.v2, and mfm.evm.jsonrpc.adapter.v2; keep the adapter
  kind/digest and do not reuse any affected v1 identity for the new output, one-node graph, or
  binding behavior.
- The successful semantic payload order is FactRecorded in reducer/ordinal order, CellProduced,
  then StateAttemptCompleted. Evidence, fact-response bytes, admissions/references, retention,
  output, and completion share that append.
- Store admission rejects a FactRecorded unless the same commit has exactly one matching
  CellProduced and StateAttemptCompleted; rejects facts in failed/interrupted/skipped/side-effect/
  run-level commits; rejects fact-only prefixes; requires each response in the same settlement
  bundle as PreparedArtifactBytes or an exact ExistingArtifactAdmission; and rejects duplicate
  FactKey within the commit. Content-addressed reuse never requires re-uploading retained bytes.
- Runtime/history validation also proves ReadExternal/non-empty mode, the one certified descriptor,
  homogeneous non-empty output, exact node/attempt/artifact/content bindings, and event ordinal
  order. Facts = () yields zero facts, while a certified NonEmpty completion with zero facts is
  invalid. Repeated keys across attempts/runs remain valid.
- On uncertain settlement, reload and verify before any live IO; recognize committed settlement or
  follow verified recovery authority, never blindly retry.
- Replay invokes the reducer and compares exact output plus fact count/order/key/descriptor/subject/
  response/content identity/artifact/node/attempt/commit binding.
- Delete ManagedWriteState, ManagedPlatformWrite and every effect/class/role/certify/runtime/
  recovery/replay/UI branch, any ManagedFactWrite proposal, FactRecordCapability,
  mfm-fact-capabilities, FactRecordInput, record states/adapters, RecordedFact(s)/prefix recovery,
  public RunnerFactRecorded/RunnerEventPayload fact construction, RunnerOutputBuilder::record_fact,
  StateSpec::emitted_fact_descriptors, public fact-registration hooks, and domain callbacks.
- Delete provenance, request, observed_at, visibility, audience, scope, StoreScopeRef,
  ScopeDecisionEvidence, their query/claim/ref/derive/SQL/API support, and all private/control
  branches. Keep real StoreScopeId and store-envelope recorded_at. Delete
  DescriptorCatalogWatermark and StoreReadFrontierType; receipts carry the one store-owned snapshot
  frontier specified above.
- In this same persisted event/schema cut, reduce ExecutableIdentity to factory_id + binary_digest.
  Delete cargo_package_digest, nix_derivation_hash, nix_output_hash, NixDerivationHash, and
  NixOutputHash from Rust, canonical/event codecs, schemas, replay, API/debug projections, and
  fixtures. Set the shared EVENT_SCHEMA_VERSION to 2 for all mfm.events.v1.* names and regenerate
  every schema hash/golden rather than creating per-event version switches. Commit 9 changes how
  the surviving binary digest is produced; it does not add another persisted field or store epoch.
- Delete CoverageStatus, HoldingSourceStatus, every fixed coverage/source-status/read-count field,
  and the last Bitcoin-to-portfolio dependency; activate the source-domain to aggregate-domain
  prohibition. Remove Observation.coverage from the public portfolio snapshot.
- Publish only mfm.portfolio/snapshot@2; delete @1. Set PortfolioSnapshot::SCHEMA_VERSION to 2 while
  keeping PortfolioReport::SCHEMA_VERSION at 1 because its shape is unchanged. Set the affected
  semantic versions exactly to mfm.portfolio.state.select_holdings.v2,
  mfm.portfolio.state.assemble_snapshot.v2, mfm.portfolio.state.project_report.v2,
  mfm.portfolio.operation.snapshot.v2, mfm.portfolio.operation.report.v2, and
  mfm.portfolio.adapter.typed.v2; keep the adapter kind/digest and delete its v1 behavior identity.
- Replace FactRecordProjection and FactIndexProjection with one public backend-facing
  FactQueryProjection with private fields and narrow constructors/accessors, shared by in-memory and
  Postgres.
- Reset Postgres baseline/catalog/checksum, SQLx metadata, canonical/API schemas, all identity
  ledger entries and exact target versions named above, and authority to mfm.postgres.store.v4. Map
  authority/checksum mismatches to redacted IncompatibleStoreSchema; add no migration/old-store
  fixture.


Fact query ownership in the same authority cut:

- Move query expressions, canonical plans, results, receipts/evidence, and semantic errors to
  mfm-facts. Install FactQueryReadCapability with name mfm.fact.query.read, kind
  (mfm.fact, query.read), digest sha256_digest_bytes(b"mfm.fact.capability:query.read"), and version
  mfm.fact.query.read.v1; delete the old index-read identity without alias.
- Add the exact object-safe boxed-future FactQueryStore contract specified above. Empty input
  performs no IO; non-empty input uses one snapshot/frontier and returns aligned results.
- Implement it directly for PostgresStore and AsyncInMemoryRunStore with exact IDs
  mfm.storage.postgres.fact-query.v1 and mfm.store.memory.fact-query.v1.
- Delete FactIndexReadCapability/Provider/Request/Future/BatchFuture/Error and invalid/provider-
  failure families, PostgresFactIndexReadProvider, ProjectionFactIndexProvider,
  PublicFactQueryExecutor/Future/Execution, all app factories/aliases/IDs, and every duplicate
  provider error/request wrapper.
- The private portfolio executor maps every S::Error to constant redacted RuntimeError::Blocked;
  malformed count/frontier/order/evidence is InvalidRunnerOutput. It never formats S::Error.
- Remove every caller-supplied fact-query implementation-ID parameter. Registration reads
  fact_query_implementation_id from the same Arc<S> it stores for execution, and permanent assembly
  validation compares the installed binding back to that Arc. Commit 17 adds semantic-key equality
  without replacing this same-object check or adding an expected-ID table.
- Remove authored scope from compiler/plans and make every non-empty aligned result carry the exact
  shared StoreScopeId/StoreCommitOrder frontier loaded from that Arc<S>.
- Collapse RunServices<S, A>, public facts reads, launch/status hydration, portfolio registration,
  CLI/REST construction, and test overrides to one Arc<S> carrying query and retained-artifact
  traits. Adversarial tests wrap that one store rather than inject a second provider.

Final Bitcoin vertical cut in the same commit:

- Replace the graph directly with BitcoinBalanceCollectionOperation ->
  CollectBitcoinBalancesState, one state for each
  (network_id, bitcoin_network, semantic_source_identity). Delete the per-address graph instead of
  migrating it to ReadFactBatch.
- Require 1 through 1,024 strictly sorted, duplicate-free checked addresses per group.
- Add the exact `BitcoinBalanceCollectionReadCapability`/`BitcoinBalanceSession` contract and
  implementation ID specified above. App's one routed session Arc covers every demanded source;
  register it once. Its live operation performs exactly getblockchaininfo, one multi-descriptor
  scantxoutset start, and getblockhash(scan.height), in order.
- Install the exact Bitcoin operation/state names, kinds, digests, versions, adapter v2 binding,
  fact kind, and eight schema/semantic identities specified above. Delete every old per-address,
  chain-head, and adapter-v1 identity in this same cut.
- Implement the exact endpoint/auth/timeouts/body-limit, strict JSON-RPC 2.0, Core 28+, scan-busy,
  script/outpoint,
  raw decimal/MAX_MONEY, checked sum, chain/IBD, and reorg contract above. Exact scriptPubKey bytes,
  not returned desc, are authority.
- Add required scan_timeout_seconds to the current runtime-configuration route now; commit 20 moves
  that final field and validation into app and deletes the old configuration crate.
- Keep no coordinator/lock, status/abort, hidden retry, alternate decoder, or version negotiation.
- Return ordered NonEmpty<BitcoinBalanceSnapshotFact> plus the minimal receipt through the new
  settlement.
- Add final domain decoders for BitcoinBalanceSnapshotResponse and the surviving EVM balance
  response. They require canonical bytes, closed typed decoding, trailing-byte rejection, and
  domain invariants. Bitcoin Core JSON-RPC structs remain separately forward-additive for unknown
  object fields. Portfolio, not these decoders, later rederives selected-ref identities.
- Delete ResolveBtcJointTipState, every per-address capability/call/state/config/output, old
  observation/record state, assembler/intermediate schema, constant read/status/coverage field,
  header/status/abort path, and obsolete Btc name.
- Add a closed PortfolioHoldingFactEvidence enum with typed Bitcoin/EVM response variants. Migrate
  current portfolio hydration to the new domain decoders; delete PortfolioHoldingFactResponse's
  canonical-string envelope and manual serde_json field parsing. Portfolio separately rederives
  descriptor, subject, response content identity, and artifact evidence against InternalFactRef.
- Update portfolio composition, certification, current production registration, replay, APIs, and
  docs.

Focused verification:

- Check/test program, derive, capabilities, certify, runtime, replay, events, facts, store, Postgres,
  current Bitcoin/EVM/portfolio packages, app, CLI, and REST.
- Inject settlement/uncertain-result failures in memory and Postgres; prove zero-or-complete payloads
  and no repeated live IO.
- Independently tamper (a) the connection authority fingerprint and (b) one SQLx migration checksum.
  Require the same redacted public IncompatibleStoreSchema code/message with no database detail,
  while internal inspection distinguishes typed StoreAuthorityMismatch from
  MigrationChecksumMismatch. Add no legacy reader or old-store fixture.
- Test no-fact reads, missing/extra/reordered/duplicate/wrong facts, retry/recovery/replay, old-schema
  rejection, and every strict response decoder case.
- Test empty-query no-IO, non-empty one-snapshot/aligned results, memory/Postgres parity, same-store
  assembly, typed backend failure, and redacted capability/app mapping.
- Run the barrier-controlled memory/Postgres fact-append and descriptor-admission interleavings
  specified above, including descriptor-only StoreCommitOrder advancement and exact before/after
  frontier contents.
- Cover all Bitcoin fixtures above and assert exactly three calls/one complete sorted scan, zero
  balances, shared anchor, timeout/cancellation without abort, retry/scan-busy/reorg, atomic facts/
  receipt, replay order, and one state per source in Bitcoin-only/mixed graphs.
- Run test-db for the physical store cut. Final validation owns the one broad ci run.

Exit condition: one read-fact path, one query projection, and one bounded aggregate Bitcoin path
remain; no managed write, per-address read/record, speculative fact concept, or old store authority
can execute/open.

### 8. fold surviving effects into capabilities

Changes:

- Move Pure, ReadExternal, ApplySideEffect, descriptors, and tests from mfm-effects to
  mfm-capabilities.
- Preserve `mfm.effect.v1` and these exact existing kind identities byte-for-byte:
  `effect:mfm.kernel.effect:pure:sha256-jcs-v1:a54d4bec4951dc5e829fa22cf2f0305c45c5151a1b3cfe965477734fb9996559`,
  `effect:mfm.kernel.effect:read_external:sha256-jcs-v1:57b1f6b604d4d724bdf593d53f397f88661f07c17cd2ed036e454f66192a6502`,
  and
  `effect:mfm.kernel.effect:apply_side_effect:sha256-jcs-v1:a007caa404540427f9773d9d3d7b1ff763ed7d3449a53f28cc503403e37d0258`.
  This is an ownership relocation, not another semantic identity cut.
- Update imports, derives, rustdoc, design docs, and metadata; delete mfm-effects and every alias.
- Re-evaluate visibility after colocation.

Focused verification:

- Check/test capabilities, program, certify, runtime, replay, affected domains, and current
  framework/production registration.
- Run compile-fail and cargo-metadata-contract tests.

Exit condition: one kernel crate owns authority/effect contracts and mfm-effects is absent.

### 9. centralize pure state execution and replay

Changes:

- Add the private generic PureStateRunner and exact caller-bound registration helper specified
  above. Preserve factory ID `pure`, retained-artifact input, optional context-output extraction,
  and caller-supplied `RunnerFactoryBinding`.
- Replace label-derived executable digests with the one app-computed current-executable byte digest
  and revised `ExecutableIdentityTemplate`/registry contract specified above. Pass its bindings
  through framework and all domain registration roots; runtime/domain code never derives a process
  identity or supplies a default.
- Delete `RunnerExecutableIdentityTemplate`, `framework_executable`, every adapter-local template
  construction, and the `production-runners`, `typed-bitcoin-jsonrpc`, `typed-evm-jsonrpc`, and
  old `typed-portfolio` executable-label argument paths. The final portfolio adapter kind/digest
  legitimately retains the separate `typed-portfolio` kind component. Independently hash the
  running test executable to prove the app helper/template binds exact bytes; changing the fixture
  digest must change admitted binding evidence.
- Test the streaming helper against one-shot raw SHA-256 for empty, short, and multi-buffer inputs,
  then compare the exact canonical identity object/content digest. All framework/domain factories
  share that template digest while retaining distinct factory IDs; open/read/canonicalization/join
  failures expose neither executable path nor backend text.
- Add one generic verify_pure_state in mfm-replay.
- Migrate every surviving ordinary pure state in current Bitcoin, EVM, and portfolio packages.
- Delete adapter pure-runner factories, identities, erased input/output plumbing, and replay copies;
  keep runtime erasure private.
- Ensure replay uses no live/current-config provider and preserves context-output semantics.

Focused verification:

- Runtime tests cover arbitrary certified input trees, canonical/context output, malformed input/
  config, executable binding, and redacted failure.
- Live-resume tests reject a different executable digest before runner/live IO. Replay tests install
  panicking executable/config/live providers and compare exact canonical output without hashing the
  current executable.
- Test every migrated state and current production registration.

Exit condition: one ordinary pure execution path and one pure replay implementation remain.

### 10. consolidate the bitcoin pure domain

Changes:

- Create mfm-bitcoin with private model, capability, state, and operation modules.
- Move final Bitcoin code and delete mfm-btc-capabilities, mfm-states-btc, and
  mfm-op-btc-collectors immediately.
- Enforce exact role direction; make package-crossing helpers private and root-export only final
  consumer types.
- Update imports/manifests/metadata/docs/rustdoc/tests/Cargo.lock without bridges.

Experiment proof:

- Record public-item/bridge-type diff and require reduction.
- Prove role direction and absence of runtime/replay/store/app/keystore/network/FS dependencies.
- Confirm one representative Bitcoin semantic change has one pure owner.

Focused verification:

- Check/test mfm-bitcoin and app/live/portfolio/integration dependents.
- Run pure-role, production-registration, and cargo-metadata-contract tests.

Exit condition: Bitcoin has one pure crate with less public/package plumbing. If Cargo isolation is
still materially required, stop here before copying the pattern.

### 11. consolidate bitcoin live with a public transport

Changes:

- Create mfm-bitcoin-live with public transport and private adapter sibling modules; delete old
  Bitcoin adapter/transport crates.
- Transport implements the one pure session trait; adapter accepts a supplied trait implementation
  and never imports concrete transport.
- Expose checked session, resolved endpoint/auth, and redacted error under transport only. Keep raw
  clients/envelopes/arbitrary calls private.
- App constructs endpoint-bound sessions, places them in the one app-private routed trait object,
  and passes that Arc to narrow root registration. Add no concrete convenience adapter, feature
  split, or root glob.
- Replace brittle source scans with external-use, fake-session, and adapter/raw-rpc privacy tests.

Focused verification:

- Check/test mfm-bitcoin-live/app, limits/requests/redaction, fake registration, external
  use/privacy, production registration, and cargo-metadata-contract.

Exit condition: transport is reusable without registry; adapter is private and transport-independent.

### 12. add pinned bitcoin core parity

Changes:

- Add a Nixfied isolated regtest Bitcoin Core service from root-pinned nixpkgs.
- Record/assert the exact bitcoind version and require it to be at least 28. Use deterministic RPC
  credentials, an isolated data directory, and no host bitcoind.
- Mine a deterministic fixture UTXO and exercise commit 7's final public typed transport against
  the live node, including chain/IBD validation, one multi-descriptor scan, scan-height hash
  confirmation, zero/nonzero balances, and redacted failure.
- Add one focused parity task, compose it into final CI, and document it in
  docs/build-and-verification.md. Add no production API/state/session path in this verification-only
  commit.

Focused verification:

- Run `nix run .#model-check` and the new focused Bitcoin Core parity task.
- Let the final revision's single ci run prove task-graph composition; do not run the component
  gates redundantly immediately beforehand.

Exit condition: the final aggregate transport passes against the pinned live Core service and CI
owns the parity environment/evidence.

### 13. consolidate the evm pure domain

Changes:

- Create mfm-evm with private model, capability, signing, state, and operation modules.
- Move and delete mfm-evm-capabilities, mfm-evm-signing, mfm-states-evm, and
  mfm-op-evm-collectors.
- Keep generic mfm-signing separate/lower and expose minimum balance, validation, transaction API.
- Enforce role direction and delete package-only bridges/re-exports.

Focused verification:

- Check/test mfm-evm and dependents; run signing security/compile-fail, pure-role,
  production-registration, and metadata tests.

Exit condition: EVM pure behavior has one owner and no live/app/keystore/store dependency.

### 14. consolidate evm live with a public transport

Changes:

- Create mfm-evm-live with public transport and private adapter; delete old EVM adapter/transport.
- One checked session implements pure read/transaction traits. Adapter is generic over those traits
  and never imports concrete transport.
- App supplies the one routed EVM read-session Arc over `(network_id, source_ref)` route instances;
  register its self-reported implementation ID once. Direct endpoint sessions remain independently
  reusable.
- Move the already-separate narrow balance, validation, and transaction registration functions;
  production continues to call balance only.
- Keep raw protocol private and transport free of runtime/replay/store/app concerns.

Focused verification:

- Test external transport, fake adapter, chain identity, bounds/redaction, balance/validation/
  transaction; check app, privacy/source-role, production-registration, and metadata.

Exit condition: transport is independently reusable and balance production creates no transaction
signer/session.

### 15. consolidate the portfolio pure domain

Changes:

- Create mfm-portfolio with private model, state, and operation modules.
- Move/delete mfm-portfolio-model, mfm-state-portfolio, and mfm-op-portfolio-snapshot.
- Depend only on pure Bitcoin/EVM and domain-facing kernel; delete bridge inputs/globs/errors/paths.

Focused verification:

- Test deterministic Bitcoin-only/EVM-only/mixed expansion and current production registration.
- Check portfolio/dependents and run pure-role/metadata tests.

Exit condition: portfolio graph/state semantics have one pure owner and no live IO.

### 16. consolidate portfolio live without platform providers

Changes:

- Create mfm-portfolio-live from final portfolio adapter; delete mfm-adapters-portfolio.
- Keep no transport. Expose registration over one Arc<S> implementing FactQueryStore and
  RetainedArtifactReadProvider, internally coerced to least-authority trait objects.
- Delete PortfolioRunnerCapabilities and separate providers.
- Use pure canonical decoders; delete copied/generic hydration.
- Move the already-typed PortfolioHoldingFactEvidence hydration path; do not recreate the deleted
  opaque envelope or manual JSON parser.
- Forbid source-live and concrete-store dependencies.

Focused verification:

- Test one fake store, shared snapshot, missing/mismatched/tampered/swapped bytes, invalid decoding,
  redaction, and inability to assemble different stores.
- Run dependency/source-role/app/portfolio/production-registration/metadata tests.

Exit condition: portfolio live owns selection/hydration only, no platform provider/transport.

### 17. bind production assembly to the published authoring catalog

Changes:

- Make mfm.portfolio/snapshot@2 declare the independent sealed authoring catalog specified above.
  Treat it as the production universe, not proof of semantic reachability.
- Extend define_program_descriptor_registry! with the one generated ProgramAuthoringCatalog API
  specified above. The same type lists generate the authoring/certification registries and catalog;
  add an undeclared-child negative fixture to prove graph construction is catalog-gated.
- Require the union of canonical Bitcoin-only, EVM-only, and mixed expansion lineage to equal the
  declared domain operation/state sets. This is the independent declared-to-reachable check; do not
  replace it with a source-text scan or an app-registry comparison.
- Add crate-private inspection and semantic-key equality checks across certification, emitted
  facts, runners, capabilities, executables, side-effect verification, and replay. The pure catalog
  names no concrete implementation or executable ID.
- Require one concrete binding for every required runner/capability key, no extra domain binding,
  and separate validation that each live/store implementation ID is self-reported by the same
  selected object used for execution.
- Put the exact framework-only bootstrap behind one sealed runtime function. It contains no domain
  package/registration.
- Verify the already-separated EVM balance, validation, and transaction registration functions.
  Production calls balance registration only.

Focused verification:

- Test mfm-app assembly and the portfolio-objective integration target.
- Add missing, duplicate, extra, wrong-executable, and wrong-implementation tests on every surface.
- Run cargo-metadata-contract for framework/domain ownership.

Exit condition: production assembly equals the independently authored catalog plus closed runtime
bootstrap; neither sample drafts nor app registration define the expected set.

### 18. delete secret export and consolidate keystore

Changes:

- Delete dangerous-secret-export, allow_secret_exports, export_private_key, related feature/config/
  audit/error/test/doc/call sites.
- Create mfm-keystore from mfm_core and mfm-signers-keystore; keep mfm-signing lower.
- Make raw key retrieval, SecureKey, and cross-package bridge private. Public API is import, list,
  metadata, delete, signing.
- Delete both old crates/paths/bridges without a compatibility parser. Consolidation alone does not
  justify format churn, but existing-file readability is not a compatibility requirement; if the
  shape must change for correctness or security, replace it directly with one strict format.
- Signing provider retains redacted checked keystore/unlock-file paths only. Each sign reads <=64
  KiB with limit + 1 on a blocking worker, rejects oversize, strips at most one LF/CRLF, and keeps
  raw/UTF-8 secret storage zeroizing through every success/error path.
- Preserve bounds, tamper/AAD/swap/constant-time/zeroization and non-Send/non-Sync properties.

Focused verification:

- Run complete keystore corruption/swap/audit/bounds/zeroization/signing/redaction suites, including
  instrumented success/empty/invalid-UTF-8/oversize/partial-read drop paths.
- Check app/CLI, metadata, and absence of export/raw bridge.

Exit condition: one secret-provider crate remains and no public/build path exports raw key material.

### 19. put keystore application services behind app

Changes:

- Add consuming SecretInput(Zeroizing<String>) with no Clone/Copy/Debug/Display/Serialize/
  Deserialize/borrowed accessor.
- Move keystore selection, CRUD/import, and signing services from CLI support to app.
- Consume one-shot secrets inside spawn_blocking; repeat signing reads unlock file per call.
- Delete CLI provider factories/construction/mirror errors and non-zeroized secret buffers.
- Keep REST free of secret-bearing keystore/config ingress.
- Add and retain binary-to-secret-provider/signing prohibition.

Focused verification:

- Test interactive/stdin/noninteractive inputs with no secret in snapshots/logs/errors.
- Test app/CLI/REST redaction and negative trait assertions.

Exit condition: binaries cannot construct/inspect keystore implementation and app retains no unlock
secret.

### 20. move runtime configuration into app

Changes:

- Move bounded load/parse/select/resolve/error mapping into private app modules and implement the
  exact configuration contract above.
- Make explicit CLI/REST `--runtime-config <PATH>` the sole document-path source. Delete
  MFM_RUNTIME_CONFIG_FILE, from_path_or_env, environment fallback/precedence, and their tests/docs;
  missing paths fail only for work that actually requires a live route or selected keystore.
- Enforce exact four sections/selected schemas, case-sensitive extension, JSON nested duplicates,
  the exact global secret-marker list, selective strict decode, one-key ValueSource, UTF-8/path/env
  grammar, Bitcoin scan-timeout bounds, and limit-plus-one bounds.
- Treat 1 MiB document and 64 KiB selected-value/file limits as new ceilings. Strip one line ending
  only from file-backed values.
- Resolve selected secrets directly into consuming zeroizing transport inputs; app retains no copy
  and Arc session clones do not copy secrets.
- Allocate protected raw buffers before secret reads/acquisition and construct protected strings
  without an ordinary secret intermediate; prove zeroizing drop on every success/error path.
- For start/resume, derive the exact pending Bitcoin/EVM route keys from certified nodes, resolve
  each required entry once, build endpoint-bound sessions, and populate the single app-private
  routed session sets specified above. Do not parse or instantiate unrelated routes.
- Keep chain/network expected IDs certified, and keystore profiles path-only.
- Delete mfm-runtime-config, btc spellings, old resolver/precedence/fallbacks/fixtures, and direct
  live/binary env/file IO. Replay never opens config.
- Add and retain binary-to-non-binary-facing-assembly prohibition.

Focused verification:

- Test JSON/TOML parity, duplicates, unknown fields, prohibited/unselected secrets, unused-entry
  isolation, exact-one sources, non-Unicode env, grammar, line endings, scan-timeout bounds,
  content limits, partial-read failures, zeroizing drops, blocking IO, and redaction.
- Test resolved live/keystore binding and panicking-config replay; check app/live/binaries/metadata.

Exit condition: one app-private resolver and final transport secret owner remain; no config package,
fallback, or ambient config IO exists.

### 21. finish thin binary assembly

Changes:

- Put Postgres/live/runtime registry construction, shared store, and run services fully behind app.
- Remove CLI/REST normal/build dependencies on runtime, replay, live, storage implementation,
  keystore/signing implementation, config, store internals, and pure execution.
- Keep direct mfm-ids/canonical only for genuine presentation. Delete mirror app DTOs/traits.
- Move generic store bounds and implementation construction out of binaries.
- Update one current CLI/REST schema/docs.
- Add and retain binary-to-non-binary-facing-kernel prohibition; the full binary row is active.

Focused verification:

- Run CLI help/text/JSON/start/resume/status/keystore and REST parity.
- Inspect Cargo metadata/source, public types, LOC, and run metadata contract.

Exit condition: app is sole execution/implementation dependency; ids/canonical are the only direct
kernel dependencies.

### 22. install final source, api, and full-matrix guardrails

Changes:

- Audit every final matrix row is active with synthetic positive/negative fixtures. Add only a
  genuinely missing permanent assertion; there is no partial/full validator mode.
- Audit Cargo target-kind coherence: every package with a bin target is layer binary and mixed
  lib/bin packages obey the binary dependency row; proc-macro targets remain dedicated/non-binary.
- Audit and promote the pure module State/Operation placement and lower-to-higher import checks
  introduced by the consolidation commits into the final durable suite.
- Audit and promote the existing live checks for public transport/private adapter, transport isolation, no
  adapter-to-concrete-transport import, and portfolio-live ownership.
- Consolidate the existing external typed-transport, fake-session, and compile-fail privacy tests;
  retain coverage only for the adapter module and public raw rpc_call escape, without freezing
  arbitrary private type names.
- Audit the existing thin-binary and authoring-catalog equality tests and make them the durable
  final guardrails; do not add parallel copies.
- Delete old category code/helpers/source scans and every path/count/name exception assertion.
- Update architecture/design docs to final contracts.

Focused verification:

- Run cargo-metadata-contract and all architecture/API/privacy/trybuild targets.
- Run focused checks for any missed edge and git diff --check.

Exit condition: one semantic matrix covers the workspace and invalid dependencies/source roles/API
exposure fail for properties, never inventory.

## Commit execution rules

Before each commit:

- Confirm git diff contains one numbered outcome and no unrelated user changes.
- Run rustfmt for Rust changes.
- Use nix develop -c cargo for focused checks/tests; do not rely on a host Rust toolchain.
- Expand to affected dependents when a public contract changes.
- Update code, tests, rustdoc, architecture/design docs, generated schemas, and SQLx metadata in the
  same commit.
- Run git diff --check.
- Use the exact lower-case commit subject.

Verification follows docs/build-and-verification.md by affected surface. Do not repeat nix run
.#check, .#test, and .#test-db before every commit or immediately before final CI. Run a component
gate when it is the smallest sufficient final coverage or to diagnose a failure.

At every deletion commit, use rg and Cargo metadata to prove absence of the old package/type/ID/path.
Matches retained only in this RFC/plan or a deliberate rejection test must be reviewed explicitly.

## Milestone audits

After commits 7, 10, 16, 20, and 22, record a short implementation note in the commit message or
review description:

- crates deleted/added;
- public items or bridge types deleted;
- duplicated runner/provider paths deleted;
- any new public item and why another final crate must name it;
- dependency firebreak tests added; and
- net diff/LOC direction.

These are review aids, not permanent package-count or LOC tests. If consolidation increases public
surface or leaves two owners for a responsibility, correct it before proceeding.

## Final validation

On the final revision:

1. Run nix run .#ci once; it owns the Nixfied-pinned Bitcoin Core parity task. Do not run its
   component gates immediately beforehand unless diagnosing a
   known failure.
2. Require the terminal closing-source-revision evidence to contain the full tested Git SHA.
3. Run git status --short and require a clean worktree.
4. Run an absence audit for deleted crates and concepts:

       mfm-artifact-capabilities
       mfm.artifact.read
       mfm.artifact.capability:read
       ArtifactReadCapability
       ArtifactReadProvider
       ArtifactReadRequest
       ArtifactReadFuture
       ArtifactReadError
       ArtifactReadInvalidRequest
       ArtifactReadDecodeFormat
       ArtifactReadBackendError
       mfm_artifact_capabilities::ArtifactEvidenceRef
       VerifiedArtifactBytes
       artifact_read_provider_from_retained
       RetainedArtifactReadAdapter
       hydrate_fact_response_json
       mfm-fact-capabilities
       mfm-effects
       mfm-runtime-config
       mfm_artifact_capabilities
       mfm_fact_capabilities
       mfm_effects
       mfm_runtime_config
       mfm_core
       mfm-signers-keystore
       after_registration
       RunnerExecutableIdentityTemplate
       framework_executable
       cargo_package_digest
       nix_derivation_hash
       nix_output_hash
       NixDerivationHash
       NixOutputHash
       production-runners
       typed-bitcoin-jsonrpc
       typed-evm-jsonrpc
       ManagedWriteState
       ManagedPlatformWrite
       managed_platform_write
       ManagedFactWrite
       FactRecordCapability
       RecordedFacts
       FactProducerProvenance
       FactIndexReadCapability
       FactIndexReadRequest
       FactIndexReadFuture
       FactIndexReadBatchFuture
       FactIndexReadError
       FactIndexReadProvider
       FactRequestEvidence
       FactIndexInvalidRequest
       FactIndexProviderFailure
       FactVisibility
       FactAudience
       FactVisibilityScope
       FactQueryScope
       DescriptorCatalogWatermark
       StoreReadFrontierType
       StoreScopeRef
       ScopeDecisionEvidence
       FactMetadataField::ObservedAt
       FactExtractionMetadata.observed_at
       FactRecordProjection
       FactIndexProjection
       apply_fact_recorded_record_only
       PostgresFactIndexReadProvider
       ProjectionFactIndexProvider
       mfm.app.postgres.fact-index.v1
       mfm.app.projection.fact-index.v1
       mfm.fact.index.adapter.v1
       mfm.integration.managed-fact-record.v1
       PublicFactQueryExecutor
       PublicFactQueryFuture
       PublicFactQueryExecution
       FactRecordInput
       RecordBtcAddressBalanceFactState
       RecordEvmBalanceFactsState
       RecordEvmBalanceFactsInput
       RunnerOutputBuilder::record_fact
       RunnerFactRecorded
       RunnerEventPayload::FactRecorded
       StateSpec::emitted_fact_descriptors
       register_fact_type
       register_fact_descriptor
       RecordedFact
       ManagedFactRecordRunner
       run_managed_fact_record
       ManagedPlatformWriteRole
       mfm.runtime.managed-fact-record.v1
       mfm.fact.record
       mfm.fact.capability:record
       mfm.fact.index.read
       mfm.fact.capability:index.read
       mfm.facts.v1
       mfm.facts.query.v2
       mfm.fact-query.v2
       mfm.fact-query-plan.v1
       mfm.fact-query-receipt.v2
       mfm.fact-query-result-set.v1
       mfm.fact-query-evidence.v2
       schema:mfm.events.v1.fact_recorded:1:
       pub const EVENT_SCHEMA_VERSION: &str = "1"
       mfm.postgres.store.v3
       RuntimeArtifactStore
       VerifiedRunArtifactBytes
       KernelEventPayload::artifact_requirements
       ResolveBtcJointTipState
       CollectorCheckpoint
       BtcChainHead
       BtcChainHeadReadCapability
       BtcChainHeadReadProvider
       BtcChainHeadReadRequest
       BtcChainHeadReadResponse
       ObserveBtcChainHeadState
       ObserveBtcAddressBalanceState
       RecordBtcChainHeadFactState
       RecordBtcAddressBalanceFactState
       AssembleBtcNetworkCollectionReceiptState
       CoverageStatus
       HoldingSourceStatus
       BTC_NATIVE_BALANCE_COVERAGE
       BTC_NATIVE_BALANCE_SOURCE_STATUS
       BtcBalanceReadCapability
       BtcBalanceReadProvider
       BtcBalanceReadRequest
       BtcBalanceReadResponse
       BtcNativeBalancesAtAnchorOperation
       BtcNetworkCollectionOperation
       BTC_JOINT_TIP_SOURCE_READS
       BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS
       getblockheader
       scantxoutset status/abort APIs
       bitcoin.address_balance_snapshot
       mfm.bitcoin.balance.read
       mfm.bitcoin.balance.read.v1
       mfm.bitcoin.capability:balance.read
       mfm.bitcoin.chain_head.read
       mfm.bitcoin.chain_head.read.v1
       mfm.bitcoin.capability:chain_head.read
       mfm.bitcoin.jsonrpc.runtime.v1
       mfm.bitcoin.jsonrpc.adapter.v1
       mfm.bitcoin.fact.address_balance.subject
       mfm.bitcoin.fact.address_balance.response
       mfm.bitcoin.fact.address_balance_snapshot
       mfm.bitcoin.fact.chain_head
       mfm.bitcoin.fact.collector_checkpoint
       mfm.bitcoin.address_balance.
       mfm.bitcoin.chain_head.
       mfm.bitcoin.collector_checkpoint.
       mfm.bitcoin.collector-checkpoint.
       mfm.bitcoin.joint_tip.
       mfm.bitcoin.external_read.address_balance.
       mfm.bitcoin.external_read.chain_head.
       mfm.bitcoin.external_read.collector_checkpoint.
       mfm.bitcoin.collection.
       mfm.bitcoin.operation.config.btc_chain_head_collector_cycle
       mfm.bitcoin.operation.config.native_balances_at_anchor
       mfm.bitcoin.operation.config.network_collection
       mfm.bitcoin.operation_outputs.btc_chain_head_collector_cycle
       mfm.bitcoin.operation_outputs.native_balances_at_anchor
       mfm.bitcoin.operation_outputs.network_collection
       mfm.bitcoin.public_outputs.btc_chain_head_collector_cycle
       mfm.bitcoin.state.config.
       mfm.bitcoin.state.input.
       mfm.bitcoin.state.output.
       mfm.bitcoin.state.value.
       mfm.bitcoin.operation.btc_chain_head_collector_cycle.v1
       mfm.bitcoin.operation.btc_native_balances_at_anchor.v1
       mfm.bitcoin.operation.btc_network_collection.v1
       mfm.bitcoin/btc_chain_head_internal_test@1
       resolve_joint_tip
       observe_address_balance
       record_address_balance_fact
       assemble_network_collection_receipt
       btc_native_balances_at_anchor
       btc_network_collection
       btc_chain_head_collector_cycle
       EvmBalanceObservationBatch
       RecordEvmBalanceFactsState
       record_balance_facts
       mfm.evm.balance_observation
       mfm.evm.input.record_balance_facts
       mfm.evm.record_balance_facts
       mfm.evm.internal_cycle_outputs.balance_collection
       evm_balance_collection_cycle_program_draft
       evm_balance_collection_cycle_program_launch_plan
       evm_balance_collection_cycle
       mfm.evm.state.collect_balances.v1
       mfm.evm.operation.balance_collection.v1
       mfm.evm.jsonrpc.adapter.v1
       PortfolioHoldingFactResponse
       PortfolioRunnerCapabilities
       mfm.portfolio/snapshot@1
       mfm.portfolio.state.select_holdings.v1
       mfm.portfolio.state.assemble_snapshot.v1
       mfm.portfolio.state.project_report.v1
       mfm.portfolio.operation.snapshot.v1
       mfm.portfolio.operation.report.v1
       mfm.portfolio.adapter.typed.v1
       MFM_RUNTIME_CONFIG_FILE
       from_path_or_env
       dangerous-secret-export
       allow_secret_exports
       export_private_key
       mfm-collectors-proof
       mfm-op-proof
       mfm-transports-proof
       mfm.proof
       mfm_collectors_proof
       mfm_op_proof
       mfm_transports_proof
       mfm-btc-capabilities
       mfm-states-btc
       mfm-op-btc-collectors
       mfm-adapters-btc-jsonrpc
       mfm-transports-btc-jsonrpc-http
       mfm-evm-capabilities
       mfm-evm-signing
       mfm-states-evm
       mfm-op-evm-collectors
       mfm-adapters-evm
       mfm-transports-evm
       mfm-portfolio-model
       mfm-state-portfolio
       mfm-op-portfolio-snapshot
       mfm-adapters-portfolio
       mfm_btc_capabilities
       mfm_states_btc
       mfm_op_btc_collectors
       mfm_adapters_btc_jsonrpc
       mfm_transports_btc_jsonrpc_http
       mfm_evm_capabilities
       mfm_evm_signing
       mfm_states_evm
       mfm_op_evm_collectors
       mfm_adapters_evm
       mfm_transports_evm
       mfm_portfolio_model
       mfm_state_portfolio
       mfm_op_portfolio_snapshot
       mfm_adapters_portfolio
       mfm_signers_keystore

Historical names may remain in this RFC/plan and deliberate reject-old-input tests only. No source,
manifest, generated schema, fixture, feature, registry, or current documentation may depend on
them.

The final adapter kind `(mfm.portfolio, typed-portfolio)` and its digest material legitimately keep
the `typed-portfolio` spelling. Audit only the deleted executable-label construction: no template,
factory-binding, executable canonical object, or runner constant may use `typed-portfolio` as its
runner label.

Executable-identity code, schemas, and goldens must contain only the exact
`mfm.executable-bytes.v1`/`sha256` canonical material specified above. Reject the old executable
identity object's `crate`, `runner`, and `version` keys on those qualified surfaces; those ordinary
words may remain in unrelated package and runtime contracts.

All retained event schema names remain under `mfm.events.v1.*`, but every generated current schema
ID must carry schema version 2. Qualify the generated-schema/golden audit to reject version 1 for
every such event name, not only the FactRecorded literal listed above.

Additionally review every production-source, current-documentation, generated-schema, SQL,
SQLx-metadata, and fixture match for `fact-index`, `fact_index`, and `managed-fact-record`; only this
RFC/plan and deliberate reject-old-input tests may retain those identities. In the same qualified
surfaces, reject the obsolete fact container keys `visibility`, `request`, and `producer` in
`FactClaim`, `InternalFactRef`, `FactRecorded`, fact-query/API schemas, canonical codecs, and
fixtures, as well as their obsolete leaves `observed_at`, `request_schema_id`, `request_hash`,
`audience`, `visibility_scope`, `query_scope`, and `scope_decision_evidence`. In fact-query
plan/receipt/evidence schemas, codecs, and fixtures, also reject the old canonical keys
`store_scope`, `descriptor_catalog_watermark`, and `frontier_type`; `store_scope_id` is the only
surviving scope key. In the final fact projection/table specifically, also reject the deleted
producer-provenance columns
`capability_kind`, `capability_version`, `adapter_kind`, and `adapter_version`; those names may
legitimately exist on non-fact authority surfaces.

In store fact projections, snapshots, SQL/SQLx metadata, and their focused fixtures, also reject
the record-only collection/member names `fact_records` and `apply_fact_recorded_record_only`; the
single `FactQueryProjection` path is the only projection authority. Do not globally reject the
ordinary English word `request` or the unrelated artifact/cell `producer` fields.

In the replaced Bitcoin collection, portfolio observation, and their generated schemas/fixtures,
reject the obsolete lower-case fields `coverage`, `source_status`, `observed_source_status`,
`max_source_reads`, and `source_read_count`. Qualify these checks to those DTO/schema families so
unrelated operational status or bounded-read concepts are not rejected by spelling alone.

Because EventArtifactRequirement and EventArtifactReferenceSource survive under a new owner, audit
their paths rather than globally rejecting their names: mfm-events must define/export neither, and
only mfm-store may own the event/read requirement constructors listed above. Runtime may retain only
its private attempt-local side-effect/staging derivation. Also audit production Rust for MFM-owned
Btc followed by an uppercase identifier character; every match must be deleted unless it is an
explicitly documented denomination or protocol term.

## Definition of done

- The workspace has one pure and one appropriate live crate per current domain.
- Pure role separation is enforced inside concentrated crates.
- Bitcoin and EVM expose reusable typed transports independently of private adapters.
- Portfolio live contains portfolio orchestration, not platform providers or transports.
- Stores directly provide fact query and retained artifact IO.
- Pure states and fact-producing reads each have one generic execution/replay path.
- Live runner registries use one exact executable-byte template; label/package/Nix provenance
  fields and alternate identity constructors are absent, while evidence-only replay does no
  executable IO.
- Managed writes and their fact-record/recovery infrastructure are deleted.
- Bitcoin collection is one bounded, three-call aggregate read and one atomic fact settlement.
- Production assembly exactly matches the published portfolio authoring catalog.
- Runtime config and keystore implementation are behind app services; binaries remain thin without
  mirror DTOs.
- No compatibility, legacy, fallback, shell, or dual implementation remains.
- Architecture tests enforce semantic dependencies, source roles, public transport reuse, and
  authoring-catalog equality without exact inventories.
- Final CI passes with retained source revision evidence and the worktree is clean.
