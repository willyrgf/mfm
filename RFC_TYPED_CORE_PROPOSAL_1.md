# RFC: Typed Core Proposal 1 - Typed Expansion Core With Certified Lowering

Date: 2026-05-21

Status: design contract draft for typed-core implementation

Related problem statement: `RFC_STATE_OPS_PROBLEM.md`

Implementation checklist: `IMPL_PLAN_RFC_TYPED_CORE_PROPOSAL_1.md`

This proposal is a candidate architecture for replacing the current dynamic state/operation core.
It is intentionally breaking. It assumes the platform is still on a development branch and does not
need to preserve the existing `PlannedOp`, `PortKey`, `DynContext`, or dynamic DAG authoring APIs as
public framework surfaces.

The proposal is centered on one stricter principle:

```text
typed state programs are the only semantic executable surface;
certified typed execution specs are the only runtime contract;
erased execution plans are implementation artifacts.
```

In this proposal, "state program" means a sequence or graph of states. It does not mean operations.
Operations may author, plan, and explain a state program, but they are not part of the executable
program after expansion.

This is not an incremental validation layer over the current dynamic DAG model. The dynamic model is
the architectural defect this proposal replaces. Proposal 1 does not require backward compatibility
with old dynamic authoring APIs. New typed execution must be implemented through a clean typed
kernel and a new typed scheduler rather than wrapping the current scheduler as semantic authority.

### Architecture Summary

MFM should move from:

```text
JSON config
  -> Operation::expand
  -> PlannedOp
  -> flat StateGraph<dyn State + StateId edges + PortKey bindings + JSON context>
  -> runtime validation and execution
```

to:

```text
typed planning config
  -> deterministic typed operation/state expansion
  -> typed state program IR
  -> certified typed execution spec
  -> erased runner plan derived from the certified spec
  -> state machine executes states only
```

The certified typed execution spec is the authoritative representation. It captures state inputs,
outputs, effects, capabilities, authoring provenance, planning lineage, value provenance, stable node
identity, public terminal output shape, replay boundaries, and resume rules.

The erased runner plan exists only so the scheduler can execute work. It must be reproducibly
derivable from the certified spec. It must not carry extra semantics that are absent from the
certified spec.

This proposal is intentionally scoped to a versioned, Rust-authored typed core first. Dynamic
third-party plugins are required as a future platform capability, but they are not part of the first
implementation slice and must not weaken the typed core. Until a future plugin certification design
exists, certified execution should assume states, operations, adapters, and connectors are compiled
Rust components registered with explicit versions.

For the typed-core implementation effort, this document is the source of truth for the typed-core
rewrite. The existing repository design documents, including `docs/design.md` and
`docs/architecture.md`, must be rewritten as part of the implementation effort to match this
contract. Until those documents are updated, typed-core implementation work should cite this
proposal as the typed-core authority. The final typed-core merge gate is that the design and
architecture docs no longer present the old `DynContext`/`IoProvider`/dynamic DAG model as
authoritative for typed execution.

### Versioning And Naming Policy

Typed-core Rust APIs should version at the module boundary, not by appending version suffixes to
every public type name. The canonical developer-facing spelling is:

```rust
mfm_spec::v1::TypedExecutionSpec
mfm_spec::v1::NodeSpec
mfm_events::v1::RunStarted
mfm_events::v1::CellProduced
mfm_events::v1::side_effect::InvocationStarted
```

not:

```rust
TypedExecutionSpecV1
NodeSpecV1
RunStartedV2
CellProducedV1
SideEffectInvocationStartedV1
```

The module version names a coherent Rust contract surface. A breaking Rust API change creates a
sibling module such as `v2`, while old modules remain available for replay and migration code as
long as old certified runs are supported.

Persisted and certified identity still carries explicit versions. Module paths and Rust type names
are not replay authority and must not be the only version evidence in storage. Specs, descriptors,
events, media types, schemas, state descriptors, capability descriptors, adapter descriptors, and
canonicalizers keep explicit fields such as:

```rust
spec_version: SpecVersion
schema_version: SchemaVersion
state_version: StateVersion
capability_version: CapabilityVersion
adapter_version: AdapterVersion
operation_version: OperationVersion
canonicalization: CanonicalizationId
```

For example, the Rust type is `mfm_events::v1::RunStarted`, while the persisted event descriptor is
identified by an explicit schema id such as:

```text
schema:mfm.kernel.run_started:1:sha256-jcs-v1:<digest>
```

Kinds do not encode versions in their kind identity. `StateKind` and `StateVersion`,
`CapabilityKind` and `CapabilityVersion`, `AdapterKind` and `AdapterVersion`, and `OperationKind`
and `OperationVersion` stay as separate typed fields. This makes version drift visible in certified
specs and event payloads without making Rust API names noisy.

The rest of this RFC should be read with that policy: code snippets are intended to live under the
appropriate `v1` module unless they explicitly discuss legacy names. Any remaining `*V1`/`*V2`
spelling is legacy shorthand to be normalized during implementation, not the preferred public Rust
API.

### Crate And Module Layout

The typed core should be introduced as a new kernel namespace instead of hiding typed semantics
inside the existing `crates/machine` or `crates/sdk` APIs:

```text
crates/kernel/ids              mfm-ids
crates/kernel/canonical        mfm-canonical
crates/kernel/values           mfm-values
crates/kernel/effects          mfm-effects
crates/kernel/capabilities     mfm-capabilities
crates/kernel/program          mfm-program
crates/kernel/program-derive   mfm-program-derive
crates/kernel/spec             mfm-spec
crates/kernel/certify          mfm-certify
crates/kernel/events           mfm-events
crates/kernel/store            mfm-store
crates/kernel/runtime          mfm-runtime
crates/kernel/replay           mfm-replay
crates/kernel/test-support     mfm-kernel-test-support
```

Versioned contract modules live inside these crates. For example, `mfm-spec::v1`,
`mfm-events::v1`, `mfm-store::v1`, and `mfm-runtime::v1` contain the first certified typed-core
contract. Domain crates import those modules rather than version-suffixed type names. Shared helper
crates such as `mfm-ids`, `mfm-canonical`, and `mfm-values` may expose unversioned primitives when
the primitive semantics are stable across contract modules, but persisted descriptors still record
explicit versions.

The intended dependency direction is:

```text
mfm-ids
  -> mfm-canonical
  -> mfm-values
  -> mfm-effects / mfm-capabilities
  -> mfm-program / mfm-spec
  -> mfm-certify / mfm-events / mfm-store
  -> mfm-runtime / mfm-replay
```

Domain and product crates sit outside the kernel:

```text
crates/domains/proof
crates/domains/portfolio
crates/domains/evm
crates/states/*
crates/ops/*
crates/storages/*
crates/transports/*
crates/app
bin/*
```

The hard crate-boundary constraints are:

- `crates/kernel/*` must not depend on domain crates, binaries, the old `crates/machine`, or the
  old `crates/sdk`.
- `mfm-program-derive` must depend only on `mfm-ids`, `mfm-canonical`, `mfm-values`, and proc-macro
  dependencies. It must not depend on `mfm-program`, `mfm-runtime`, or domain crates.
- `states/*` may depend on `mfm-program`, `mfm-values`, `mfm-effects`, `mfm-capabilities`, and
  domain model crates, but not on `mfm-runtime`, `mfm-store`, binaries, or old dynamic machine APIs.
- `ops/*` assemble typed state programs. They may depend on typed states and domain/config crates,
  but not on runtime scheduling or storage implementations.
- `storages/*` implement `mfm-store`; they do not know domain semantics.
- `transports/*` implement live/replay adapters and capability backends. They do not mint
  production capability tokens directly.
- `crates/app` and `bin/*` are assembly only: decode input, build registries, choose stores and
  adapters, start/resume/replay runs, and render typed public outputs.

The old `crates/machine` and `crates/sdk` may exist while the rewrite is staged, but they are not
allowed as dependencies of the new kernel. They should be deleted, isolated, or reduced to typed
reexports once workflows move to the new system.

### Design Principles

- Rust-authored workflows should push every knowable semantic authoring error into compile-time
  validation when Rust can express it without making the API unusable.
- Operations are deterministic expansion recipes over typed planning config and typed input
  handles.
- Operations are not runtime execution units. After expansion, the state machine executes states
  only.
- A single state and a larger operation should use the same composition model, so users can build a
  workflow by choosing states directly, operations directly, or a mix of both.
- The public semantic authoring API must not expose `StateGraph`, `DependencyEdge`, `PortKey`,
  `OutputCellId`, `NodeId`, or erased runner constructors. Workflow authors create executable
  structure only through typed builders that derive dependencies from typed handles.
- States are pure by construction unless they implement an explicit effect-specific execution
  trait.
- Framework-mediated IO is capability-only. EVM, Bitcoin, other blockchain families, HTTP,
  databases, object storage, clocks, and other external systems are accessed through typed adapters
  exposed only to states whose effect/capability declarations allow them. Direct ambient IO is
  forbidden by crate-boundary policy and lints.
- Values crossing state boundaries are typed MFM values stored in typed cells, not anonymous JSON
  blobs.
- JSON context is not a semantic dataflow mechanism. JSON may appear at API, artifact, snapshot,
  public-output, and audit boundaries, but state-to-state wiring must use typed cells and handles.
- Dependency edges are derived from typed handles. Workflow authors do not manually write state
  edges.
- Operation outputs are Rust types containing typed handles. An operation cannot declare a terminal
  result without returning a handle to a value produced by its expanded state program.
- Runtime-produced values cannot change the topology of the already-certified same-run program.
- Side effects require typed intent, typed idempotency material, typed receipt, and typed replay
  verification.
- States, adapters, connectors, operation descriptors, value schemas, public output schemas, and the
  lowering algorithm are versioned contracts. Breaking semantic changes create new versions; old
  versions remain valid for replay/resume.
- Resume and replay are allowed only against the exact certified typed execution spec that produced
  the run history.
- Runtime errors should represent facts that cannot be known before execution. Invalid wiring,
  invalid lifecycle ordering, illegal adapter access, and invalid terminal shapes should be rejected
  by the compiler where practical.

### Compile-Time First Enforcement Model

The architecture should treat compile-time checking as the first correctness line for Rust-authored
workflows. Certification and runtime checks remain necessary, but they should not compensate for
semantic authoring mistakes that Rust can reject directly.

Each implementation task should record the guarantee owner in four buckets: compile-time,
certification-time, runtime/storage-time, and policy/lint-time. The table below names the
compile-time target first and compresses the remaining buckets only for readability; implementation
issues should split those remaining checks explicitly.

The intended split is:

| Guarantee | Compile-time target for Rust-authored APIs | Remaining certification/runtime/policy responsibility |
|---|---|---|
| Producer/consumer type match | `Handle<'p, 's, T>` and `IntoStateInput` reject wrong value types | Certify generated `v1::NodeSpec` and `v1::CellSpec`; runtime verifies stored cell schema and digest |
| Consuming a value before it exists | Handles are only returned by builder calls that produced cells | Runtime checks predecessor cells are complete before execution |
| Hand-authored dependency edges that lie about dataflow | `DependencyEdge` is not a public authoring API; dependencies derive from handles | Lowering verifies erased plan edges derive from the certified typed spec |
| Operation result actually produced | Operation outputs are structs containing handles returned by expansion | Certification verifies public output cells are reachable from produced cells |
| Terminal output not routed through string context keys | Public output APIs consume typed output structs of handles | Runtime renders only declared public cells and verifies public schema ids |
| Same JSON shape with different domain meaning | Different `MfmValue` Rust types and semantic type ids | Registry/certification reject inconsistent semantic/schema descriptors |
| Cross-scope value mixing | Branded program/scope handles reject accidental mixing | Persisted scope ids and provenance are verified in the certified spec and event stream |
| Optional or skipped values | `Handle<MaybeValue<T>>` is distinct from `Handle<T>` | Runtime records skip reason, producer node, schema id, and provenance |
| Runtime value used as same-run topology input | `MfmConfig` and runtime handles are separate types | New planning boundaries and child specs are certified explicitly |
| Pure/read/managed-write/side-effect capability access | Effect-specific traits expose only allowed capability types | Ambient IO bans are policy/lint; runtime injects only certified capabilities |
| Side-effect intent/idempotency/receipt shape | `SideEffectState` associated types make missing pieces fail to compile | Runtime owns idempotency key stability, ledger ambiguity, receipt validity, and replay verification |
| Portfolio fanout/fanin wiring | Typed handles guarantee batch input/output types | Planning/certification verify config-derived cardinality, canonical ordering, and duplicate keys |
| Deploy/configure/validate lifecycle order | Typestate values such as `DeployedContract` and `ConfiguredContract` encode lifecycle transitions | Generic validation of an existing deployment remains a separate explicitly typed workflow |
| Canonical JSON, no floats, schema ids | Derives reject known unsupported field types where possible | Certification computes canonical bytes/hash and validates descriptors are secret-free |
| Secret boundary | Framework-known secret wrappers fail value/config/output derive bounds | Runtime/persistence no-secret validation remains mandatory defense in depth |

This matrix intentionally does not claim that all guarantees can become compile-time guarantees.
Rust can make invalid framework-mediated authoring unrepresentable. It cannot prove external system
facts, storage integrity, replay fact availability, registry availability, or arbitrary ambient IO
inside arbitrary Rust code. Those remain certification, runtime, storage, or policy concerns.

### Problem Taxonomy Coverage

The typed-core contract must close the six problem classes named in `RFC_STATE_OPS_PROBLEM.md` with
explicit owners and tests:

| Problem class | Primary owner in Proposal 1 | Required evidence |
|---|---|---|
| Invalid topology | builder, stable ids, lowering, and store certification | duplicate/missing/cyclic structure rejection |
| Invalid interface wiring | `IntoStateInput`, `OperationInput`, `OperationOutput`, and `PublicOutputs` derives | compile-fail and lowering rejection for wrong bindings |
| Invalid semantic transition | typed handles, typestate values, effect traits, and capability-set evidence | compile-fail for wrong producer, lifecycle, or effect access |
| Invalid data shape | schema descriptors, canonicalization, no-float/no-secret derives | golden schema hashes and unsupported-shape rejection |
| Invalid data meaning | semantic type ids plus value lineage, scope, producer, config, operation lineage, and stable domain keys | lineage mismatch rejection |
| Invalid terminal shape | `PublicOutputSpec`, render node, and public-output event evidence | missing or wrong terminal output rejection |

The `typed-kernel-contract` gate must prove all six rejection classes before workflow ports begin.

### Strong Identity Types

Framework identities must not be represented in public APIs as untyped strings. Each identity family
gets a category-branded type with private fields:

```rust
pub struct Identity<K> {
    canonical_name: &'static str,
    version: &'static str,
    algorithm: DigestAlgorithm,
    digest: DigestBytes,
    _kind: PhantomData<fn(K) -> K>,
}

pub enum SemanticTypeKind {}
pub enum SchemaKind {}
pub enum StateKindKind {}
pub enum CapabilityKindKind {}
pub enum AdapterKindKind {}
pub enum OperationKindKind {}

pub type SemanticTypeId = Identity<SemanticTypeKind>;
pub type SchemaId = Identity<SchemaKind>;
pub type StateKind = Identity<StateKindKind>;
pub type CapabilityKind = Identity<CapabilityKindKind>;
pub type AdapterKind = Identity<AdapterKindKind>;
pub type OperationKind = Identity<OperationKindKind>;
```

Canonical names and, where applicable, manually assigned versions exist as human-readable
descriptor inputs. APIs accept category-branded identity types, never bare `&str` or `String`, so a
`SchemaId` cannot be passed as a `SemanticTypeId` and a `CapabilityKind` cannot be passed as a
`StateKind`.

Identity construction is framework-controlled:

- derive macros and framework macros may create `const` identities after validating grammar,
  category, versioning policy, and descriptor hash
- runtime may deserialize persisted identities only through checked constructors that verify
  category prefix, grammar, and digest consistency
- domain crates may name identities through generated constants, but may not construct arbitrary
  identities from strings

The grammar is intentionally stricter than arbitrary UTF-8 and category-specific:

```text
SemanticTypeId = semantic:<namespace>:<name>:<version>:<algorithm>:<digest>
SchemaId       = schema:<schema_name>:<schema_version>:<algorithm>:<digest>
StateKind      = state:<namespace>:<name>:<algorithm>:<digest>
CapabilityKind = capability:<namespace>:<name>:<algorithm>:<digest>
AdapterKind    = adapter:<namespace>:<name>:<algorithm>:<digest>
OperationKind  = operation:<namespace>:<name>:<algorithm>:<digest>
DescriptorId   = descriptor:<algorithm>:<digest>

namespace/name/version use [a-z0-9][a-z0-9._/-]*
algorithm = "sha256-jcs-v1" initially
digest = hex sha256 digest of the category-specific canonical descriptor identity
```

Kinds that have explicit `*Version` fields do not also encode that version into the `*Kind`
identity. This keeps `StateKind` and `StateVersion`, `CapabilityKind` and `CapabilityVersion`, and
adapter/operation kind-version pairs independently checkable in specs, events, and policy.

Associated constants such as `MfmValue::SEMANTIC_ID`, `StateSpec::KIND`, and
`CapabilitySpec::KIND` therefore remain compile-time visible without weakening type safety into
stringly-typed APIs.

### MFM Values

Every value that can cross a state boundary must implement a core value trait:

```rust
pub trait MfmValue:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static
{
    const SEMANTIC_ID: SemanticTypeId;

    fn schema_descriptor() -> SchemaDescriptor;

    fn schema_id() -> SchemaId {
        SchemaId::derive(&Self::schema_descriptor())
    }
}
```

The semantic type id names the stable kind of value. The schema id names its stable serialized
shape. Neither one is instance lineage. Two values with the same JSON representation but different
stable domain meaning must be different Rust types and semantic type ids. Two values with the same
Rust type, semantic type id, schema id, and scope may still be different domain instances; those
instance relationships are carried by certified value lineage, not by `SemanticTypeId`.

Domain crates must use derives for `MfmValue`, `MfmConfig`, and public output implementations.
Derive macros generate schema descriptors, field-level schema requirements, no-float/no-secret
descriptor checks where statically knowable, and compile-time trait bounds for every persisted
field. Manual implementations are unsupported outside explicit framework allowlists and must be
blocked by source-boundary checks or rejected by certification when descriptor provenance is not
derive-generated or allowlisted.

### Schema Id Derivation And Manual Migration

Schema ids should be derived from canonical schema descriptors, not hand-written arbitrary strings.
Every persisted `MfmValue`, `MfmConfig`, and public output type must expose a schema descriptor that
is itself canonical-json-hashable and secret-free.

At minimum, a schema descriptor should identify:

- schema kind: value, planning config, or public output
- semantic type id where applicable
- schema name
- manually assigned schema version
- canonical serialized shape
- canonicalization rules
- redaction/no-secret policy
- owner crate and type name for audit/debug

The descriptor must distinguish identity-bearing fields from audit-only fields. Including owner
crate or Rust type name in the schema hash makes harmless refactors schema-breaking; excluding them
from all persisted evidence weakens auditability. The recommended split is:

- identity fields: schema kind, semantic type id, schema name, schema version, canonical serialized
  shape, canonicalization rules, compatibility policy, and redaction/no-secret policy
- audit fields: owner crate, Rust type path, derive macro version, and source package/build
  provenance

The schema id is derived from the canonical bytes of the descriptor identity, not from audit-only
metadata:

```text
SchemaId = schema:<schema_name>:<schema_version>:<algorithm>:<digest(canonical_schema_identity)>
```

The initial algorithm should be the same canonical JSON digest family used elsewhere in MFM, with
an explicit algorithm/version prefix so the derivation itself can evolve later.

Migration policy is manual. A schema change never silently replaces or upgrades an existing schema
id. Breaking serialized-shape or semantic changes require a new manually assigned schema version,
which produces a new derived schema id. Existing certified specs continue to reference the old
schema id and old state/adapter/connector versions. If data must move from an old schema to a new
schema, that transition must be modeled as an explicit versioned migration state or operation whose
input and output types make the conversion visible in the certified spec.

No secret-bearing type may implement `MfmValue`, `MfmConfig`, or public output traits. Because
stable Rust does not provide a general negative trait bound such as "not secret", MFM should use a
positive allowlist model: framework-known secret wrappers do not implement value/config/output
traits, derive macros reject fields that are known secret wrappers, and persistence layers continue
to perform no-secret validation as defense in depth.

The `typed-kernel-contract` slice should use a strict `v1::SchemaDescriptor` grammar. The descriptor is
itself an MFM value whose identity portion is hash-defining and whose audit portion is persisted but
not included in the schema id:

```rust
pub struct SchemaDescriptor {
    pub descriptor_version: u32,
    pub canonicalization: CanonicalizationId,
    pub identity: SchemaIdentity,
    pub audit: SchemaAudit,
}
```

```text
SchemaId = schema:<schema_name>:<schema_version>:sha256-jcs-v1:<sha256(canonical_json(identity))>
```

The v1 identity grammar supports:

- unit, bool, string, bytes encoded as base64url without padding, signed/unsigned integers, and
  decimal strings
- option, vec, non-empty vec, tuple, named struct, enum, and `BTreeMap<String, V>`
- generic constructors with argument schema and semantic ids

The v1 grammar rejects floats, `usize`, `isize`, `HashMap`, untagged enums, `serde(flatten)`, skip
attributes, asymmetric serialize/deserialize renames, custom serde functions, raw secret wrappers,
and opaque `serde_json::Value`.

Allowed enum tagging is externally tagged, internally tagged for struct-like variants, and
adjacently tagged. `serde(rename)` and `rename_all` are allowed only after derives resolve them to
concrete wire names; duplicate resolved names are compile errors. `serde(default)` is allowed only
when the field type implements `MfmDefault`; custom default functions are rejected in v1 unless a
future descriptor-producing default provider exists.

Decimal strings must reject `+`, exponent syntax, leading zeroes, trailing decimal points,
NaN/Infinity, and negative zero. The accepted forms are:

```text
variable decimal =
  0
  | [1-9][0-9]*(\.[0-9]*[1-9])?
  | -(0\.[0-9]*[1-9]|[1-9][0-9]*(\.[0-9]*[1-9])?)

fixed decimal =
  (0|[1-9][0-9]*)\.[0-9]{scale}
  | -<fixed decimal with at least one non-zero digit>
```

Manual descriptor, value, config, and public-output implementations are not part of the normal
platform extension model. The default policy is derive-only for domain crates. Framework-owned
kernel crates may provide hand-written implementations for closed generic constructors such as
`MaybeValue<T>` and `ArtifactRef<T>`, but external workflow crates must not implement persisted
descriptor traits manually. If a future exception is required, it must be framework-owned,
feature-gated, covered by golden descriptor tests, and rejected by certification unless descriptor
provenance is allowlisted. `unsafe` should be used only for invariants that are genuinely
memory-safety-like; descriptor integrity should normally be enforced through framework-only
evidence, certification, and tests.

Trait sealing follows the same authority split. Framework-authority traits and evidence tokens stay
sealed, including capability evidence, registration evidence, bridge session evidence, and certified
runner binding evidence. Derive-backed descriptor and traversal traits are not authority by
themselves; they may be implemented by derives in downstream crates, while manual implementations
outside framework allowlists are blocked by source-boundary checks and certification.

Examples:

```rust
pub struct PreparedSources { /* ... */ }
pub struct ResolvedSubjects { /* ... */ }
pub struct PinnedViews { /* ... */ }
pub struct ResolvedValuations { /* ... */ }
pub struct ObservationBatchOutput { /* ... */ }
pub struct MergedObservations { /* ... */ }
pub struct PortfolioSnapshot { /* ... */ }
pub struct PortfolioReport { /* ... */ }

pub struct ProofFact { /* ... */ }
pub struct ProofSideEffectIntent { /* ... */ }
pub struct ProofSideEffectReceipt { /* ... */ }
pub struct ProofSideEffectResult { /* ... */ }
pub struct ProofOutput { /* ... */ }

pub struct DcvBuiltConfig { /* ... */ }
pub struct DeployConfig { /* ... */ }
pub struct ConfigureConfig { /* ... */ }
pub struct ValidateConfig { /* ... */ }
pub struct DeployedContract { /* ... */ }
pub struct ConfiguredContract { /* ... */ }
pub struct ValidationReport { /* ... */ }
```

Typed artifact references should also carry the value type:

```rust
pub struct ArtifactRef<T: MfmValue> {
    pub id: ArtifactId,
    pub digest: ContentDigest,
    pub schema_id: SchemaId,
    pub semantic_type_id: SemanticTypeId,
    _value: PhantomData<fn(T) -> T>,
}
```

JSON still exists at boundaries: CLI/API input, manifests, artifacts, facts, snapshots, and audit
records. But JSON is the serialization format, not the semantic wiring contract.

### Secret Boundary

Secrets must remain below the state-machine boundary. Keystore secrets, private keys, mnemonics,
password material, raw signing keys, decrypted bytes, and similar secret-bearing values must not be
MFM values, typed cells, artifacts, facts, events, public outputs, or error details.

State programs may reference secret-bearing systems only through non-secret typed references and
capabilities, such as:

```rust
pub struct WalletRef { /* non-secret wallet label or account id */ }
pub struct KeyLabel { /* non-secret keystore entry label */ }
pub struct SignerRef { /* non-secret signing authority reference */ }
```

The keystore layer should be modeled as a typed adapter/capability boundary. States may ask a
keystore capability to sign, decrypt, or unlock by using non-secret references and non-secret
configuration. They must never receive the secret material itself. This makes secret leakage a type
and boundary violation for framework-mediated secret access, not a convention that state authors must
remember.

Secret-bearing wrapper types should avoid `Serialize`, `MfmValue`, `MfmConfig`, and public output
implementations entirely. If a secret type needs `Debug`, it must be redacted. Keystore capabilities
should return non-secret signatures, receipts, protected artifact references, or typed authority
references, never raw private keys, mnemonics, passwords, or decrypted secret bytes.

### Typed Planning Config

The architecture must distinguish planning config from runtime values.

```rust
pub trait MfmConfig:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static
{
    fn schema_descriptor() -> SchemaDescriptor;

    fn schema_id() -> SchemaId {
        SchemaId::derive(&Self::schema_descriptor())
    }

    fn validate(&self) -> Result<(), ConfigError>;
}
```

Typed configs are deterministic planning inputs. They are decoded before expansion, validated before
lowering, canonicalized for hashing, and safe to persist in manifests and certified specs. Hashed
config structures must use canonical serialization and must not contain floats. Typed configs may
decide which states are generated.

Certified specs must retain enough config material to recreate every state runner. A node-level
config hash alone is not sufficient. Each node must carry either canonical config bytes or a
content-addressed reference to those bytes, plus the config schema id used to decode them.

Runtime values are produced by states and referenced through typed handles. A runtime value cannot
change the already-certified topology of the same run. If a later state needs data produced by an
earlier state, that data must be part of the later state's typed input, not the later state's
planning config.

If a runtime-produced value is needed to choose future topology, MFM must create an explicit
planning boundary. Acceptable boundaries include:

- compile a new certified child spec
- start a child run
- resume into a newly certified continuation
- split the workflow into two user-visible planning phases

Using a runtime handle to mutate planner config after lowering is forbidden.

### Typed Handles

State outputs are referenced through typed handles:

```rust
#[derive(Clone, Copy)]
pub struct Handle<'program, 'scope, T: MfmValue> {
    cell: CellId,
    _program: PhantomData<fn(&'program ()) -> &'program ()>,
    _scope: PhantomData<fn(&'scope ()) -> &'scope ()>,
    _value: PhantomData<fn(T) -> T>,
}
```

The `T` parameter enforces producer/consumer type compatibility. A state that requires
`Handle<ResolvedSubjects>` cannot receive `Handle<PinnedViews>`.

Handles are small immutable references to planned output cells. They can be copied or cloned freely
inside expansion code without copying the produced runtime value.

Handle fields and constructors must be private. Handles are minted only by the typed builder when it
adds a producer node or by explicit framework bridge APIs. Workflow authors must not be able to
forge a handle from a raw `CellId`, because forged handles would reintroduce the same semantic gap
as string context keys.

The `'program` brand prevents mixing handles from unrelated typed programs. The `'scope` brand
prevents accidentally mixing values from different semantic workflow instances inside the same
program. This matters when two values have the same Rust type but belong to different execution
segments.

The brand should be invariant, not merely documentary. The concrete branding representation can
change, but the API must include a generative program/scope construction pattern that prevents two
independently created scopes from being unified accidentally by inference.

The public build API must not return arbitrary handle-bearing values from a generative closure. Root
public outputs are bound while the program and scope brands are still in scope:

```rust
pub fn build_root<F>(
    root_key: ScopeKey,
    f: F,
) -> Result<TypedProgramDraft, PlanError>
where
    F: for<'p, 'root> FnOnce(
        &mut RootBuilder<'p, 'root>,
    ) -> Result<RootBound, PlanError>;

pub struct RootBound {
    public_output_spec: v1::PublicOutputSpec,
    // private marker; callers cannot construct this directly
}

impl<'p, 's> RootBuilder<'p, 's> {
    pub fn scope(&mut self) -> &mut ScopeBuilder<'p, 's>;

    pub fn seed<T: MfmValue>(
        &mut self,
        key: SeedKey,
        value: CanonicalSeed<T>,
    ) -> Result<Handle<'p, 's, T>, PlanError>;

    pub fn bind_public_outputs<P>(
        &mut self,
        key: PublicOutputKey,
        outputs: &P,
    ) -> Result<RootBound, PlanError>
    where
        P: PublicOutputs<'p, 's>;
}
```

`build_root` returns an unbranded `TypedProgramDraft`. The closure returns `RootBound`, not a handle
or operation output struct. This prevents branded handles from escaping into ordinary Rust values
while still allowing the builder to extract an unbranded `v1::PublicOutputSpec`.

External typed values enter a root program only through seed cells. Seeds represent decoded manifest
or launch inputs that become typed cells without a producer state. A seed cell has a cell id,
semantic type id, schema id, digest, scope id, redaction policy, and mandatory launch artifact
evidence, and it is bound to the certified spec or manifest. Seeds are immutable, canonical,
no-float/no-secret checked, and cannot be created from a runtime value in the same run. Duplicate
seed keys are planning errors; missing seed artifacts or seed digest mismatch rejects run start.

For example, snapshot assembly can require all inputs to belong to the same portfolio execution
scope:

```rust
fn assemble_snapshot<'p, 's>(
    subjects: Handle<'p, 's, ResolvedSubjects>,
    views: Handle<'p, 's, PinnedViews>,
    observations: Handle<'p, 's, MergedObservations>,
) -> Handle<'p, 's, PortfolioSnapshot>;
```

Passing values from another portfolio execution scope must require an explicit bridge state or
operation. The bridge creates a new value with new provenance, making the semantic transition
visible in the program.

Scope ids must be persisted in the certified spec and value provenance records. This matters for
workflows with the same Rust type in multiple semantic scopes, such as two portfolio executions in
one program, two chains producing the same `DeployedContract` type, or staging and production
deployments with identical schemas.

Child scopes expose certified bridge operations, not raw exported handles:

```rust
impl<'p, 'parent> ScopeBuilder<'p, 'parent> {
    pub fn child_scope<R>(
        &mut self,
        key: ScopeKey,
        f: impl for<'child> FnOnce(
            &mut ChildScopeBuilder<'p, 'parent, 'child>,
        ) -> Result<Bridged<'p, 'parent, R>, PlanError>,
    ) -> Result<R, PlanError>;
}

pub struct Bridged<'p, 'parent, R> {
    value: R,
    // private marker; framework constructs this only from bridge evidence
    _brand: PhantomData<fn(&'p (), &'parent ()) -> (&'p (), &'parent ())>,
}

impl<'p, 'parent, 'child> ChildScopeBuilder<'p, 'parent, 'child> {
    pub fn export_to_parent<T: MfmValue>(
        &mut self,
        key: StableNodeKey,
        value: Handle<'p, 'child, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'p, 'parent, T>, PlanError>;

    pub fn import_from_parent<T: MfmValue>(
        &mut self,
        key: StableNodeKey,
        value: Handle<'p, 'parent, T>,
        policy: BridgePolicy,
    ) -> Result<Handle<'p, 'child, T>, PlanError>;

    pub fn bridge_to_parent<R>(
        &mut self,
        value: R,
    ) -> Result<Bridged<'p, 'parent, R>, PlanError>
    where
        R: BridgeableToParent<'p, 'parent>;
}
```

The child closure may return only parent-branded values wrapped in `Bridged`. `Bridged` is produced
only by `ChildScopeBuilder::bridge_to_parent` after all returned handles have explicit import/export
bridge evidence created by the current child-scope invocation. This keeps child-branded handles from
escaping through ordinary return values while still allowing scoped composition to produce
parent-visible results.

The framework verifies returned values through live bridge evidence, not by trusting public bridge
records:

```rust
pub trait BridgeableToParent<'p, 'parent> {
    fn bridge_evidence(&self) -> Vec<BridgeEvidence<'p, 'parent>>;
}

pub struct BridgeEvidence<'p, 'parent> {
    bridge_ref: BridgeRef,
    // private active-session token; not serializable and not constructible by domain crates
    _brand: PhantomData<fn(&'p (), &'parent ()) -> (&'p (), &'parent ())>,
}

pub struct BridgeRef {
    pub source_scope_id: ScopeId,
    pub target_scope_id: ScopeId,
    pub source_cell_id: CellId,
    pub target_cell_id: CellId,
    pub semantic_type_id: SemanticTypeId,
    pub schema_id: SchemaId,
    pub bridge_node_id: NodeId,
}
```

The framework implements `BridgeableToParent` only for handles, tuples, derive-backed structs, and
closed framework wrappers whose handles are already parent-branded or have explicit bridge nodes.
Domain crates do not manually implement this trait in v1; derives may generate it for handle-only
structs. `BridgeRef` is persisted, serializable, and hashable evidence. `BridgeEvidence` is live
authority owned by the active child-scope builder invocation and carries a private bridge session
token. `bridge_to_parent` must reject evidence whose session token, source scope, target scope,
source cell, target cell, or bridge node id was not created by the active child-scope builder
invocation. If the child closure attempts to return a child-branded handle without bridge evidence,
the code fails to compile or the builder rejects it before certification.

The `v1` bridge policy is same-run, same-value, no-transform:

```rust
pub struct BridgeNodeSpec {
    pub node_id: NodeId,
    pub stable_key: StableNodeKey,
    pub source_scope_id: ScopeId,
    pub target_scope_id: ScopeId,
    pub source_cell_id: CellId,
    pub target_cell_id: CellId,
    pub semantic_type_id: SemanticTypeId,
    pub schema_id: SchemaId,
    pub bridge_kind: BridgeKind,
    pub policy: BridgePolicy,
    pub provenance: BridgeProvenance,
}
```

Transforming bridges are ordinary states. Bridge completion is event-defined like any other typed
cell completion. Same-value import/export bridges are framework nodes in `v1::TypedExecutionSpec`,
not erased-plan-only helper edges; their node ids, source and target scopes, source and target
cells, policy, provenance, and deterministic predecessor edges are part of the certified spec hash.
Certified bridge nodes are the only authority for cross-scope same-value movement; public
`BridgeRef` values alone cannot authorize a bridge.

### Typed Optionality And Skip Cells

Optional runtime paths must be represented explicitly in the typed program. A plain missing context
key, absent JSON value, or raw `Option<T>` boundary is not enough for audit or replay.

The typed optionality API should be `MaybeValue<T>`:

```rust
pub enum MaybeValue<T: MfmValue> {
    Produced(T),
    Skipped(SkipReason),
}

pub struct SkipReason {
    pub code: SkipCode,
    pub explanation: String,
}
```

`MaybeValue<T>` is the state-boundary shape. Internal implementation may use `Option<T>` where
appropriate, but typed cells and state inputs must expose optionality through `MaybeValue<T>` so the
certified spec and event stream can preserve skip provenance. A skipped value must have a typed cell
identity, semantic type id, schema id, producer node, and reason. Downstream states must declare
whether they accept a produced value only or `MaybeValue<T>`.

The distinction must be visible in handles: `Handle<'p, 's, T>` and
`Handle<'p, 's, MaybeValue<T>>` are not interchangeable. A produced-only consumer cannot be wired to
a skipped-or-produced cell without an explicit state or operation that handles the skip semantics and
returns a new typed value.

`MaybeValue<T>` and `ArtifactRef<T>` are framework-provided generic `MfmValue` constructors. A
`Handle<T>` has cell terminal policy `ProducedOnly`; a `Handle<MaybeValue<T>>` has terminal policy
`MaybeSkipped`. A produced optional cell stores canonical bytes for `MaybeValue::Produced(T)`. A
skipped optional cell emits `v1::events::CellSkipped` with `SkipReason` and no value artifact, and runtime
materialization yields `MaybeValue::Skipped(reason)`. `CellSkipped` for a produced-only cell is a
certification/history error. An `ArtifactRef<T>` is a typed value reference, not a loose artifact id:
runtime materialization must verify its digest, schema id, and semantic type id against `T`.

### States

A state is the only executable unit. Each state declares its configuration, input, output, effect,
capability set, kind, and version:

```rust
pub trait StateSpec {
    type Config: MfmConfig;
    type Input: StateInput;
    type Output: MfmValue;
    type Effect: EffectSpec;
    type Caps: CapabilitySet;

    const KIND: StateKind;
    const VERSION: StateVersion;

    fn new(config: Self::Config) -> Result<Self, PlanError>
    where
        Self: Sized;
}
```

`StateSpec` alone is not enough to enter the typed program. It is descriptive, not execution
authority. Domain crates implement public author traits such as `PureState`, `ReadState`,
`ManagedWriteState`, or `SideEffectState`; a framework registry converts those implementations into
a `RegisteredState<S>` only after it has validated descriptor evidence, effect class, capability set,
runner shape, and executable factory identity. Planning may use only registered state evidence
obtained from the builder's registry or an equivalent framework-owned registration token.

```rust
pub struct RegisteredState<S: StateSpec> {
    descriptor: StateDescriptorIdentity,
    runner: RunnerKind,
    evidence: StateRegistrationEvidence<S>,
    _state: PhantomData<fn(S) -> S>,
}

pub struct StateRegistrationEvidence<S: StateSpec> {
    // private fields; constructed only by framework registration code
    _state: PhantomData<fn(S) -> S>,
}

pub trait EffectRunner<S: StateSpec>: sealed::Sealed {
    fn runner_kind() -> RunnerKind;
}
pub trait StateRegistry {
    fn registered_state<S>(&self) -> Result<RegisteredState<S>, RegistryError>
    where
        S: StateSpec;
}
```

`EffectRunner` and `StateRegistrationEvidence` are not downstream extension points. They are emitted
or implemented by framework crates only. This avoids relying on proc macros to bypass sealed private
traits from downstream crates. A type that declares `StateSpec` but has not been registered as a
valid executable state cannot be planned. Certification still compares the registered descriptor
identity against the persisted descriptor and rejects any mismatch.

Runtime input types are separate from authoring bindings:

```rust
pub trait StateInput: Send + Sync + 'static {
    fn input_descriptor() -> InputDescriptor;
}

pub trait IntoStateInput<'p, 's, I: StateInput> {
    fn into_binding(self) -> Result<InputBinding<I>, PlanError>;
}

pub struct InputBinding<I> {
    descriptor: InputDescriptor,
    root: InputBindingNode,
    digest: ContentDigest,
    _input: PhantomData<I>,
}

pub struct InputBindingSpec {
    pub input_schema_id: SchemaId,
    pub input_descriptor_id: InputDescriptorId,
    pub root: InputBindingNode,
    pub digest: ContentDigest,
}

pub enum InputBindingNode {
    Unit,
    Cell {
        field_path: FieldPath,
        cell_id: CellId,
        semantic_type_id: SemanticTypeId,
        schema_id: SchemaId,
        value_lineage: ValueLineageRef,
        required_terminal: RequiredTerminal,
    },
    Tuple {
        elements: Vec<InputBindingNode>,
    },
    Struct {
        fields: Vec<NamedInputBinding>,
    },
    Vec {
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
    },
    NonEmptyVec {
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
    },
}
```

`InputBinding<I>` is a canonical tree, not an erased bag. `v1::InputBindingSpec` is the persisted
lowered form. Its digest is `sha256-jcs-v1(canonical_json(root))`. Supported v1 conversions are:

- `()` to `()`
- `Handle<'p, 's, T>` to `T`
- tuples of handles to tuples of values, macro-generated to a fixed arity initially 12
- `Vec<Handle<'p, 's, T>>` to `Vec<T>`, preserving explicit vector order and recording ordering
  evidence, only when the input descriptor declares no same-type lineage constraint
- `DomainKeyedHandles<'p, 's, K, T>` to `Vec<T>` or a domain collection, preserving canonical
  `StableDomainKey` ordering and lineage evidence
- `NonEmptyHandles<'p, 's, T>` to `NonEmpty<T>`
- `Handle<'p, 's, MaybeValue<T>>` to `MaybeValue<T>`
- `Handle<'p, 's, ArtifactRef<T>>` to `ArtifactRef<T>`
- derive-generated domain handle structs to domain runtime input structs

There is no implementation for `serde_json::Value`, context keys, raw `OutputCellId`, or erased
dynamic input values. Config-derived fanout must use canonical `StableDomainKey` sorting.

Runtime materializes `Self::Input` only from the certified binding tree. Vector ordering evidence is
either explicit author order or `StableDomainKey` canonical order. `NonEmptyVec` rejects empty input
at planning time. Reordered source maps must produce identical binding digests.

`StateSpec` describes the state. Execution must be split by effect class. There must not be one
universal execution trait that hands every state a context object, IO provider, clock, artifact
store, and event recorder.

State input types may be a single value, tuples of values, typed collections, or domain-specific
input structs:

```rust
pub struct ObserveBatchInput {
    pub subjects: ResolvedSubjects,
    pub views: PinnedViews,
    pub valuations: ResolvedValuations,
}
```

During typed expansion, inputs are handles. During execution, the runtime materializes those handles
into typed values from prior output cells.

For every derive-backed domain input struct, the derive must generate a handle-side binding struct
with the same field names and a deterministic field-path descriptor:

```rust
pub struct ObserveBatchInputHandles<'p, 's> {
    pub subjects: Handle<'p, 's, ResolvedSubjects>,
    pub views: Handle<'p, 's, PinnedViews>,
    pub valuations: Handle<'p, 's, ResolvedValuations>,
}

impl<'p, 's> IntoStateInput<'p, 's, ObserveBatchInput>
    for ObserveBatchInputHandles<'p, 's>
{
    fn into_binding(self) -> Result<InputBinding<ObserveBatchInput>, PlanError> {
        /* derive-generated canonical binding tree */
    }
}
```

The v1 lifting rules are:

- `T: MfmValue` becomes `Handle<'p, 's, T>`
- `MaybeValue<T>` becomes `Handle<'p, 's, MaybeValue<T>>`
- `ArtifactRef<T>` becomes `Handle<'p, 's, ArtifactRef<T>>`
- `Vec<T>` becomes `Vec<Handle<'p, 's, T>>` with explicit ordering evidence
- non-empty collections become `NonEmptyHandles<'p, 's, T>`
- nested structs become nested handle structs with canonical field paths
- `#[mfm(rename = "...")]` may rename a field after duplicate-name validation

There is no v1 lifting for `HashMap`, `serde_json::Value`, raw `CellId`, raw `OutputCellId`,
runtime context keys, or erased dynamic input values.

### Versioned State Descriptors And Runner Rehydration

A certified spec cannot persist Rust trait objects. It must persist enough information for the
runtime to rehydrate the exact versioned runner that was certified.

Every executable state kind must therefore have a registered descriptor:

```rust
pub struct StateDescriptor {
    pub identity: StateDescriptorIdentity, // hash-defining
    pub audit: StateDescriptorAudit,       // stored, not hash-defining
}
```

The runtime must resolve `(StateKind, StateVersion)` through a state registry before lowering to an
erased runner. State descriptors are immutable semantic contracts. If a state's config shape, input
contract, output contract, effect behavior, capability needs, idempotency behavior, or replay
semantics changes incompatibly, the state version must change.

Certified node specs must reference canonical state config bytes or a content-addressed config
artifact, not only a config hash. Hashes prove identity; bytes are required to reconstruct the
runner, audit the planned behavior, and verify replay. Old state versions must remain resolvable as
long as runs certified against them may need replay/resume.

Certification compares registry identity against derive-generated descriptor evidence for
`S::Config`, `S::Input`, `S::Output`, `S::Effect`, `S::Caps`, the side-effect contract where
applicable, runner ABI version, and state kind/version. Descriptor mismatch is a certification
error, never a runtime warning.

Runner factory ABI is:

```rust
pub trait RunnerFactory {
    fn state_descriptor(&self) -> StateDescriptorIdentity;

    fn instantiate(
        &self,
        canonical_config: CanonicalJsonBytes,
        expected_config_ref: &v1::ConfigRef,
    ) -> Result<Box<dyn ErasedNodeRunner>, RegistryError>;
}
```

The factory must reject config bytes whose schema id, canonical digest, media type, or byte length
does not match the certified node spec's `v1::ConfigRef`. The `v1` module uses one output cell per state. Multiple
logical values are represented as one output struct or explicit projection states; projection states
are ordinary runtime states that preserve source-cell provenance in their node provenance.

### Effects, Purity, And Capabilities

States are pure unless they explicitly opt into an effect-specific execution trait.

```rust
pub enum Pure {}
pub enum ReadExternal {}
pub enum ManagedPlatformWrite {}
pub enum ApplySideEffect {}
```

Pure states receive no external capability:

```rust
pub enum NoCaps {}

pub trait PureState: StateSpec<Effect = Pure, Caps = NoCaps> {
    fn run(&self, input: Self::Input) -> Result<Self::Output, StateError>;
}
```

Read states may observe external systems through declared read capabilities. Reads produce recorded
facts and may be replayed from those facts:

```rust
#[async_trait]
pub trait ReadState: StateSpec<Effect = ReadExternal> {
    async fn run(
        &self,
        input: Self::Input,
        caps: &Self::Caps,
    ) -> Result<Self::Output, StateError>;
}
```

Managed platform write states may write through configured MFM-certified persistence and output
surfaces for the current `run_id`, `spec_hash`, `node_id`, and `attempt_id`. These surfaces may be
external systems from the platform's point of view, such as Postgres, local filesystems, S3, or an
artifact service. The distinction is not "inside the process" versus "outside the process"; the
distinction is MFM-managed platform persistence versus external domain mutation.

Managed platform writes may stage or write content-addressed artifacts, diagnostics, rendered
public outputs, retention refs, and other framework-owned evidence. They must not mutate external
domain systems such as chains, user infrastructure, external databases, message queues, or APIs
outside MFM's certified storage/output authority. They also do not append arbitrary events directly:
the scheduler and store still own typed commit authority.

```rust
#[async_trait]
pub trait ManagedWriteState: StateSpec<Effect = ManagedPlatformWrite> {
    async fn run(
        &self,
        input: Self::Input,
        caps: &Self::Caps,
    ) -> Result<Self::Output, StateError>;
}
```

The first managed platform capability set should include:

```rust
pub trait ArtifactWriteCap {
    async fn write_value_artifact<T: MfmValue>(
        &self,
        role: ArtifactRole,
        value: &T,
    ) -> Result<StagedArtifactRef<T>, StateError>;

    async fn write_bytes_artifact(
        &self,
        role: ArtifactRole,
        media_type: MediaType,
        bytes: &[u8],
    ) -> Result<StagedArtifactRef<OpaqueArtifact>, StateError>;
}

pub trait PublicOutputRenderCap {
    async fn write_rendered_public_output(
        &self,
        public_schema_id: SchemaId,
        bytes: CanonicalJsonBytes,
    ) -> Result<StagedArtifactRef<RenderedPublicOutput>, StateError>;
}

pub struct StagedRetentionRefs {
    pub refs: Vec<RetentionRef>,
    pub reason: RetentionReason,
}

pub trait RetentionCap {
    fn retain(&self, refs: Vec<RetentionRef>, reason: RetentionReason)
        -> Result<StagedRetentionRefs, StateError>;
}

pub trait DiagnosticArtifactCap {
    async fn write_redacted_diagnostic(
        &self,
        diagnostic: RedactedDiagnostic,
    ) -> Result<StagedArtifactRef<RedactedDiagnostic>, StateError>;
}
```

`StagedArtifactRef<T>` and `StagedRetentionRefs` are not terminal evidence by themselves. The
scheduler/store must validate staged artifact refs and staged retention refs and bind them to typed
commit payloads before they affect replay, resume, retention, or public completion. This preserves
the rule that platform persistence is capability-mediated while the typed store remains the event
authority.

This category prevents artifact publication, retention projection, and public-output rendering from
being mislabeled as pure computation or as external domain side effects. It does not mean those
storage systems are "internal" or ambient; they are still capability-mediated configured systems.

Effects are framework-sealed. Capability descriptor registration is extensible for Rust-authored
third-party crates, but production token construction remains private:

```rust
pub trait EffectSpec: sealed::Sealed {
    const KIND: EffectKind;
}

pub trait CapabilitySpec: Send + Sync + 'static {
    const KIND: CapabilityKind;
    const VERSION: CapabilityVersion;
    const ROLE: CapabilityRole;

    fn descriptor() -> CapabilityDescriptor;
}

pub trait CapabilitySet: sealed::Sealed + Send + Sync + 'static {
    fn descriptor() -> CapabilitySetDescriptor;
}

pub trait CapabilitySetFor<E: EffectSpec>: CapabilitySet {}

pub enum CapabilityRole {
    ReadExternal,
    ManagedPlatformWrite,
    Support,
    ExternalMutationAuthority,
}
```

The v1 capability rules are:

- `Pure` receives `NoCaps` only.
- `ReadExternal` may receive read capabilities and support capabilities that cannot mutate external
  systems.
- `ManagedPlatformWrite` may receive MFM-certified platform persistence/output capabilities only.
- `ApplySideEffect` receives exactly one `ExternalMutationAuthority` plus declared support/read
  capabilities.

`CapabilitySetFor<E>` is framework-owned evidence. Domain crates can name capability token types in
`StateSpec::Caps`, but they do not manually implement `CapabilitySetFor<E>`. The framework provides
closed tuple implementations up to a fixed arity and derive-generated evidence only when the role
rules are valid for the effect. Registration of a state requires:

```rust
S: StateSpec,
S::Effect: EffectSpec,
S::Caps: CapabilitySetFor<S::Effect>,
// the framework state registry returns RegisteredState<S>
```

This is the compile-time gate for framework-mediated capability access. Certification repeats the
same rule against the persisted `CapabilitySetDescriptor` so descriptor drift or any future manual
escape hatch cannot widen a state's access.

For example, an EVM transaction side-effect state may declare one EVM transaction submitter as the
external mutation authority, plus signer, keystore, chain metadata, and RPC read support
capabilities. Support capabilities must not expose external mutation methods.

Production capability tokens have private fields and are minted only by the live broker from
certified descriptors. Replay brokers cannot mint mutation authority tokens. Test mocks are
registered through test-support brokers and adapter registries, not through public token
constructors.

This is a compile-time guarantee for framework-mediated capabilities: the state method signature
does not contain a generic context, generic IO provider, generic recorder, or broad capability bag.
It is not a complete proof that arbitrary Rust code performs no ambient IO. State and operation
crates must also use lint and crate-boundary policy, such as denying direct filesystem, environment,
clock, randomness, process, and network APIs outside designated transport/capability crates.

Side-effect states are split into typed intent construction, submission/recovery, and deterministic
output construction. The side-effect runner owns the durable protocol; state code owns deterministic
domain transformation:

```rust
#[async_trait]
pub trait SideEffectState: StateSpec<Effect = ApplySideEffect> {
    type Intent: MfmValue;
    type IdempotencyInput: MfmValue;
    type Submission: MfmValue;
    type Receipt: MfmValue;
    type Confirmation: MfmValue;

    fn prepare_intent(&self, input: &Self::Input) -> Result<Self::Intent, StateError>;

    fn idempotency_input(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
    ) -> Result<Self::IdempotencyInput, StateError>;

    async fn submit(
        &self,
        intent: &Self::Intent,
        key: &IdempotencyKey<Self::IdempotencyInput>,
        caps: &Self::Caps,
    ) -> Result<SubmissionEvidence<Self::Submission>, StateError>;

    async fn recover_submission(
        &self,
        intent: &Self::Intent,
        key: &IdempotencyKey<Self::IdempotencyInput>,
        caps: &Self::Caps,
    ) -> Result<SubmissionRecovery<Self::Submission>, StateError>;

    async fn recover_receipt(
        &self,
        submission: &Self::Submission,
        caps: &Self::Caps,
    ) -> Result<ReceiptRecovery<Self::Receipt>, StateError>;

    async fn recover_confirmation(
        &self,
        receipt: &Self::Receipt,
        caps: &Self::Caps,
    ) -> Result<ConfirmationRecovery<Self::Confirmation>, StateError>;

    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        intent: &Self::Intent,
        submission: &Self::Submission,
        receipt: &Self::Receipt,
        confirmation: &Self::Confirmation,
    ) -> Result<Self::Output, StateError>;
}
```

`SubmissionEvidence<T>`, `SubmissionRecovery<T>`, `ReceiptRecovery<T>`, and
`ConfirmationRecovery<T>` are typed result wrappers whose payloads are persisted as the matching
side-effect events before the scheduler advances to the next phase. They are not store authority by
themselves; the scheduler/store still owns event envelopes, logical keys, commit idempotency, and
projection updates.

This separation forces side-effect state authors to model external mutation explicitly. It also
gives replay a durable boundary: replay verifies intent, idempotency input, idempotency key,
submission evidence, receipt, and confirmation instead of reapplying the mutation.

The associated types make the shape of a side effect compile-time visible: a side-effect state
cannot omit intent, idempotency input, submission, receipt, or confirmation without failing to
implement the trait. Runtime still owns the durable protocol: canonical intent hashing, idempotency
ledger claims, ambiguous recovery, submission recovery, receipt validation, confirmation, and replay
verification.

Replay receipt and confirmation checks are explicit no-live-IO verifier calls:

```rust
pub trait SideEffectReplayVerifier {
    type Intent: MfmValue;
    type Submission: MfmValue;
    type Receipt: MfmValue;
    type Confirmation: MfmValue;

    fn verify_submission(
        &self,
        intent: &Self::Intent,
        submission: &Self::Submission,
        recorded: &RecordedSideEffectFacts,
    ) -> Result<(), ReplayError>;

    fn verify_receipt(
        &self,
        intent: &Self::Intent,
        submission: &Self::Submission,
        receipt: &Self::Receipt,
        recorded: &RecordedSideEffectFacts,
    ) -> Result<(), ReplayError>;

    fn verify_confirmation(
        &self,
        receipt: &Self::Receipt,
        confirmation: &Self::Confirmation,
        recorded: &RecordedSideEffectFacts,
    ) -> Result<(), ReplayError>;
}
```

Verifier implementations receive only recorded facts, artifacts, and typed evidence from the run
stream. Constructing live capabilities, live transports, or live read clients during replay
verification is a certification error.

### Adapters

There must be no typed-state equivalent of:

```text
IoProvider(namespace: String, request: serde_json::Value) -> serde_json::Value
```

External systems are represented by typed capabilities. Capabilities define typed requests, typed
responses, canonical request hashing, fact keys, redaction behavior, and replay behavior.
Capabilities are the state-facing contract; adapters/connectors are versioned implementations of
those contracts.

Capability tokens must have private constructors and be produced only by certified lowering/runtime
injection. External state crates may name the capability types in their `StateSpec::Caps`, but they
must not be able to forge new capability instances or widen a read-only capability set into a
transaction-capable one.

Examples:

```rust
#[async_trait]
pub trait EvmReadCap {
    async fn call_contract(
        &self,
        request: EvmCallRequest,
    ) -> Result<RecordedFact<EvmCallResult>, AdapterError>;
}

#[async_trait]
pub trait EvmTxCap {
    async fn submit_transaction(
        &self,
        intent: EvmTransactionIntent,
        idempotency: IdempotencyKey,
    ) -> Result<EvmTransactionReceipt, AdapterError>;
}
```

Examples:

- EVM read adapter
- EVM transaction adapter
- Bitcoin read adapter
- Bitcoin transaction adapter
- HTTP adapter
- SQL database adapter
- object storage adapter
- artifact store adapter
- clock adapter

States must reuse these adapters rather than reaching into ambient IO. The platform must provide
live and replay implementations for each capability. Replay implementations must answer only from
recorded facts or receipts. They must not silently fall back to live IO.

Adapter and connector versions are part of the certified execution contract when they affect
canonical request formation, fact-key derivation, receipt interpretation, replay verification,
redaction, or externally visible behavior. Breaking changes create new adapter/connector versions;
old versions must remain available for replay/resume of specs that reference them.

This creates a stable extension surface: future developers implement new states and operations while
reusing the MFM harness for adapters, replay, facts, artifacts, idempotency, tracing, and testing.

Adapters own:

- canonical request serialization
- request and response schema ids
- fact key derivation
- idempotency ledger interaction
- secret redaction
- no-secret persisted payload validation
- live versus replay behavior
- adapter-specific trace records

States own domain transformation logic. They must not implement bespoke fact lookup, replay, or
idempotency plumbing.

### Idempotency And Side-Effect Replay

Side effects require a durable idempotency model. Runtime metadata is insufficient.

For every side-effect node, the certified spec and event stream must record:

- state kind and version
- scope id
- input cell ids
- input semantic type ids and schema ids
- intent semantic type id and schema id
- canonical intent hash
- idempotency input semantic type id and schema id
- idempotency key
- adapter capability
- submission semantic type id and schema id
- receipt semantic type id and schema id
- confirmation semantic type id and schema id
- side-effect attempt record id

The idempotency ledger must be keyed by certified state identity plus canonical intent hash and
idempotency key. Re-running the same certified side-effect state must either:

- observe that the effect already succeeded and return the recorded receipt
- observe that the effect is pending and follow the adapter's confirmation protocol
- fail without reapplying if the previous state is ambiguous

Replay must verify the recorded receipt and confirmation against the certified spec. It must not
apply the side effect.

The framework-derived idempotency key is:

```rust
pub struct IdempotencyKey<I: MfmValue> {
    pub digest: ContentDigest,
    _input: PhantomData<fn(I) -> I>,
}
```

```text
idem:v1:digest(
  spec_hash,
  node_id,
  scope_id,
  state_kind,
  state_version,
  adapter_binding,
  idempotency_input_schema_id,
  canonical(idempotency_input)
)
```

The durable ledger key is:

```text
sidefx:v1:digest(
  run_id,
  spec_hash,
  node_id,
  scope_id,
  state_kind/version,
  capability_kind/version,
  adapter_kind/version,
  intent_schema_id,
  intent_hash,
  idempotency_input_hash,
  idempotency_key
)
```

The `v1` module defaults to run-scoped dedupe by including `run_id` in the ledger key. Cross-run dedupe changes
external mutation semantics and must be introduced later as an explicit `IdempotencyScope::CrossRun`
contract, not as an implicit adapter convention.

The scheduler commits side-effect execution across multiple durable commits. The uncertainty
boundary must be durable before the external submit call:

| Durable commit | Events in the commit | External action after commit | Resume if crash occurs after commit |
|---|---|---|---|
| attempt | `v1::events::StateAttemptStarted` | none | restart attempt or continue to intent |
| prepare | `v1::events::side_effect::IntentPersisted`, `Claimed`, `InvocationPrepared` | none | continue to invocation-started |
| invocation-started | `v1::events::side_effect::InvocationStarted` | call `submit` exactly after this commit succeeds | enter `recover_submission`; never blindly resubmit |
| submission-result | exactly one of `SubmissionObserved`, `NotSubmittedProven`, `SubmissionUnknown`, or `Ambiguous` | none | follow the recorded state |
| receipt | `v1::events::side_effect::ReceiptObserved` | none | recover confirmation |
| confirmation | `v1::events::side_effect::ConfirmationObserved` | none | derive output |
| output | `v1::events::CellProduced`, `v1::events::StateAttemptCompleted` | none | node is terminal |
| failed-attempt | `v1::events::side_effect::Failed`, `v1::events::StateAttemptFailed` | none | retry only if failure phase and retry policy permit |

Artifacts for intent, receipt, confirmation, and output are written before the commit that
references them. A crash after `v1::events::side_effect::InvocationStarted` means resume must enter recovery; it
must not blindly submit again.

Per ledger key, v1 uses this normative transition table:

| From state | Allowed next event/state | Resume behavior |
|---|---|---|
| `absent` | `StateAttemptStarted` -> `attempt_started` | start attempt |
| `attempt_started` | `side_effect::IntentPersisted` -> `intent_persisted` | recompute/persist intent if missing |
| `attempt_started` | `side_effect::Failed` -> `failed` | fail before any external uncertainty |
| `intent_persisted` | `side_effect::Claimed` -> `claimed` | claim or detect conflicting claim |
| `intent_persisted` | `side_effect::Failed` -> `failed` | fail before any external uncertainty |
| `claimed` | `side_effect::ClaimTakenOver` with higher `claim_generation` -> `claimed` | replace stale owner before invocation uncertainty |
| `claimed` | `side_effect::InvocationPrepared` -> `invocation_prepared` | prepare invocation |
| `claimed` | `side_effect::Failed` -> `failed` | fail before any external uncertainty |
| `invocation_prepared` | `side_effect::ClaimTakenOver` with higher `claim_generation` -> `claimed` | replace stale owner only if no `InvocationStarted` exists |
| `invocation_prepared` | `side_effect::InvocationStarted` -> `invocation_started_unknown` | commit uncertainty before submit |
| `invocation_prepared` | `side_effect::Failed` -> `failed` | fail before any external uncertainty |
| `invocation_started_unknown` | `side_effect::NotSubmittedProven` -> `not_submitted_proven` | may retry with next `invocation_epoch` |
| `invocation_started_unknown` | `side_effect::SubmissionObserved` -> `submission_observed` | recover receipt |
| `invocation_started_unknown` | `side_effect::SubmissionUnknown` -> `submission_unknown` | recover submission |
| `invocation_started_unknown` | `side_effect::Ambiguous` -> `ambiguous` | block for manual resolution |
| `not_submitted_proven` | `side_effect::Claimed` with next epoch -> `claimed` | retry safely through the normal claim/prepare path |
| `not_submitted_proven` | `side_effect::Failed` -> `failed` | fail after proof that no external mutation happened |
| `submission_unknown` | `side_effect::NotSubmittedProven` -> `not_submitted_proven` | retry safely |
| `submission_unknown` | `side_effect::SubmissionObserved` -> `submission_observed` | recover receipt |
| `submission_unknown` | `side_effect::Ambiguous` -> `ambiguous` | block for manual resolution |
| `submission_observed` | `side_effect::ReceiptObserved` -> `receipt_observed` | recover receipt |
| `submission_observed` | `side_effect::Ambiguous` -> `ambiguous` | block for manual resolution |
| `receipt_observed` | `side_effect::ConfirmationObserved` -> `confirmation_observed` | recover confirmation |
| `receipt_observed` | `side_effect::Ambiguous` -> `ambiguous` | block for manual resolution |
| `confirmation_observed` | `CellProduced` + `StateAttemptCompleted` -> `completed` | derive output without external mutation |
| `failed` | new attempt if retryable | retry starts with a new `attempt_id` |
| `ambiguous` | none in v1 | manual resolution required |
| `completed` | none | terminal |

`v1::events::side_effect::InvocationStarted` is the uncertainty boundary. After it is durable, resume must
assume the mutation may have happened. Retry is legal only after adapter-proven `not_submitted`
evidence and must use the next `invocation_epoch`. Permanent ambiguity blocks and surfaces for
manual resolution in v1.

Claim takeover is legal only before `InvocationStarted` exists for the ledger key and invocation
epoch. It must advance a store-fenced `claim_generation` and produce a new `claim_fencing_token`.
Commits from stale claim owners or stale fencing tokens are rejected by store preconditions. Once
`InvocationStarted` is durable, takeover for that invocation epoch is forbidden; resume must enter
submission recovery instead.

`v1::events::side_effect::Failed` records attempt failure, not proof that an external mutation did
or did not happen. It is legal before `v1::events::side_effect::InvocationStarted`, or after
`v1::events::side_effect::NotSubmittedProven`. After `InvocationStarted`, failures without
not-submitted proof must become `side_effect::SubmissionUnknown` or `side_effect::Ambiguous`.

### Operations

Operations are typed expansion recipes over states and other operations. They never execute at
runtime.

```rust
pub trait Operation {
    type Config: MfmConfig;
    type Input<'p, 's>: OperationInput<'p, 's>;
    type Output<'p, 's>: OperationOutput<'p, 's>;

    const KIND: OperationKind;
    const VERSION: OperationVersion;

    fn expand<'p, 's>(
        &self,
        config: Self::Config,
        input: Self::Input<'p, 's>,
        builder: &mut ScopeBuilder<'p, 's>,
    ) -> Result<Self::Output<'p, 's>, PlanError>;
}

pub trait IntoOperationInput<'p, 's, I: OperationInput<'p, 's>> {
    fn into_operation_input(self) -> Result<I, PlanError>;
}

pub trait OperationInput<'p, 's> {
    fn input_binding(&self) -> OperationInputBindingSpec;
}

pub trait OperationOutput<'p, 's> {
    fn output_handles(&self) -> Vec<TypedHandleRef>;
}
```

As with states, `Operation` is descriptive and callable only through framework registration
authority:

```rust
pub struct RegisteredOperation<O: Operation> {
    descriptor: OperationDescriptorIdentity,
    evidence: OperationRegistrationEvidence<O>,
    _operation: PhantomData<fn(O) -> O>,
}

pub struct OperationRegistrationEvidence<O: Operation> {
    // private fields; constructed only by framework registration code
    _operation: PhantomData<fn(O) -> O>,
}

pub trait OperationRegistry {
    fn registered_operation<O>(&self) -> Result<RegisteredOperation<O>, RegistryError>
    where
        O: Operation;
}
```

`Operation::expand` is the deterministic framework hook used after operation registration has
resolved descriptor evidence. User-facing certified planning must enter through `ScopeBuilder::call`
or an equivalent builder API that resolves operation registration and records operation lineage.
Direct calls to `Operation::expand` may be used only inside framework registration, tests, or
certification fixtures; they are not a certifiable launch path by themselves.

An operation's output is usually a struct of typed handles:

```rust
pub struct PortfolioOutputs<'p, 's> {
    pub snapshot: Handle<'p, 's, PortfolioSnapshot>,
    pub snapshot_artifact: Handle<'p, 's, ArtifactRef<PortfolioSnapshot>>,
    pub report: Handle<'p, 's, PortfolioReport>,
}
```

An operation cannot claim to export a report unless its expansion returns a
`Handle<PortfolioReport>`.

Not every operation output must be a public API output. Internal operations may return typed helper
structs that do not implement `PublicOutputs`. Public launch/render surfaces, however, must require
the root output type to implement `PublicOutputs<'p, 's>` so user-facing terminal shape is checked
against actual typed output cells.

`OperationOutput` and `PublicOutputs` are derive-backed by default for domain crates. Manual
implementations are framework-owned exceptions for closed wrapper types only. The builder validates
that every public cell reference came from a private handle minted inside the current typed program.
Public output binding must occur through `RootBuilder::bind_public_outputs` inside the branded build
closure.

Operation inputs and outputs are not a second unchecked export surface. Their derive
implementations may only traverse branded handles, tuples, derive-backed structs, and closed
framework wrappers. Domain crates must not hand-write `OperationOutput` or `PublicOutputs`
implementations in v1.

States and operations must share a common expansion interface:

```rust
pub trait Expandable {
    type Config: MfmConfig;
    type Input<'p, 's>;
    type Output<'p, 's>;

    fn expand<'p, 's>(
        self,
        config: Self::Config,
        input: Self::Input<'p, 's>,
        builder: &mut ScopeBuilder<'p, 's>,
    ) -> Result<Self::Output<'p, 's>, PlanError>;
}
```

A single state is the smallest expandable unit. An operation is a larger expandable unit. This lets
users compose workflows by stacking operations, selecting individual states, or mixing both with
typed configuration injection.

Recursive operation expansion is valid only if every child operation reduces to the same typed
program IR and certified spec. No child operation may smuggle a dynamic graph around the typed
builder.

### Typed Program Builder

The typed builder is the only way to create executable program structure.

```rust
pub struct ScopeBuilder<'program, 'scope> {
    // internal typed program representation
}

impl<'p, 's> ScopeBuilder<'p, 's> {
    pub fn state<S, I>(
        &mut self,
        key: StableNodeKey,
        config: S::Config,
        input: I,
    ) -> Result<Handle<'p, 's, S::Output>, PlanError>
    where
        S: StateSpec,
        I: IntoStateInput<'p, 's, S::Input>;

    pub fn call<O, I>(
        &mut self,
        key: StableOperationKey,
        operation: O,
        config: O::Config,
        input: I,
    ) -> Result<O::Output<'p, 's>, PlanError>
    where
        O: Operation,
        I: IntoOperationInput<'p, 's, O::Input<'p, 's>>,
        O::Output<'p, 's>: OperationOutput<'p, 's>;
}
```

These signatures are shorthand for registry-mediated planning. Before minting a handle or operation
output, the builder must resolve `S` through its `StateRegistry` and `O` through its
`OperationRegistry` or receive an equivalent `RegisteredState<S>` / `RegisteredOperation<O>` token.
Unregistered specs are planning errors and cannot lower to erased runners.

The builder derives dependencies from handles. Workflow authors do not manually write
`DependencyEdge` values. The builder records:

- which typed input cells each state consumes
- which typed output cell each state produces
- state kind and version
- operation kind and version
- canonical state config hash and config artifact/reference
- planning lineage
- effect and capability declarations
- idempotency policy
- scope identity
- stable node key
- deterministic ordering metadata

The builder must reject:

- duplicate stable node keys in the same scope
- cross-program handles
- cross-scope handles without an explicit bridge
- undeclared capability use
- missing terminal output handles
- dynamic graph fragments that bypass typed handles

`IntoStateInput<S::Input, 'p, 's>` is a correctness-critical API, not an ergonomic afterthought. It
must define the allowed conversions from handles into state inputs for:

- single handles
- tuples of handles
- domain-specific input structs
- typed vectors and canonically ordered collections
- `NonEmptyHandles<T>` or equivalent wrappers
- `MaybeValue<T>` skip-aware inputs
- typed artifact references

If an input conversion is not represented by this trait family, workflow authors should not be able
to smuggle it through JSON, context keys, or manually written graph edges.

### Dynamic Deterministic Expansion

Dynamic fanout is still supported, but only when expansion remains deterministic. The number of
generated states may depend on typed planning config, as long as expansion is deterministic and
produces typed handles.

Required:

- stable domain keys for generated children
- canonical sorting before emission
- duplicate-key rejection
- deterministic node and cell id derivation
- explicit `NonEmptyHandles<T>` or equivalent when a domain rule requires at least one child
- no dependence on wall-clock time, random values, live IO, database reads, or same-run state output
  during expansion

This is enforced with two layers. Static source checks deny `std::fs`, `std::env`, `std::process`,
`tokio::process`, network clients, randomness, clocks, and database clients in `ops/*`,
`mfm-program`, and typed expansion modules. These APIs are allowed only in `transports/*`,
`crates/app`, binaries, and explicit test modules. Certification then rejects duplicate stable keys
and missing canonical ordering evidence. The test suite must expand the same config repeatedly in
one process and across separate processes and compare the certified spec hash.

The deterministic expansion lint should be implemented as a source-boundary check over crate/module
paths and denied symbol families, then backed by compile-fail fixtures. The `v1` denied families are:

```text
std::fs
std::env
std::process
tokio::process
std::time::SystemTime / chrono::Utc::now / time::OffsetDateTime::now_*
rand::* and getrandom::*
network clients and sockets
database clients
unscoped process-local counters
```

Allowed exception zones are `transports/*`, `collectors/*` that are transport/client
implementations, `crates/app`, binaries, explicit `#[cfg(test)]` modules, and kernel runtime/store
code that is not used for expansion. Any new exception must name the crate path, denied symbol
family, reason, and test that proves the symbol cannot influence certified spec construction.

Config-derived fanout cardinality and duplicate generated keys are planning/certification
concerns, not runtime concerns. If expansion must depend on observed runtime data, that is not
same-run expansion. It is a new planning boundary and must produce a new certified spec.

### Stable Node And Cell Identity

Stable identities are part of reproducibility. Author-supplied stable keys are local labels; they
are not the certified identities themselves.

Author key grammar:

```text
key        = segment *("/" segment)
segment    = [a-z0-9] *([a-z0-9] | "-" | "_" | ".")
segment len: 1..64
full len:   1..256
reserved prefixes: "mfm.", "sys.", "_"
```

No Unicode and no escaping are allowed in author keys. Dynamic keys use `StableDomainKey`, not
string concatenation:

```rust
pub trait StableDomainKey: MfmValue {
    fn domain_key_descriptor() -> SchemaDescriptor;
    fn canonical_domain_bytes(&self) -> CanonicalBytes;
}
```

Same-type values inside one scope may still belong to different domain instances. MFM therefore
persists value lineage separately from semantic type and schema identity:

```rust
pub struct ValueLineageRef {
    pub lineage_digest: ContentDigest,
}

pub struct ValueLineage {
    pub scope_id: ScopeId,
    pub producer: v1::CellProducer,
    pub input_cells: Vec<CellId>,
    pub config_ref_digest: Option<ContentDigest>,
    pub operation_lineage: OperationLineage,
    pub domain_keys: Vec<StableDomainKeyRef>,
    pub transform_policy: LineageTransformPolicy,
}
```

Lineage is hash-defining evidence for typed cells, input bindings, and public outputs. Projection,
bridge, and transform states may produce new lineage, but consuming a same-type value whose lineage
does not satisfy the consumer's descriptor is a certification error.

Certified ids are derived from canonical evidence:

```text
ScopeId = scope:sha256-jcs-v1({
  alg, parent_scope_id, local_scope_key, operation_lineage
})

OperationInstanceId = op:sha256-jcs-v1({
  alg, parent_scope_id, operation_key, operation_kind, operation_version,
  operation_descriptor_id, parent_operation_lineage,
  config_digest, input_binding_digest
})

NodeId = node:sha256-jcs-v1({
  alg, lowering_version, scope_id, local_node_key,
  state_kind, state_version, config_digest, input_binding_digest
})

CellId = cell:sha256-jcs-v1({
  alg,
  lowering_version,
  scope_id,
  producer: { node_id } | { seed_id },
  output_index: 0,
  semantic_type_id,
  schema_id
})
```

Duplicate author keys in the same parent namespace are planning errors. Digest collision between
different identity payloads is fatal certification corruption. `SeedId` is derived from `{ alg,
lowering_version, scope_id, seed_key, semantic_type_id, schema_id }`. Seed value digests are not
part of `SeedId`; `v1::events::RunStarted` is the runtime value-digest authority. `v1::SeedSpec`
declares the planned seed cells in the certified spec, and `RunStarted` must bind each seed cell to
the actual digest and artifact evidence supplied at launch. If `v1::SeedSpec::required_digest` is
present, the run-start digest must match it exactly. Lowering version is included in derived ids and
in the certified spec hash.

Derived ids must not depend on:

- `HashMap` iteration order
- allocation order
- pointer identity
- thread scheduling
- non-canonical JSON
- runtime wall-clock time
- process-local counters unless the counter input is itself derived from canonical ordering

Changing the lowering algorithm version must change the certified spec hash.

The implementation must publish golden test vectors for each identity payload: `ScopeId`,
`OperationInstanceId`, `NodeId`, `CellId`, `input_binding_digest`, `operation_lineage`,
`config_ref_digest`, and dynamic collection ordering. Reordered maps must produce identical ids,
duplicate domain keys must reject planning, and changing the lowering version must change node ids
and the final spec hash.

### v1 Module Invariants

The `v1` Rust modules should keep identity and digest names narrow:

```text
ContentDigest  generic digest of canonical bytes or artifact bytes
SpecHash       digest of canonical v1::TypedExecutionSpec bytes
CellId         planned/certified cell id in the typed spec
ArtifactId     storage object identity
EventId        store-owned event identity
```

`OutputCellId` must not be a public v1 API type. Semantically, v1 has one kind of typed cell id:
`CellId`. A private transitional alias is allowed only inside migration modules. A cell is an output
of exactly one producer node or a root seed, and v1 states produce exactly one output cell. Multiple
domain values must be represented as one output struct or explicit projection states. Projection
states are ordinary states with their own node id, input binding, output cell, provenance, and
terminal events.

`SpecHash` must not be used for arbitrary content digests. `ContentDigest` must not imply the bytes
are a certified spec. Public APIs, event payloads, and storage projections should preserve this
distinction so replay and certification errors can name the violated invariant precisely.

### Certified Typed Execution Spec

After typed expansion, MFM produces a certified typed execution spec. The persisted v1 shape is the
`v1::TypedExecutionSpec` below; older sketches with a separate `state_program` and `outputs` field
are not part of the v1 contract.

The persisted v1 spec is canonical JSON:

```text
spec:sha256-jcs-v1:<hex64>
```

```rust
pub mod v1 {
    pub struct CertifiedSpecEnvelope {
        pub spec_hash: SpecHash,
        pub spec: TypedExecutionSpec,
        pub audit: TypedExecutionSpecAudit,
    }

    pub struct TypedExecutionSpec {
        pub spec_version: SpecVersion, // "mfm.typed.execution_spec.v1"
        pub media_type: MediaType,     // application/vnd.mfm.typed-execution-spec+json;version=1
        pub canonicalization: CanonicalizationId, // sha256-jcs-v1
        pub lowering_version: LoweringVersion,
        pub authoring: AuthoringProvenance,
        pub scopes: Vec<ScopeSpec>,
        pub seeds: Vec<SeedSpec>,
        pub descriptor_identities: Vec<DescriptorIdentity>,
        pub config_refs: Vec<ConfigRef>,
        pub nodes: Vec<NodeSpec>,
        pub cells: Vec<CellSpec>,
        pub public_outputs: PublicOutputSpec,
    }

    pub struct TypedExecutionSpecAudit {
        pub descriptor_audit_refs: Vec<DescriptorAuditRef>,
        pub source_package_refs: Vec<SourcePackageRef>,
        pub non_semantic_provenance: Vec<AuditProvenanceRef>,
    }

    pub struct ConfigRef {
        pub schema_id: SchemaId,
        pub artifact_id: ArtifactId,
        pub digest: ContentDigest,
        pub byte_len: u64,
        pub media_type: MediaType,
    }

    pub struct ScopeSpec {
        pub scope_id: ScopeId,
        pub parent_scope_id: Option<ScopeId>,
        pub stable_key: ScopeKey,
    }

    pub struct SeedSpec {
        pub seed_id: SeedId,
        pub seed_key: StableSeedKey,
        pub cell_id: CellId,
        pub scope_id: ScopeId,
        pub semantic_type_id: SemanticTypeId,
        pub schema_id: SchemaId,
        pub required_digest: Option<ContentDigest>,
    }

    pub struct NodeSpec {
        pub node_id: NodeId,
        pub stable_key: StableNodeKey,
        pub scope_id: ScopeId,
        pub state_kind: StateKind,
        pub state_version: StateVersion,
        pub descriptor_id: DescriptorId,
        pub config_ref: ConfigRef,
        pub input_bindings: InputBindingSpec,
        pub output_cell: CellId,
        pub effect_kind: EffectKind,
        pub capability_bindings: CapabilitySetDescriptor,
        pub adapter_bindings: Vec<AdapterBinding>,
        pub side_effect: Option<SideEffectContractSpec>,
        pub framework: Option<FrameworkNodeSpec>,
        pub planning_lineage: PlanningLineage,
        pub deterministic_predecessors: Vec<NodeId>,
    }

    pub enum FrameworkNodeSpec {
        Bridge(BridgeNodeSpec),
        PublicOutputRender(PublicOutputRenderNodeSpec),
    }

    pub struct PublicOutputRenderNodeSpec {
        pub public_schema_id: SchemaId,
        pub output_spec_digest: ContentDigest,
        pub renderer_descriptor: RendererDescriptorIdentity,
        pub required_cells: Vec<PublicOutputCell>,
    }

    pub enum CellProducer {
        Node(NodeId),
        Seed(SeedId),
    }

    pub struct CellSpec {
        pub cell_id: CellId,
        pub producer: CellProducer,
        pub scope_id: ScopeId,
        pub semantic_type_id: SemanticTypeId,
        pub schema_id: SchemaId,
        pub value_lineage: ValueLineageRef,
        pub terminal_policy: CellTerminalPolicy,
        pub storage_policy: StoragePolicy,
        pub redaction_policy: RedactionPolicy,
    }
}
```

Only `v1::TypedExecutionSpec` bytes are hashed into `SpecHash`; `v1::TypedExecutionSpecAudit` is
carried beside the spec in `v1::CertifiedSpecEnvelope` and is not semantic authority. Descriptor
identities required for certification are embedded in the spec and hash-defining. Larger
audit/provenance material lives in the envelope audit section. The `v1` module prefers config
artifacts over inline config bytes. Config refs must be resolvable at run start, and the run cannot
commit `v1::events::RunStarted` until every referenced config artifact exists with the declared
schema id, digest, byte length, and media type.

All semantic framework nodes are also pre-hash spec nodes. Same-value bridges and public-output
rendering are represented in `v1::NodeSpec::framework` before certification. Certified lowering may
bind those persisted nodes to framework runners, but it must not invent bridge edges, render nodes,
public-output declarations, dependencies, capability grants, or input bindings after the spec hash
is fixed.

Authoring provenance names what authored the state program. It is non-executable audit metadata:
the state machine executes the `StateProgramSpec`, not the operations or composition helpers that
authored it. MFM supports three authoring modes: operation expansion, direct state composition, and
a mix of operations and directly declared states:

```rust
pub enum AuthoringProvenance {
    OperationExpansion {
        operation: OperationDescriptor,
        config_hash: ContentDigest,
    },
    StateComposition {
        descriptor: StateCompositionDescriptor,
        config_hash: ContentDigest,
    },
    MixedComposition {
        descriptor: MixedCompositionDescriptor,
        config_hash: ContentDigest,
    },
}

pub struct StateProgramSpec {
    pub nodes: Vec<v1::NodeSpec>,
    pub cells: Vec<v1::CellSpec>,
}
```

This keeps the state machine model explicit. A workflow built from a single state, a sequence or DAG
of states, an operation, or a mix of states and operations all reduce to a certified
`StateProgramSpec`. The runtime executes only the states in that spec.

Each node spec must include:

- node id
- state kind and version
- state descriptor id
- state config hash
- state config artifact/reference
- planning lineage
- scope id
- input cells
- input semantic type ids
- input schema ids
- input value lineage refs
- output cell
- output semantic type id
- output schema id
- output value lineage ref
- effect kind
- capability set
- adapter/connector version references where behaviorally relevant
- idempotency policy
- stable node key
- deterministic predecessor ids

Each cell spec must include:

- cell id
- semantic type id
- schema id
- value lineage ref
- producer, either `v1::CellProducer::Node` or `v1::CellProducer::Seed`
- scope id
- content-addressing policy
- redaction policy
- artifact/fact policy where applicable

The spec is certified only after a lowerer verifies that:

- every consumed cell has exactly one producer, either a node or a seed
- every handle type matches the consumer input type
- every node's state descriptor id, state kind, and state version resolve to a registered immutable
  state descriptor and runner factory
- every state descriptor matches the associated types and effect/capability declarations generated
  or registered by the compiled state implementation
- every operation lineage frame resolves to a registered immutable operation descriptor whose
  kind/version, config/input/output descriptors, and deterministic expansion ABI match the spec
- every state config reference resolves to canonical bytes with the expected schema id and hash
- every state has a deterministic id
- every dependency is derived from typed handles
- every effect matches the state's execution trait
- every capability is allowed by the state effect
- every side-effect state has typed intent, idempotency, submission, receipt, and confirmation
- every public output is reachable from produced typed cells
- every dynamic collection is canonically ordered
- every persisted value has semantic and schema ids
- every same-type same-scope lineage requirement matches, or an explicit transform, bridge, or
  projection state produces new lineage

Certification is also where compile-time evidence becomes a persisted contract. Rust lifetimes,
associated types, and private constructors disappear after lowering, so the certified spec must
persist their effects as explicit node, cell, scope, schema, effect, capability, and public-output
metadata.

### Certified Lowering

Certified lowering turns the typed execution spec into an erased runner plan:

```rust
pub struct CertifiedPlan<O> {
    pub spec: TypedExecutionSpec,
    pub spec_hash: SpecHash,
    erased_plan: ErasedExecutionPlan,
    _output: PhantomData<O>,
}

pub struct ErasedExecutionPlan {
    pub spec_hash: SpecHash,
    pub runner_bindings: BTreeMap<NodeId, RegisteredRunnerBinding>,
}
```

The erased execution plan is an implementation binding only. Scheduler runnable-node computation,
dependencies, cell producers, bridge behavior, public-output rendering, capability grants, adapter
choices, and input bindings must all come from `v1::TypedExecutionSpec`. The erased plan may not carry
additional semantic edges or hidden framework nodes.

The lowered plan may use erased runners internally:

```rust
pub trait ErasedNodeRunner: Send + Sync {
    fn run_erased<'a>(
        &'a self,
        ctx: ErasedRunCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<(), StateError>> + Send + 'a>>;
}

pub struct ErasedRunCtx<'a> {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub store: &'a mut dyn TypedValueStore,
    pub caps: &'a mut dyn CertifiedRuntimeCaps,
    pub commits: &'a mut dyn TypedRunCommitSink,
}
```

Trait object erasure is acceptable only after certification. Before certification, the authoring API
must remain typed and generic. After certification, object safety and scheduler practicality become
implementation concerns, not semantic compromises.

The boxed-future ABI is the stable erased boundary. Implementations may use `async_trait` as local
ergonomic sugar, but the public `dyn ErasedNodeRunner` contract must remain object-safe without
assuming language-level async trait object support.

The erased plan must not be persisted as the authoritative contract. The persisted contract is the
certified typed execution spec plus its canonical hash.

Lowering must use registered state descriptors and runner factories. It must not deserialize a
spec into arbitrary executable code, and it must not allow unregistered state kinds to run. The
erased runner is an implementation detail derived from a certified, versioned spec.

### Typed Scheduler Contract

The first certified runtime slice should introduce a new typed scheduler/executor. The current
scheduler is not the semantic authority for typed runs and should not be wrapped as the certified
runtime execution plan. The v1 typed scheduler should be deliberately serial and small:

1. load or receive a certified `v1::TypedExecutionSpec`
2. verify the spec hash, descriptor identities, config refs, executable identities, adapter
   identities, and canonicalizer identity
3. write the spec and required config artifacts before run start
4. append `v1::events::RunStarted` at `run:{run_id}` sequence `1`
5. rebuild projections from the authoritative run stream
6. compute the next runnable node in deterministic topological order
7. materialize inputs only from certified typed cells
8. resolve the registered runner factory and certified config
9. mint only the capabilities allowed by the node's certified capability set
10. execute through the node's effect-specific runner
11. write required artifacts first
12. atomically append typed commit payloads and update derived projections through the store
13. repeat until the pre-hash framework public-output render node emits `v1::events::PublicOutputProduced`
14. append `v1::events::RunCompleted(Completed)` only after public-output evidence exists

Parallel execution is out of scope for v1. Parallelism may be added later only after the typed
commit protocol, idempotency rules, and projection rebuild behavior are proven with serial
execution.

The typed scheduler must not accept `PlannedOp`, `PortKey`, public `StateGraph`,
`DependencyEdge`, semantic `DynContext` dataflow, string export keys, or generic `IoProvider` as
typed-run semantics. These APIs are deleted or isolated as the typed kernel lands; they are not
compatibility surfaces for certified typed execution.

The first certified runtime slice must execute on this typed scheduler. The old scheduler may remain in the
repository for unmigrated workflows, but no old scheduler component may provide semantic authority,
input materialization, output completion, replay, or public-output rendering for a certified typed
run.

### Runtime Execution

The runtime executes states only.

For each node, runtime must:

1. verify that the run uses the expected certified spec hash
2. check that predecessor cells are complete
3. load typed input cells by cell id
4. verify stored semantic type ids and schema ids against the certified spec
5. resolve the node's registered state descriptor and canonical config reference
6. deserialize input cells into the state input type
7. provide only the capabilities allowed by the certified spec
8. execute the state through the effect-specific runner
9. serialize and persist the typed output cell
10. content-address the output
11. append provenance-bearing events

Cell completion must be evidence-defined. Writing a cell artifact is not enough to make the cell
complete after a crash. A state-produced cell is complete only when the run stream contains the
provenance-bearing event that binds the cell id, semantic type id, schema id, content digest,
producer node id, scope id, attempt id, and certified spec hash. A seed cell is complete only when
`v1::events::RunStarted` binds the certified spec, seed id, cell id, semantic type id, schema id,
scope id, value lineage, digest, and mandatory launch artifact evidence. Orphaned artifacts without
such evidence may be garbage collected or ignored, but they must not advance resume.

Cell terminal states in v1 are `Pending`, `Produced`, and `Skipped`. There is no terminal
cell-failure event; failures belong to state attempts. Pending cells remain unavailable if the run
fails.

```rust
pub struct CellProduced {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub cell_id: CellId,
    pub scope_id: ScopeId,
    pub attempt_id: AttemptId,
    pub semantic_type_id: SemanticTypeId,
    pub schema_id: SchemaId,
    pub value_lineage: ValueLineageRef,
    pub artifact_id: ArtifactId,
    pub content_digest: ContentDigest,
    pub producer_state_kind: Option<StateKind>,
    pub producer_state_version: Option<StateVersion>,
}

pub struct CellSkipped {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub cell_id: CellId,
    pub scope_id: ScopeId,
    pub attempt_id: AttemptId,
    pub semantic_type_id: SemanticTypeId,
    pub schema_id: SchemaId,
    pub value_lineage: ValueLineageRef,
    pub skip_reason: SkipReason,
}
```

Produced after skipped, skipped after produced, the same cell with a different digest, a second
attempt terminally completing an existing cell, missing artifacts, wrong artifact digest/schema/
semantic id/producer, or `v1::events::CellSkipped` for a non-`MaybeValue` cell are corruption or certified
history errors. Exact duplicate recovery is idempotent only through the same typed commit key.

Deserialization may still fail if persisted data is corrupt or unavailable. It must not fail because
the planner wired the wrong producer to the wrong consumer.

The runtime may maintain context snapshots for observability, but states must not read their
semantic inputs from those snapshots.

#### Non-Side-Effect Attempt Recovery

Pure, read, and managed-platform-write states use the same attempt envelope, but they do not use the
side-effect ledger. For these states, `StateAttemptStarted` without a terminal commit means resume
continues the same attempt from certified evidence. A new attempt may start only after
`StateAttemptFailed { retryable: true, ... }` has committed for the prior attempt.

For all non-side-effect states, `CellProduced` or `CellSkipped` and `StateAttemptCompleted` must be
committed in the same atomic terminal commit. A terminal cell without the matching attempt
completion, or attempt completion without the terminal cell, is corrupt certified history. If a read
attempt has already committed `FactRecorded` events, resume must reuse those facts for that attempt;
retry may record additional facts only under a new attempt id. Managed-write states may re-stage
content-addressed artifacts after a crash, but staged artifacts do not become replay, resume,
retention, or public-output authority until the scheduler/store binds them in a typed commit.

### Typed Kernel Event Schema And Commit Authority

Typed kernel events are mandatory. Runtime-critical facts, artifact references, cell completion,
side-effect ledger state, and public-output evidence must be typed runtime events or commit payloads
and cannot be suppressed by event profile settings.

Every committed event is wrapped by the store:

```rust
pub struct KernelEventEnvelope {
    pub event_id: EventId,
    pub event_schema_id: SchemaId,
    pub run_id: RunId,
    pub seq: StreamSeq,
    pub ordinal: CommitOrdinal,
    pub spec_hash: SpecHash,
    pub commit_key: CommitKey,
    pub logical_key: LogicalEventKey,
    pub payload_hash: ContentDigest,
    pub payload: KernelEventPayload,
    pub audit: KernelEventAudit,
}
```

Payloads are MFM values and canonical-json-hashable. `seq` is contiguous per run stream. `ordinal`
orders events inside one atomic commit. `event_id` is derived from run id, sequence, ordinal, event
schema id, and payload hash. `payload_hash` covers canonical payload bytes only, not audit fields.
`logical_key` drives duplicate/conflict checks, such as `cell:{cell_id}:terminal` or
`sidefx:{ledger_key}:claim`.

The `v1` logical keys are derived by the store from payload fields:

```text
run:start                         v1::events::RunStarted
run:complete                      v1::events::RunCompleted
attempt:{node_id}:{attempt_id}    state attempt lifecycle events
cell:{cell_id}:terminal           v1::events::CellProduced / v1::events::CellSkipped
fact:{node_id}:{attempt_id}:{fact_key}
artifact:{artifact_id}:ref
sidefx:{ledger_key}:intent
sidefx:{ledger_key}:claim:{invocation_epoch}:{claim_generation}
sidefx:{ledger_key}:claim:{invocation_epoch}:{claim_generation}:taken_over
sidefx:{ledger_key}:invocation:{invocation_epoch}:prepared
sidefx:{ledger_key}:invocation:{invocation_epoch}:started
sidefx:{ledger_key}:invocation:{invocation_epoch}:submission_result
sidefx:{ledger_key}:invocation:{invocation_epoch}:receipt
sidefx:{ledger_key}:invocation:{invocation_epoch}:confirmation
sidefx:{ledger_key}:invocation:{invocation_epoch}:failure
sidefx:{ledger_key}:ambiguous
public_output:{public_schema_id}
retention:{run_id}:refs:{payload_hash}
retention:{run_id}:manifest:{manifest_seq}
```

`submission_result` is mutually exclusive for observed, not-submitted, unknown, or ambiguous
evidence for one invocation epoch. The store must reject duplicate or conflicting phase-specific
keys for the same ledger key.

The `v1` event payload variants include:

```text
RunStarted
StateAttemptStarted
FactRecorded
ArtifactReferenced
CellProduced
CellSkipped
side_effect::IntentPersisted
side_effect::Claimed
side_effect::ClaimTakenOver
side_effect::InvocationPrepared
side_effect::InvocationStarted
side_effect::NotSubmittedProven
side_effect::SubmissionObserved
side_effect::SubmissionUnknown
side_effect::ReceiptObserved
side_effect::ConfirmationObserved
side_effect::Ambiguous
side_effect::Failed
PublicOutputProduced
PublicOutputRenderFailed  // audit detail only; not an attempt-terminal authority
StateAttemptCompleted
StateAttemptFailed
RunCompleted
RetentionRefsAppended
RetentionManifestProjected
```

The event enum is closed in v1:

```rust
pub enum KernelEventPayload {
    RunStarted(RunStarted),
    StateAttemptStarted(StateAttemptStarted),
    FactRecorded(FactRecorded),
    ArtifactReferenced(ArtifactReferenced),
    CellProduced(CellProduced),
    CellSkipped(CellSkipped),
    SideEffectIntentPersisted(side_effect::IntentPersisted),
    SideEffectClaimed(side_effect::Claimed),
    SideEffectClaimTakenOver(side_effect::ClaimTakenOver),
    SideEffectInvocationPrepared(side_effect::InvocationPrepared),
    SideEffectInvocationStarted(side_effect::InvocationStarted),
    SideEffectNotSubmittedProven(side_effect::NotSubmittedProven),
    SideEffectSubmissionObserved(side_effect::SubmissionObserved),
    SideEffectSubmissionUnknown(side_effect::SubmissionUnknown),
    SideEffectReceiptObserved(side_effect::ReceiptObserved),
    SideEffectConfirmationObserved(side_effect::ConfirmationObserved),
    SideEffectAmbiguous(side_effect::Ambiguous),
    SideEffectFailed(side_effect::Failed),
    PublicOutputProduced(PublicOutputProduced),
    PublicOutputRenderFailed(PublicOutputRenderFailed),
    StateAttemptCompleted(StateAttemptCompleted),
    StateAttemptFailed(StateAttemptFailed),
    RunCompleted(RunCompleted),
    RetentionRefsAppended(RetentionRefsAppended),
    RetentionManifestProjected(RetentionManifestProjected),
}
```

Non-side-effect v1 payloads must include enough typed evidence for projection rebuild and replay:

```rust
pub struct RunStarted {
    pub run_id: RunId,
    pub spec_hash: SpecHash,
    pub spec_artifact_id: ArtifactId,
    pub spec_media_type: MediaType,
    pub spec_version: SpecVersion,
    pub lowering_version: LoweringVersion,
    pub public_output_schema_id: SchemaId,
    pub descriptor_identities: Vec<DescriptorIdentity>,
    pub runner_executables: Vec<ExecutableIdentity>,
    pub adapter_executables: Vec<ExecutableIdentity>,
    pub canonicalizer_identity: CanonicalizerIdentity,
    pub framework_version: FrameworkVersion,
    pub source_revision: SourceRevision,
    pub seed_cells: Vec<SeedCellRef>,
}

pub struct SeedCellRef {
    pub seed_id: SeedId,
    pub cell_id: CellId,
    pub scope_id: ScopeId,
    pub semantic_type_id: SemanticTypeId,
    pub schema_id: SchemaId,
    pub digest: ContentDigest,
    // schema_id, semantic_type_id, producer_seed_id, digest, byte length,
    // media type, and ArtifactRole::SeedInput are mandatory here.
    pub seed_artifact: ArtifactEvidenceRef,
}

pub struct StateAttemptStarted {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub attempt_no: u32,
    pub state_kind: StateKind,
    pub state_version: StateVersion,
}

pub struct ArtifactReferenced {
    pub spec_hash: SpecHash,
    pub node_id: Option<NodeId>,
    pub attempt_id: Option<AttemptId>,
    pub artifact_ref: ArtifactEvidenceRef,
}

pub struct PublicOutputRenderFailed {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub public_schema_id: SchemaId,
    pub renderer_descriptor_id: DescriptorId,
    pub error: MfmErrorInfo,
}

pub struct StateAttemptCompleted {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub output_cell_id: CellId,
}

pub struct StateAttemptFailed {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub retryable: bool,
    pub error: MfmErrorInfo,
}

pub enum RunCompletionStatus {
    Completed,
    Failed,
    Cancelled,
}

pub struct RunCompleted {
    pub run_id: RunId,
    pub spec_hash: SpecHash,
    pub status: RunCompletionStatus,
    pub public_output_schema_id: Option<SchemaId>,
    pub public_output_event_id: Option<EventId>,
    pub terminal_error: Option<MfmErrorInfo>,
}
```

Side-effect event payloads must carry enough typed evidence to resume without guessing. The `v1`
module uses the following required fields; domain-specific evidence lives in typed artifacts
referenced by digest:

```rust
pub mod side_effect {
    pub struct ClaimFencingToken {
        // store-owned opaque fencing token
    }

    pub struct IntentPersisted {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub scope_id: ScopeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub intent_schema_id: SchemaId,
        pub intent_hash: ContentDigest,
        pub intent_artifact_id: ArtifactId,
        pub idempotency_input_schema_id: SchemaId,
        pub idempotency_input_hash: ContentDigest,
        pub idempotency_key: IdempotencyKeyRef,
        pub capability_kind: CapabilityKind,
        pub capability_version: CapabilityVersion,
        pub adapter_kind: AdapterKind,
        pub adapter_version: AdapterVersion,
    }

    pub struct Claimed {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub claim_owner: RunnerInvocationId,
        pub invocation_epoch: u32,
        pub claim_generation: u32,
        pub claim_fencing_token: ClaimFencingToken,
    }

    pub struct ClaimTakenOver {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub previous_claim_owner: RunnerInvocationId,
        pub new_claim_owner: RunnerInvocationId,
        pub invocation_epoch: u32,
        pub previous_claim_generation: u32,
        pub claim_generation: u32,
        pub claim_fencing_token: ClaimFencingToken,
    }

    pub struct InvocationPrepared {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub claim_generation: u32,
        pub claim_fencing_token: ClaimFencingToken,
        pub prepared_artifact_id: Option<ArtifactId>,
        pub prepared_hash: Option<ContentDigest>,
    }

    pub struct InvocationStarted {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub claim_owner: RunnerInvocationId,
        pub claim_generation: u32,
        pub claim_fencing_token: ClaimFencingToken,
    }

    pub struct NotSubmittedProven {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub proof_schema_id: SchemaId,
        pub proof_hash: ContentDigest,
        pub proof_artifact_id: ArtifactId,
    }

    pub struct SubmissionObserved {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub submission_schema_id: SchemaId,
        pub submission_hash: ContentDigest,
        pub submission_artifact_id: ArtifactId,
    }

    pub struct SubmissionUnknown {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub evidence_schema_id: SchemaId,
        pub evidence_hash: ContentDigest,
        pub evidence_artifact_id: ArtifactId,
    }

    pub struct ReceiptObserved {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub receipt_schema_id: SchemaId,
        pub receipt_hash: ContentDigest,
        pub receipt_artifact_id: ArtifactId,
    }

    pub struct ConfirmationObserved {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub confirmation_schema_id: SchemaId,
        pub confirmation_hash: ContentDigest,
        pub confirmation_artifact_id: ArtifactId,
        pub replay_verifier_id: ReplayVerifierId,
    }

    pub struct Ambiguous {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub ambiguity_code: AmbiguityCode,
        pub evidence_schema_id: SchemaId,
        pub evidence_hash: ContentDigest,
        pub evidence_artifact_id: ArtifactId,
    }

    pub struct Failed {
        pub spec_hash: SpecHash,
        pub node_id: NodeId,
        pub attempt_id: AttemptId,
        pub ledger_key: SideEffectLedgerKey,
        pub invocation_epoch: u32,
        pub failure_phase: FailurePhase,
        pub retryable: bool,
        pub error: MfmErrorInfo,
    }

    pub enum FailurePhase {
        BeforeInvocationStarted,
        AfterNotSubmittedProven,
    }
}
```

The legal transition graph in "Idempotency And Side-Effect Replay" is normative. A store must reject
side-effect events whose `ledger_key`, `invocation_epoch`, claim owner, claim generation, fencing
token, schema ids, or hashes do not match the certified spec and current ledger projection.

The store, not callers, constructs event envelopes. The typed commit API accepts payloads:

```rust
append_typed_run_commit(
    run_id: RunId,
    expected_next_seq: StreamSeq,
    commit_key: CommitKey,
    payloads: Vec<v1::events::KernelEventPayload>,
    required_artifacts: Vec<v1::ArtifactEvidenceRef>,
    preconditions: v1::CommitPreconditions,
) -> CommitResult;
```

The v1 precondition shape is:

```rust
pub struct CommitPreconditions {
    pub required_run_state: RequiredRunState,
    pub required_absent_logical_keys: Vec<LogicalEventKey>,
    pub required_present_logical_keys: Vec<LogicalEventKey>,
    pub required_cell_states: Vec<CellStatePrecondition>,
    pub required_side_effect_states: Vec<SideEffectStatePrecondition>,
    pub required_public_output_absent: bool,
}

pub struct ArtifactEvidenceRef {
    pub artifact_id: ArtifactId,
    pub digest: ContentDigest,
    pub byte_len: u64,
    pub media_type: MediaType,
    pub schema_id: Option<SchemaId>,
    pub semantic_type_id: Option<SemanticTypeId>,
    pub producer_node_id: Option<NodeId>,
    pub producer_seed_id: Option<SeedId>,
    pub artifact_role: ArtifactRole,
}
```

The commit is atomic over appending one ordered event batch to `run:{run_id}`, recording the commit
key, verifying required artifact existence/digest/schema or kind/semantic id/producer, and deriving
cell terminal, side-effect status, retention, and public-output projection writes from validated
payloads. Callers do not provide index writes. The store owns event envelopes, sequence numbers,
ordinals, logical keys, projection writes, and idempotency checks. `side_effect:*` streams, if
exposed, are projections rebuilt from committed run events; they are not a second write authority.
Stores that cannot provide this contract are not certified for typed side-effect runs.

The atomic scope is:

- verify required artifacts exist with the declared digest, schema/kind, semantic id, and producer
- append ordered events to `run:{run_id}`
- record the commit-key idempotency entry
- derive and update cell terminal projections
- derive and update side-effect status projections
- derive and update retention projections
- derive and update public-output projections

The run stream is authoritative. Projection corruption is repairable by rebuilding from the stream;
stream corruption is fatal.

Commit processing order is normative:

1. Look up `commit_key`.
2. If the key exists with the same canonical commit fingerprint, return `AlreadyCommitted` even if
   `expected_next_seq` is now stale.
3. If the key exists with a different canonical commit fingerprint, return `CommitConflict`.
4. Only absent commit keys check `expected_next_seq` against the current stream sequence.
5. If the sequence precondition succeeds, apply logical-key, artifact, and projection
   preconditions atomically with the append.

Idempotence and conflict rules are:

```text
same commit_key + same canonical commit fingerprint -> AlreadyCommitted
same commit_key + different canonical commit fingerprint -> CommitConflict
same cell terminal key + different terminal payload -> Corruption
same side_effect_key + different claim owner/generation/payload -> Corruption
```

Artifacts are written before commit events. Orphan artifacts are acceptable and quarantined. Events
referencing missing or digest-mismatched artifacts are corruption.

### First Certified Persistent Store

The first certified persistent store path should be:

```text
stream-store-postgres + artifact-store-fs
```

`stream-store-postgres` is the append-only authority for `run:{run_id}` events, commit keys, and
derived projections. `artifact-store-fs` stores canonical bytes by digest for local development and
typed-certified-slice replay fixtures. S3 or other artifact stores may be certified later after they implement
the same artifact existence, digest, byte length, media type, retention, and GC refusal contracts.

The initial migrations must create, at minimum:

```text
typed_run_events
typed_commit_keys
typed_artifacts
typed_cell_projection
typed_fact_projection
typed_side_effect_projection
typed_public_output_projection
typed_retention_projection
typed_retention_manifests
```

Certification fixtures for the store must prove:

- contiguous sequence enforcement per run stream
- commit-key idempotency and conflict behavior
- store-owned envelope fields cannot be forged by callers
- required artifact preconditions are atomic with event append
- projections rebuild exactly from `typed_run_events`
- crash/restart around every side-effect durable boundary resumes correctly
- retained artifacts cannot be garbage-collected while referenced by a verified retention projection
- replay from Postgres plus filesystem artifacts does not require live capabilities

### Traceability And Provenance

Every produced value must have a typed value reference:

```rust
pub struct TypedValueRef {
    pub cell: CellId,
    pub semantic_type: SemanticTypeId,
    pub schema_id: SchemaId,
    pub producer: v1::CellProducer,
    pub producer_state_kind: Option<StateKind>,
    pub producer_state_version: Option<StateVersion>,
    pub planning_lineage: PlanningLineage,
    pub scope_id: ScopeId,
    pub config_hash: ContentDigest,
    pub content_digest: ContentDigest,
    pub artifact_id: Option<ArtifactId>,
    pub fact_id: Option<FactId>,
}
```

The certified spec plus typed value refs must make it possible to audit:

- which authoring source created a state
- which state produced a value
- which typed inputs a state consumed
- which artifacts, facts, and outputs belong to a value
- which semantic workflow scope a value belongs to
- which adapter capability was used
- which side-effect intent and receipt were recorded
- why replay or resume re-enters at a type-valid boundary

### Typed Facts, Receipts, And Errors

Capability adapters own fact keys and request hashing. Read facts are committed as typed evidence:

```rust
pub struct FactRecorded {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub capability_kind: CapabilityKind,
    pub capability_version: CapabilityVersion,
    pub adapter_kind: AdapterKind,
    pub adapter_version: AdapterVersion,
    pub request_schema_id: SchemaId,
    pub request_hash: ContentDigest,
    pub response_schema_id: SchemaId,
    pub response_hash: ContentDigest,
    pub fact_key: FactKey,
    pub artifact_id: ArtifactId,
}
```

Read capabilities return recorded facts, not raw values:

```rust
pub struct RecordedFact<T: MfmValue> {
    pub fact_key: FactKey,
    pub request_hash: ContentDigest,
    pub response: T,
    pub response_ref: ArtifactRef<T>,
}
```

Live read semantics:

1. capability adapter canonicalizes the typed request
2. adapter computes `(request_schema_id, request_hash, fact_key)`
3. adapter performs the live read
4. response bytes are canonicalized, no-secret checked, and written as an artifact
5. scheduler/store commits `v1::events::FactRecorded`
6. state receives `RecordedFact<T>` or the typed response derived from it

Failure and retry rules:

- failure before `v1::events::FactRecorded` commits is retryable according to the typed error
- once `v1::events::FactRecorded` commits, resume must reuse the recorded fact for that attempt
- if a state fails after recording one or more facts, retry may record additional facts only in a new
  attempt id; existing facts remain immutable evidence
- replay with a missing fact fails with `MFM_REPLAY_FACT_MISSING`
- replay with request hash, schema id, adapter id, spec hash, or artifact digest mismatch fails
  with a typed replay error
- replay brokers must never fall back to live IO

Replay brokers answer only from recorded facts matching request hash, schema ids, adapter identity,
and spec hash. They must not silently fall back to live IO. Receipts and confirmations use analogous
typed events with intent hash, receipt/confirmation schema ids, artifact ids, and replay verifier
identity. Replay verifiers receive only recorded facts, artifacts, and typed evidence.

Persisted errors must also be typed and redaction-aware:

```rust
pub struct MfmErrorInfo {
    pub code: ErrorCode,
    pub category: ErrorCategory,
    pub retryable: bool,
    pub safe_message: String,
    pub public_details: Option<RedactedJson>,
    pub diagnostic_ref: Option<ArtifactRef<RedactedDiagnostic>>,
}
```

Error payloads are canonical JSON, contain no floats, and contain no secrets. External raw
responses, environment values, signing material, mnemonics, passwords, private keys, and decrypted
secret bytes must not enter persisted error details. Storage-layer secret scanning remains a
backstop, not the primary enforcement mechanism.

Stable typed error codes are part of the public operational contract. The `v1` error code families are:

```text
MFM_CERT_*           certification and descriptor/spec rejection
MFM_REPLAY_*         replay evidence, fact, receipt, verifier, or live-cap violations
MFM_RESUME_*         resume drift, corrupt history, frontier, or ambiguity rejection
MFM_SIDEFX_*         side-effect ledger, recovery, idempotency, or ambiguity errors
MFM_RETENTION_*      retention projection, manifest, executable, or artifact gaps
MFM_PUBLIC_OUTPUT_*  public-output render, schema, evidence, or completion errors
MFM_BOUNDARY_*       crate-boundary, deterministic expansion, or forbidden API violations
MFM_STORE_*          commit, projection, stream, artifact, or corruption errors
```

Initial required codes include:

```text
MFM_CERT_DESCRIPTOR_MISMATCH
MFM_CERT_SPEC_HASH_MISMATCH
MFM_CERT_UNREGISTERED_STATE
MFM_CERT_CAPABILITY_ROLE_INVALID
MFM_REPLAY_FACT_MISSING
MFM_REPLAY_FACT_MISMATCH
MFM_REPLAY_LIVE_CAP_REQUESTED
MFM_REPLAY_EXECUTABLE_IDENTITY_MISMATCH
MFM_REPLAY_CANONICALIZER_MISMATCH
MFM_RESUME_SPEC_DRIFT
MFM_RESUME_CORRUPT_HISTORY
MFM_SIDEFX_AMBIGUOUS
MFM_SIDEFX_IDEMPOTENCY_CONFLICT
MFM_RETENTION_GAP
MFM_RETENTION_MANIFEST_INCOMPLETE
MFM_PUBLIC_OUTPUT_RENDER_FAILED
MFM_PUBLIC_OUTPUT_MISSING_EVIDENCE
MFM_BOUNDARY_FORBIDDEN_DYNAMIC_API
MFM_BOUNDARY_NONDETERMINISTIC_EXPANSION_API
MFM_STORE_COMMIT_CONFLICT
MFM_STORE_CORRUPTION
```

Error codes are stable identifiers. Human-readable messages may improve over time, but code meaning
must not drift without introducing a new code.

### Replay And Resume

Replay and resume must be driven by certified specs, not by best-effort graph reconstruction.

On run start, MFM must persist:

- canonical manifest input
- certified typed execution spec artifact
- certified spec hash
- authoring provenance descriptor
- lowering version
- framework version
- public output schema id

The run event stream must bind events to the certified spec hash. A `v1::events::RunStarted` event
without a spec hash is insufficient for the new architecture.

The new kernel/domain event schema must preserve the existing append-only and attempt-envelope
invariants while adding typed-spec evidence. At minimum, `v1::events::RunStarted` must carry the
certified spec artifact id, certified spec hash, lowering version, framework/build provenance, and
public output schema id. State completion and output events must carry typed cell references rather
than relying on a final context snapshot as the semantic terminal contract.

For typed runs, `v1::events::RunStarted` at `run:{run_id}` sequence `1` is mandatory. The spec
artifact may be written before `RunStarted`, but it is an orphan until the event commits.
`RunStarted` rejects missing specs, digest mismatch, unsupported media type, unsupported spec
version, or unresolved config refs.

`RunStarted` must bind `run_id`, `spec_hash`, `spec_artifact_id`, `lowering_version`,
`public_output_schema_id`, descriptor identities, runner executable identities, adapter executable
identities, and canonicalizer identity. A run whose first event does not provide this evidence is
not a certified typed run.

Retention is event-sourced. The authoritative retention record is an append-only sequence of
retention events in the run stream. Any dedicated retention table, index, or stream is a projection
rebuilt from committed run events, not a second write authority. `v1::RetentionManifest` is a
projection artifact over that history, not mutable authority. MFM never overwrites a retention
manifest; it appends a new projection that links to the previous projection digest and includes all
accumulated retained references.

Each certified run appends retention evidence and may materialize a manifest projection:

```rust
pub struct RetentionManifest {
    pub run_id: RunId,
    pub spec_hash: SpecHash,
    pub manifest_seq: u64,
    pub previous_manifest_digest: Option<ContentDigest>,
    pub spec_artifact: ArtifactRef<v1::TypedExecutionSpec>,
    pub config_artifacts: Vec<ConfigArtifactRef>,
    pub descriptor_identities: Vec<DescriptorIdentity>,
    pub descriptor_digests: Vec<ContentDigest>,
    pub runner_executables: Vec<ExecutableIdentity>,
    pub adapter_executables: Vec<ExecutableIdentity>,
    pub canonicalizer_identity: CanonicalizerIdentity,
    pub event_schema_ids: Vec<SchemaId>,
    pub value_artifacts: Vec<ArtifactId>,
    pub receipt_artifacts: Vec<ArtifactId>,
    pub confirmation_artifacts: Vec<ArtifactId>,
    pub public_output_artifacts: Vec<ArtifactId>,
}
```

```rust
pub struct RetentionRefsAppended {
    pub run_id: RunId,
    pub spec_hash: SpecHash,
    pub refs: Vec<RetentionRef>,
    pub reason: RetentionReason,
}

pub struct RetentionManifestProjected {
    pub run_id: RunId,
    pub spec_hash: SpecHash,
    pub manifest_seq: u64,
    pub manifest_digest: ContentDigest,
    pub previous_manifest_digest: Option<ContentDigest>,
    pub manifest_artifact_id: ArtifactId,
}
```

`v1::ExecutableIdentity` includes the logical factory id plus reproducible code identity: source
revision, package digest, binary digest, Nix derivation/output hash, or an equivalent build artifact
identity. `canonicalizer_identity` records the implementation and version used for canonical JSON
and descriptor hashing. First-slice certified persistent stores retain all manifest entries
indefinitely. In-memory stores are never retention authorities. Local file stores are certified only
if garbage collection refuses referenced artifacts. Garbage collection may act only from a verified
complete retention projection rebuilt from the append-only retention history.

The first certified runtime slice uses composite executable identity rather than one chosen field:

```rust
pub struct ExecutableIdentity {
    pub factory_id: RunnerFactoryId,
    pub source_revision: SourceRevision,
    pub cargo_package_name: PackageName,
    pub cargo_package_version: PackageVersion,
    pub cargo_package_digest: ContentDigest,
    pub binary_digest: ContentDigest,
    pub nix_derivation_hash: Option<NixDerivationHash>,
    pub nix_output_hash: Option<NixOutputHash>,
}

pub struct CanonicalizerIdentity {
    pub name: CanonicalizerName,
    pub version: CanonicalizerVersion,
    pub binary_digest: ContentDigest,
    pub algorithm: CanonicalizationId,
}
```

For the first certified runtime slice, replay requires source revision, package digest, binary digest, and
Nix output hash when available from the build environment. Canonicalizer identity mismatch is always
a replay rejection. Executable identity mismatch is a replay rejection unless the descriptor
lifecycle explicitly records a `ReplayOnly` replacement whose descriptor identity and replay
verifier contract are unchanged.

Long-term replay rehydrates executable code from reproducible built artifacts that contain the
versioned states, adapters, connectors, and framework code referenced by the certified spec. The
same certified spec in the same reproducible environment must resolve to the same executable
artifacts. Replay still uses replay adapters and recorded facts/receipts; reproducible executable
artifacts are the code identity and availability mechanism, not permission to re-query live external
systems.

Descriptor and executable lifecycle states are explicit:

```text
Active
Deprecated
ReplayOnly
Retired
```

New typed runs may use only `Active` descriptors. Resume and replay may use `Active`, `Deprecated`,
or `ReplayOnly` descriptors. `Retired` requires an exported archive bundle or explicit unsupported
run acknowledgment. There is no silent migration: schema, state, adapter, or connector changes
require new versions, and data migration is an explicit typed migration state or operation.

Replay must:

1. load the stored certified spec
2. verify the stored spec hash, descriptor identities, executable identities, adapter identities,
   and canonicalizer identity
3. lower the stored certified spec through registered replay-capable runners
4. execute using replay adapters only
5. verify every fact, submission, receipt, confirmation, typed cell, and terminal output against the
   spec

When the original operation code and manifest input are available, replay may also rebuild the typed
program as a conformance check. If supplied, the rebuilt canonical spec bytes or spec hash must match
the stored certified spec exactly. Replay authority still comes from the stored certified spec, not
from reconstructing authoring code.

Resume must:

1. load the stored certified spec
2. verify the requested code path still produces the same certified spec, or explicitly run against
   the stored spec artifact
3. inspect event history at certified node boundaries
4. resume only from nodes whose predecessors have complete typed cells
5. reject if any completed node has an output semantic id, schema id, content digest, effect record,
   receipt, or provenance record inconsistent with the spec

Matching state ids is not enough. Resume must reject if state kind, state version, state descriptor,
config hash, config bytes/artifact reference, input cells, output type, effect, capability set,
adapter/connector version, idempotency policy, or terminal output shape changed.

Resume must also reject if any completed cell, skipped cell, side-effect record, or public output
record is missing the certified spec hash or disagrees with the certified node/cell descriptors.

Resume validation reconstructs projections from the authoritative run stream before execution:

1. load `run:{run_id}`
2. require contiguous sequences and `v1::events::RunStarted` at sequence `1`
3. load the stored spec artifact and verify hash, media type, and version
4. require a rebuilt spec hash to equal the stored hash when a rebuilt spec is supplied
5. validate event envelopes, event schema ids, spec hashes, logical keys, and commit idempotency
6. verify every cell, side-effect, public-output, submission, receipt, and confirmation event against the
   certified spec
7. resolve registry descriptors and runner factories exactly, including executable identity
8. compute the frontier in topological order: nodes with no terminal output cell and all required
   predecessor cells satisfied
9. block on ambiguous side effects, unsupported adapters, missing runners, corrupt cells, or missing
   public-output evidence for runs that reached the public-output step

Side-effect resume frontier:

```text
terminal cell exists and ledger is confirmation_observed/completed -> skip
terminal cell exists without complete ledger evidence -> corrupt certified history
claim only, no invocation -> take over the claim with a higher claim_generation and continue
invocation_prepared, no invocation_started -> take over the claim with a higher claim_generation
invocation_started_unknown -> recover_submission
submission_unknown -> recover_submission
not_submitted_proven -> claim and prepare next invocation_epoch
submission_observed -> recover_receipt
receipt_observed -> recover_confirmation
confirmation_observed, no cell -> derive output and append cell
side_effect_failed before invocation and retryable -> retry attempt
side_effect_failed after not_submitted_proven and retryable -> claim and prepare next invocation_epoch
side_effect_failed non-retryable -> fail run with typed error
ambiguous -> block
```

Replay uses the stored spec and replay broker only. It re-executes pure, managed-platform-write,
and read states only when replay capabilities can answer from recorded facts/artifacts and retained
managed platform outputs. It verifies produced value digests, receipts, confirmations, and public
outputs through typed evidence and replay-only verifier contracts. Replay fails on missing facts,
missing receipts, unsupported adapter versions, unavailable runner factories, retention gaps,
executable identity mismatch, canonicalizer identity mismatch, or any live-cap request.

### Terminal Outputs And Public API

Operation outputs must be typed structs of handles. String exports are not sufficient. Internal
operation outputs may be typed helper structs, but any output exposed by CLI, REST, or other public
launch/render surfaces must implement the public output contract.

The public output contract must be derived from a typed output spec:

```rust
pub trait PublicOutputs<'p, 's> {
    fn public_schema_descriptor() -> SchemaDescriptor;

    fn public_schema_id() -> SchemaId {
        SchemaId::derive(&Self::public_schema_descriptor())
    }

    fn public_output_spec(&self) -> v1::PublicOutputSpec;

    fn output_cells(&self) -> Vec<PublicOutputCell>;
}
```

The persisted public-output spec is hash-defining:

```rust
pub struct PublicOutputSpec {
    pub public_schema_id: SchemaId,
    pub outputs: Vec<PublicOutputCell>,
    pub renderer_descriptor: RendererDescriptorIdentity,
}

pub struct PublicOutputCell {
    pub public_field_path: PublicFieldPath,
    pub cell_id: CellId,
    pub producer: v1::CellProducer,
    pub scope_id: ScopeId,
    pub semantic_type_id: SemanticTypeId,
    pub schema_id: SchemaId,
    pub value_lineage: ValueLineageRef,
    pub required_terminal: RequiredTerminal,
}

pub struct RendererDescriptorIdentity {
    pub descriptor_id: DescriptorId,
    pub renderer_kind: RendererKind,
    pub renderer_version: RendererVersion,
    pub public_schema_id: SchemaId,
    pub canonicalizer_identity: CanonicalizerIdentity,
}
```

CLI and API rendering must load typed terminal cells and render stable JSON from the public output
schema. The renderer may emit JSON, but it must not discover final results by looking up arbitrary
context keys.

Artifact-bearing outputs must use typed artifact refs. Public output schemas are part of the
user-facing API. They require rustdoc and versioning. Public output schema ids use the same derived
schema descriptor mechanism as MFM values and configs. Breaking public output changes require new
public output schema versions; automatic public-output migration is out of scope for the typed core.

`v1::events::RunCompleted(Completed)` is invalid without public-output evidence. A completed run must have
typed public output refs bound to the certified spec hash and public schema id. Context snapshots may
remain for observability and debugging, but they are not the public output contract.

The terminal public-output record is:

```rust
pub struct PublicOutputProduced {
    pub spec_hash: SpecHash,
    pub node_id: NodeId,
    pub attempt_id: AttemptId,
    pub receipt_cell_id: CellId,
    pub public_schema_id: SchemaId,
    pub output_spec_digest: ContentDigest,
    pub cells: Vec<NamedTypedCellRef>,
    pub rendered_digest: ContentDigest,
    pub rendered_artifact_id: Option<ArtifactId>,
    pub renderer_descriptor_id: DescriptorId,
}

pub struct NamedTypedCellRef {
    pub public_field_path: PublicFieldPath,
    pub cell_id: CellId,
    pub producer: v1::CellProducer,
    pub scope_id: ScopeId,
    pub semantic_type_id: SemanticTypeId,
    pub schema_id: SchemaId,
    pub value_lineage: ValueLineageRef,
    pub content_digest: ContentDigest,
    pub artifact_id: ArtifactId,
}
```

Typed terminal cells plus `v1::PublicOutputSpec` are authoritative. Rendered JSON is only a cache for
CLI/API clients. Public-output rendering is an explicit render state attempt. The render node is a
normal typed node whose `v1::NodeSpec::output_cell` stores a `PublicOutputReceipt` value. Its
successful terminal commit includes `v1::events::CellProduced` for that receipt cell,
`PublicOutputProduced`, and `v1::events::StateAttemptCompleted` whose `output_cell_id` equals
`receipt_cell_id`. If the
required cells exist but rendering fails, the run is not completed; the render state emits
`v1::events::StateAttemptFailed`, may also emit `v1::events::PublicOutputRenderFailed` as audit
detail, and remains resumable from the public-output rendering step.
`v1::events::PublicOutputRenderFailed` is not a terminal authority and cannot substitute for
`v1::events::PublicOutputProduced`.

```rust
pub struct PublicOutputReceipt {
    pub public_schema_id: SchemaId,
    pub output_spec_digest: ContentDigest,
    pub cells: Vec<NamedTypedCellRef>,
    pub rendered_digest: ContentDigest,
    pub rendered_artifact_id: Option<ArtifactId>,
    pub renderer_descriptor_id: DescriptorId,
}
```

The public-output render state is a framework-owned node inserted during typed program finalization
whenever `RootBuilder::bind_public_outputs` succeeds, before certification and spec hashing. Users
do not hand-author terminal render nodes. The `RenderPublicOutputs` framework node is persisted in
`v1::TypedExecutionSpec::nodes` through `v1::NodeSpec::framework`, depends on the declared public cells,
records the renderer descriptor and public-output spec digest, emits `v1::events::PublicOutputProduced`, and
is the only path to `v1::events::RunCompleted(Completed)` for public runs. Certified lowering may only bind
this persisted node to its framework runner.

Resume behavior for rendering is explicit:

```text
required public cells incomplete -> render node not runnable
required public cells complete, no render attempt -> run render node
render attempt failed -> retry render node with same declared cells and renderer descriptor
PublicOutputProduced exists, no RunCompleted -> append RunCompleted(Completed)
RunCompleted without PublicOutputProduced -> corrupt certified history
```

Changing renderer descriptor identity, public schema id, declared cells, or canonicalizer identity
changes the certified spec hash and is resume drift.

### Portfolio Example

The workflow examples in this section are API-shaped pseudocode unless the shown helper methods are
defined by the implementation. In particular, `b.scope::<...>` is shorthand for the real root or
child-scope builder API and must become either a compile-pass fixture or explicitly marked
pseudocode before implementation closes the corresponding milestone.

The portfolio workflow becomes typed staged expansion:

```rust
b.scope::<PortfolioExecution>(|b| {
    let prepared = b.state::<PrepareSources, _>(
        StableNodeKey::new("prepare_sources"),
        cfg.sources,
        (),
    )?;

    let subjects = b.state::<ResolveSubjects, _>(
        StableNodeKey::new("resolve_subjects"),
        cfg.subjects,
        prepared,
    )?;

    let views = b.state::<PinViews, _>(
        StableNodeKey::new("pin_views"),
        cfg.views,
        prepared,
    )?;

    let valuations = b.state::<ResolveValuations, _>(
        StableNodeKey::new("resolve_valuations"),
        cfg.valuations,
        views,
    )?;

    let batches: Vec<Handle<'_, '_, ObservationBatchOutput>> = cfg
        .batches
        .canonical_sorted()
        .map(|batch| {
            b.state::<ObserveBatch, _>(
                StableNodeKey::from_domain_key(batch.key()),
                batch.config,
                (subjects, views, valuations),
            )
        })
        .collect::<Result<_, _>>()?;

    let observations = b.state::<MergeObservations, _>(
        StableNodeKey::new("merge_observations"),
        cfg.merge,
        batches,
    )?;

    let snapshot = b.state::<AssembleSnapshot, _>(
        StableNodeKey::new("assemble_snapshot"),
        cfg.snapshot,
        (subjects, views, observations),
    )?;

    let snapshot_artifact = b.state::<PublishSnapshot, _>(
        StableNodeKey::new("publish_snapshot"),
        cfg.snapshot_artifact,
        snapshot,
    )?;

    let report = b.state::<ProjectReport, _>(
        StableNodeKey::new("project_report"),
        cfg.report,
        snapshot,
    )?;

    Ok(PortfolioOutputs {
        snapshot,
        snapshot_artifact,
        report,
    })
})
```

The compiler prevents wiring `PinnedViews` where `ResolvedSubjects` is required. The shared scope
prevents assembling a snapshot from values belonging to different portfolio execution instances
unless an explicit bridge is introduced.

Zero observation batches must be modeled explicitly. For the first portfolio port,
`MergeObservations` should require `NonEmptyHandles<ObservationBatchOutput>` unless the product
contract deliberately defines an empty snapshot as meaningful. If empty snapshots are later
supported, they must use a distinct workflow contract with `Vec<Handle<ObservationBatchOutput>>`
and documented public-output semantics for empty observations.

Portfolio dynamic keys must be typed `StableDomainKey` values, not strings. Required first-port
domain keys include source key, subject key, view key, valuation key, observation batch key, and
report key. Duplicate resolved domain keys are planning errors. CLI/API parity for portfolio applies
only to documented public JSON fields; content ids, spec hashes, event ids, and retained artifact
ids remain semantic evidence and should not be normalized away in parity fixtures.

### Proof Example

The proof workflow becomes:

```rust
let draft = build_root(ScopeKey::new("proof"), |root| {
    let b = root.scope();

    let fact = b.state::<ReadProofFact, _>(
        StableNodeKey::new("read_fact"),
        cfg.read,
        (),
    )?;

    let side_effect = b.state::<ApplyProofSideEffect, _>(
        StableNodeKey::new("apply_side_effect"),
        cfg.apply,
        fact,
    )?;

    let output = b.state::<AssembleProofOutput, _>(
        StableNodeKey::new("assemble_output"),
        cfg.assemble,
        (fact, side_effect),
    )?;

    let artifact = b.state::<PublishOutput, _>(
        StableNodeKey::new("publish_output"),
        cfg.publish,
        output,
    )?;

    root.bind_public_outputs(
        PublicOutputKey::new("proof"),
        &ProofPublicOutputs { artifact },
    )
})?;
```

`ApplyProofSideEffect` declares `Effect = ApplySideEffect`, typed intent, typed idempotency input,
typed receipt, and typed confirmation. `PublishOutput` requires a `ProofOutput`, so it cannot run
before `AssembleProofOutput` has produced one.

The proof domain contract for every proof implementation is explicit:

```rust
pub struct ProofFact { /* recorded external observation */ }
pub struct ProofIntent { /* mutation or proof action to apply */ }
pub struct ProofIdempotencyInput { /* stable dedupe material */ }
pub struct ProofReceipt { /* submitted or accepted evidence */ }
pub struct ProofConfirmation { /* confirmed final evidence */ }
pub struct ProofOutput { /* terminal domain result */ }

pub struct ProofPublicOutputs<'p, 's> {
    pub artifact: Handle<'p, 's, ArtifactRef<ProofOutput>>,
}
```

The proof replay verifier receives only recorded facts, artifacts, side-effect receipt evidence, and
confirmation evidence:

```rust
pub trait ProofReplayVerifier {
    fn verify_receipt(
        &self,
        intent: &ProofIntent,
        receipt: &ProofReceipt,
        facts: &RecordedProofFacts,
    ) -> Result<(), ReplayError>;

    fn verify_confirmation(
        &self,
        receipt: &ProofReceipt,
        confirmation: &ProofConfirmation,
        facts: &RecordedProofFacts,
    ) -> Result<(), ReplayError>;
}
```

Different proof implementations may provide different adapters and verifier implementations, but
they must expose equivalent typed contracts: fact, intent, idempotency input, receipt,
confirmation, output, public outputs, and replay verifier behavior. A proof implementation that
requires live IO during replay is not conformant.

### EVM Deploy / Configure / Validate Example

The deploy/configure/validate workflow should encode lifecycle stages:

```rust
b.scope::<DcvExecution>(|b| {
    let built = b.state::<BuildDcvConfig, _>(
        StableNodeKey::new("build_config"),
        cfg.build,
        (),
    )?;

    let deploy_cfg = b.state::<SelectDeployConfig, _>(
        StableNodeKey::new("select_deploy_config"),
        cfg.deploy,
        built,
    )?;

    let configure_cfg = b.state::<SelectConfigureConfig, _>(
        StableNodeKey::new("select_configure_config"),
        cfg.configure,
        built,
    )?;

    let validate_cfg = b.state::<SelectValidateConfig, _>(
        StableNodeKey::new("select_validate_config"),
        cfg.validate,
        built,
    )?;

    let deployed = b.state::<DeployContract, _>(
        StableNodeKey::new("deploy_contract"),
        (),
        deploy_cfg,
    )?;

    let configured = b.state::<ConfigureContract, _>(
        StableNodeKey::new("configure_contract"),
        (),
        (configure_cfg, deployed),
    )?;

    let validated = b.state::<ValidateContract, _>(
        StableNodeKey::new("validate_contract"),
        (),
        (validate_cfg, configured),
    )?;

    Ok(validated)
})
```

`ValidateContract` expects a `ConfiguredContract`, not a `DeployedContract` or raw contract address.
Validation before configuration is therefore unrepresentable in Rust-authored workflows.

This lifecycle guarantee applies to the deploy/configure/validate workflow contract. A separate
"validate an existing deployment" workflow may still be valid, but it must take an explicitly typed
input such as `ExistingContractRef` or `ConfiguredContractRef` rather than reusing the DCV lifecycle
path with a raw string address.

Deploy and configure are side-effecting EVM states. They must use typed EVM transaction intents,
idempotency keys, receipts, and confirmations. Validate is normally a read state using typed EVM
read capabilities.

EVM capability roles are fixed for the first port:

- signer and keystore capabilities are support capabilities
- EVM read RPC is a read capability
- EVM transaction submitter is the single external mutation authority for each side-effect state
- raw protected transactions are managed artifacts only and must not implement `MfmValue`,
  `PublicOutputs`, or any public-output wrapper trait

EVM side-effect acceptance fixtures must run against a managed local reth service and cover crash or
restart at these boundaries:

```text
intent persisted
claim committed
invocation prepared
invocation started before submit
submission observed
receipt observed
confirmation observed
output cell before RunCompleted
```

Fixtures must prove no duplicate mutation occurs, replay confirms from recorded evidence without
reapplying, protected raw transaction artifacts remain non-public and redacted, and validation
before configuration remains a compile-fail typestate error.

### Plugin And Third-Party Workflows

Rust-authored third-party states and operations can compile against the typed framework and receive
the same compile-time guarantees.

Dynamic third-party plugins are required as a future capability, but the first architecture slice
does not need to solve them. The typed core must be designed so dynamic plugins can be added later
without reopening the semantic contract.

Dynamic plugins cannot be allowed to bypass the typed core by submitting arbitrary erased graphs. A
future dynamic plugin design must either:

- expose Rust types and compile as a typed extension, or
- submit a declarative typed spec that passes runtime certification against trusted registered
  state descriptors, semantic type ids, schema ids, state kinds and versions, effect declarations,
  capabilities, idempotency contracts, adapter/connector versions, and public outputs.

If a plugin cannot provide that evidence, it cannot participate in certified MFM execution.

Until that future design exists, dynamic plugin execution should remain out of scope for certified
MFM runs. The typed Rust-authored core should not keep compatibility hooks that allow arbitrary
dynamic graphs to enter the runner.

### Compile-Time Guarantees

After the typed authoring API replaces `PlannedOp`, `PortKey`, `DynContext`, and public dynamic DAG
construction, this architecture should make the following classes of mistakes unrepresentable in
Rust-authored programs:

- consuming `PortfolioSnapshot` before it exists
- passing `PinnedViews` where `ResolvedSubjects` is required
- projecting a portfolio report without a `PortfolioSnapshot`
- declaring an operation result that no state produced
- validating an EVM deployment before configuration
- applying a side effect without typed idempotency material
- applying a side effect without a typed intent and receipt
- using external adapters from a pure state
- using a transaction adapter from a read-only state
- hand-authoring dependency edges that lie about data flow
- confusing same-shape values with different semantic types
- mixing values from separate workflow scopes without an explicit bridge
- treating a skipped value as a produced value without accepting typed optionality
- exporting terminal results through string context keys
- using a runtime-produced value to change same-run topology

These guarantees apply to the Rust-authored typed API. Persisted specs, dynamic future plugins,
registry resolution, replay data, and external systems still require certification/runtime checks.
Framework-mediated capability access should be compile-time constrained; ambient IO inside arbitrary
Rust code must be controlled by crate boundaries and lint policy.

These guarantees should be locked with compile-fail tests using `trybuild` before workflow ports
begin.

Required compile-fail cases:

- wrong producer type passed to a consumer
- same Rust type from wrong scope passed without a bridge
- forged handle construction from a raw `CellId`
- pure state attempts to access IO
- read state attempts to submit a transaction
- side-effect state lacks idempotency input
- side-effect state lacks typed receipt
- operation output references an unavailable handle
- terminal output lacks a public schema id
- dynamic state graph bypasses the typed builder
- runtime value is used where planning config is required
- secret-bearing type attempts to cross the state-machine boundary as an `MfmValue`
- produced-only consumer is passed a `MaybeValue<T>` or skipped cell without explicit handling
- root public launch/render output does not implement `PublicOutputs`

### Runtime Responsibilities

Some checks remain runtime concerns:

- malformed user input before it is decoded into typed config
- external IO failures
- chain state and external system behavior
- database, HTTP, and storage availability
- replay fact availability
- corrupted persisted artifacts
- clock-dependent observations
- concurrency and scheduler failures
- domain facts observed from external systems

This is the intended boundary. Runtime should handle facts unknowable before execution, not recover
from invalid semantic wiring.

Config-derived dynamic fanout cardinality, duplicate generated keys, and canonical ordering are
planning/certification concerns. Runtime cardinality remains relevant only when it comes from facts
that truly cannot be known until execution.

### Crate Boundary Enforcement

The crate layout in "Crate And Module Layout" is normative for the typed rewrite. The key design
choice is that state and operation crates compile against authoring and capability contracts, not
against runtime scheduling, storage, or old dynamic machine APIs.

The thin-layer principle becomes:

- reusable executable behavior belongs in typed states and typed capabilities/connectors
- ops assemble typed state programs through `mfm-program`
- storages implement `mfm-store` and derive projections from append-only events
- transports implement capability backends and live/replay adapters
- binaries parse input, assemble registries/stores/capabilities, start/resume/replay runs, and
  render typed public outputs only

Current repository cleanup priorities follow from this DAG:

- introduce `crates/kernel/*` without depending on old `crates/machine` or `crates/sdk`
- move semantic ids, schema ids, canonical value traits, and descriptor helpers into the kernel
- keep portfolio config/model/plan crates pure and below executable runtime crates
- keep executable Aave/EVM behavior in state or transport crates, not pure model crates
- keep process, environment, filesystem, clock, randomness, and network access inside transports,
  apps, binaries, or explicitly marked tests
- delete or isolate old semantic APIs instead of building compatibility wrappers around them

`crate-dag` in `nix run .#check` must enforce these dependency directions through
`cargo metadata --no-deps`.

### Developer Extension Model

Once the platform is established, external developers should usually implement only values, states,
and operations. The `typed-kernel-contract` slice is derive-first and explicit-trait-first; broad
`#[mfm_state]` and `#[mfm_operation]` attribute macros are long-term ergonomic wrappers, not
requirements for this slice:

```rust
#[derive(MfmValue)]
pub struct MyOutput {
    // stable public fields
}

#[derive(MfmConfig)]
pub struct MyConfig {
    // canonical planning fields
}

#[mfm_state(effect = Pure)]
impl MyState {
    // typed config, typed input, typed output
}

#[mfm_operation]
impl MyOperation {
    // deterministic expansion over states/operations
}
```

The framework should eventually provide:

- typed builder APIs
- canonical config hashing
- derive macros for value/config/output schema descriptors
- thin state/operation attribute macros that generate descriptors without hiding builder semantics
- stable id derivation
- adapter mocks
- replay harnesses
- idempotency helpers
- certified lowering
- trace rendering
- public output rendering
- compile-fail test helpers
- integration-test helpers for live and replay modes

Future developers should not need to understand the erased runner plan, event encoding details, or
manual graph validation to create correct states and operations.

Macros should reduce boilerplate, not conceal the typed model. Generated code should still expose
clear type errors for wrong handles, wrong scopes, missing public outputs, and invalid effect/cap
declarations.

### Rust Type System Fit

The proposal should use Rust's type system aggressively but not encode the entire DAG as nested
types.

Good fits:

- associated types for state config, input, output, effect, and capabilities
- generic associated types for operation outputs branded by program and scope lifetimes
- invariant phantom/lifetime branding for program and scope isolation
- sealed traits for framework-owned effect markers and capability set internals
- typestate values for lifecycle transitions such as deployed -> configured -> validated
- private constructors for handles, cells, nodes, certified plans, and capability tokens
- derive-first schema/value/config/public-output traits with framework-owned manual exceptions only
- const generics for narrow cases such as fixed arity, at-least-N wrappers, or chain-id-branded
  values, but not for whole-DAG encoding
- trait objects only after certification
- `trybuild` tests for API misuse

Risks:

- overly nested generics can make error messages unusable
- operation output GATs may require careful API design
- object safety before certification can distort the authoring model
- monomorphization cost may increase with many states
- macros can hide important type errors if they are too magical

The right compromise is typed handles plus a typed IR. Handles give compile-time producer/consumer
and scope guarantees without requiring the whole workflow graph to exist as one enormous generic
type.

### Complexity Reduction

The typed expansion core should reduce or eliminate several categories of defensive code:

- manual semantic port compatibility checks
- repeated context key shape checks
- producer/consumer schema checks for known typed edges
- terminal output existence checks
- hand-authored dependency edges
- review-only semantic ordering conventions
- deserialization checks caused by wrong wiring
- repeated lineage recovery checks
- duplicated side-effect precondition checks
- ad hoc replay fact lookup inside states
- adapter access checks performed after state construction

Some validation remains, but it moves to the correct layers: canonicality, storage integrity,
external IO behavior, replay availability, idempotency ambiguity, and domain facts.

### Typed Kernel Contract Acceptance

The first mandatory CI gate should prove the typed authoring, spec, descriptor, event-schema, and
certification contracts before the full persistent runtime is built. The gate is named
`typed-kernel-contract`. It uses a synthetic reference program and negative fixtures; it does not
require Postgres, filesystem artifact persistence, replay/resume execution, retention projection,
portfolio migration, EVM services, or a real proof backend.

```text
reference program expands through typed API
root public outputs bind inside build_root
typed handles, scopes, bridges, and lineage lower into v1 spec evidence
registered state and operation descriptors are required
certified spec hash is deterministic
v1 event schemas exist as golden fixtures
old dynamic authoring APIs are absent from the typed kernel dependency graph
```

If this gate still depends on semantic JSON context dataflow, generic IO, public erased DAG
construction, or hand-authored dependency edges, Proposal 1 has not solved the problem.

The `typed-kernel-contract` summary must include these keys and fail if any key is missing, false,
zero where a positive count is required, skipped, or marked expected-failure:

```text
kernel_crates_present
typed_boundary_firewall_passed
value_config_output_derives_present
manual_value_config_output_impls_rejected
registered_state_required
registered_operation_required
bridge_session_evidence_required
semantic_lineage_evidence_present
stable_ids_golden
typed_spec_hash_deterministic
v1_event_schema_golden
invalid_topology_rejected
invalid_interface_wiring_rejected
invalid_semantic_transition_rejected
invalid_data_shape_rejected
invalid_data_meaning_rejected
invalid_terminal_shape_rejected
```

The first full runtime/store gate is named `typed-certified-slice`. It runs a minimal reference
certified workflow on the serial typed scheduler and exercises typed storage, managed platform
writes, one deterministic local side-effect adapter, public outputs, replay, resume, and retention.
It is the next milestone after `typed-kernel-contract`, not a prerequisite for starting typed-core
engineering.

The `typed-certified-slice` summary must include these keys and fail if any key is missing, false,
zero where a positive count is required, skipped, or marked expected-failure:

```text
typed_spec_hash_persisted
run_started_v1_present
seed_material_persisted
cell_events_count
side_effect_ledger_complete
side_effect_invocation_started_before_submit
side_effect_crash_cases_passed
side_effect_no_duplicate_submit
side_effect_submission_unknown_recovered
side_effect_failed_semantics_covered
side_effect_logical_key_conflicts_rejected
managed_platform_outputs_committed
public_output_before_run_completed
public_output_event_id
replay_live_cap_requests_count == 0
resume_drift_fixture_count
resume_drift_rejected
retention_projection_complete
```

Proof implementations are validated by a reusable conformance suite rather than by hard-coding a
single proof backend into the kernel gate:

```text
proof-implementation-conformance
  proof_impl:{name}:facts_valid
  proof_impl:{name}:submission_valid
  proof_impl:{name}:receipt_valid
  proof_impl:{name}:confirmation_valid
  proof_impl:{name}:replay_valid
```

Each proof implementation must satisfy the same typed proof contracts, but the kernel gate remains
stable as new proof implementations are added.

### Test And CI Plan

The typed core should add named suites rather than rely on broad integration tests to find semantic
breakage:

```text
typed-core-trybuild
schema-canonicalization
stable-id-determinism
descriptor-registry-certification
lowering-certification
typed-store-events
storage-commit-contract
side-effect-ledger
adapter-recovery-conformance
replay-resume
public-output-terminal
no-secret-no-float-derives
retention-event-sourcing
crate-dag
typed-boundary-firewall
typed-kernel-contract
typed-certified-slice
proof-implementation-conformance
```

Nixfied mapping:

```text
nix run .#check
  crate-dag
  typed-boundary-firewall
  deterministic-expansion-lint
  source boundary checks
  typed-kernel-contract summary schema

nix run .#test
  typed-core-trybuild
  schema-canonicalization
  stable-id-determinism
  descriptor-registry-certification
  lowering-certification
  typed-store-events
  storage-commit-contract
  side-effect-ledger
  adapter-recovery-conformance
  replay-resume
  public-output-terminal
  no-secret-no-float-derives
  retention-event-sourcing
  typed-kernel-contract

nix run .#ci -- --mode parity --summary
  workflow parity only for typed workflow ports enabled in this checkout

nix run .#ci -- --mode full --summary
  typed-kernel-contract acceptance
  typed-certified-slice acceptance once the runtime milestone is enabled
  proof implementation conformance only for enabled proof implementations
```

Required compile-fail coverage includes:

- root closure attempts to return a handle instead of `RootBound`
- runtime value attempts to become a same-run root seed
- child handle used in parent without a bridge
- sibling handles mixed without a parent bridge
- forged handle construction from a raw `CellId`
- `serde_json::Value` used as state input
- wrong state input field type
- empty vector passed to `NonEmptyHandles<T>`
- operation output without `OperationOutput`
- root launch output without `PublicOutputs`
- pure state names capabilities
- read state receives mutation authority
- side-effect state has zero or two mutation authorities
- secret field derives `MfmValue`
- float field derives `MfmValue`, `MfmConfig`, or `PublicOutputs`
- unsupported serde attribute in a persisted type
- manual `impl MfmValue`, manual `impl MfmConfig`, or manual public-output impl outside framework
  allowlists
- unregistered state or operation reaches builder planning
- same-type same-scope values with mismatched required lineage are bound without an explicit
  transform, bridge, or projection state

Every RFC code example that is intended to be real API should become a fixture:

- compile-pass fixtures for happy-path builder examples
- compile-fail fixtures for wrong handle type, wrong scope, wrong typestate, wrong capability role,
  missing public outputs, forged ids, and invalid input lifting
- explicitly marked pseudocode for any example that intentionally omits required boilerplate

The RFC should not contain unmarked examples that cannot be represented by the implemented API.

Required runtime/unit/integration coverage includes:

- bridge node spec and completion evidence
- duplicate scope/node/operation key rejection
- duplicate seed key rejection, missing seed artifact rejection, and seed digest mismatch rejection
- stable identity golden vectors for scope, operation, node, cell, input binding, config ref, and
  collection ordering
- canonical vector/domain-key ordering
- schema descriptor golden hashes
- canonical JSON duplicate-key rejection
- decimal negative-zero rejection and bytes grammar
- `RunStarted` missing, wrong sequence, or digest mismatch
- typed commit idempotence and conflict behavior
- typed commit idempotency taking precedence over stale `expected_next_seq`
- typed commit caller cannot forge sequence, ordinal, or event id
- typed commit artifact precondition atomicity
- projection rebuild from authoritative run stream
- typed fact request hash/schema/adapter mismatch during replay
- secret-shaped typed error detail rejection
- mandatory fact/artifact/cell/public-output evidence ignores event profiles
- output artifacts use output role/kind metadata, not fact-payload metadata
- cell produced/skipped conflicts
- side-effect ledger legal and illegal transitions
- side-effect claim takeover after claim/prepared, stale owner rejection, and takeover rejection after
  invocation started
- crash matrix around intent, claim, invocation, submission, receipt, and confirmation
- non-side-effect attempt recovery for pure, read, and managed-write states
- adapter recovery conformance
- side-effect replay verifier with no live caps
- replay with no live caps
- resume ambiguity blocks
- retained executable/canonicalizer identity mismatch
- retention event projection and manifest completeness
- portfolio config compilation does not depend on runtime state crates
- portfolio adapter reader payloads do not live in stable semantic metadata
- public-output render failure resumes through render state attempt
- typed kernel does not depend on old dynamic machine or SDK crates
- old dynamic APIs cannot submit certified typed execution specs

### Workflow Migration Gates

Each workflow has three gates:

```text
entry gate: typed prerequisites exist; old semantic APIs are not dependencies of the typed workflow
exit gate: no semantic dependency on old authoring APIs remains
certification gate: CI proves public behavior, replay/resume, and typed-boundary compliance
```

Proof implementation exit:

```text
typed spec
persisted spec hash
RunStarted
typed cells
side-effect ledger
managed platform outputs
typed public output before completion
replay-only caps
drift rejection
no old SDK semantic imports
no PortKey
no semantic JSON context dataflow
no hand-authored edges
no generic IoProvider
```

Portfolio exit:

```text
no PortKey
no context dataflow
no hand-authored edges
no old SDK semantic imports
deterministic fanout/fanin
replay/resume tests
StableDomainKey for all dynamic batches
duplicate batch/domain key rejection
typed terminal public outputs
documented CLI/API field parity
```

EVM exit:

```text
typed lifecycle only
signer/keystore are support caps
one external mutation authority per side-effect state
replay confirms without reapply
crash-boundary tests prove no duplicate mutation
deploy -> configure -> validate typestate
validation-before-configuration compile-fail
protected raw tx never implements public value/output traits
```

### Public Behavior Parity

Byte-for-byte parity applies only to documented stable CLI/API JSON fields. Content IDs must not be
normalized away by default because they are semantic evidence unless a versioned canonicalization
transition explicitly says otherwise.

Internal events use schema-level parity. Side-effecting workflows require behavioral parity plus
ledger/replay invariants against deterministic local services.

### Macro Readiness

First-slice macro readiness requires:

```text
MfmValue derive
MfmConfig derive
StateInput derive
OperationOutput derive
PublicOutputs derive
derive diagnostics covered by trybuild
generated descriptors have golden fixtures
generated code snapshots or equivalent review surface exist
no operation/state attribute macros
no generated runtime scheduling logic
```

Broad state/operation attribute macros may be reconsidered only after proof and portfolio
migrations demonstrate stable diagnostics.

### Documentation Gates

Update docs with the change that introduces each contract:

```text
docs/design.md: typed spec as normative execution contract
docs/architecture.md: refined crate DAG and typed runtime boundary
docs/ops-and-states.md: per-workflow typed inventory
crates/kernel/runtime/README.md: typed scheduler, event, replay, and resume semantics
crates/kernel/program-derive/README.md: derive surface
new kernel crate READMEs for ids, values, program, spec, store, runtime, and replay
bin/cli/README.md: typed public-output rendering when behavior changes
migration notes: old dynamic APIs deleted or isolated from the typed kernel
```

### Rejected Alternatives

Proposal 1 explicitly rejects:

- returning handles from generative build/scope closures
- raw bridge exports without target scope and certified bridge evidence
- operation inputs that can carry unbranded dynamic values
- public handle constructors or raw `CellId`/`OutputCellId` to handle conversion
- encoding the whole DAG as nested Rust types
- multi-output state cells in v1
- JSON Schema as the descriptor source of truth
- context snapshots as terminal output
- mutable cell tables as authority
- generic `IoProvider` or untyped capability bags
- state-authored string idempotency keys
- `apply_started` as mutation-status evidence
- apply-then-record side-effect ordering
- blind retry based only on idempotency keys
- independent authoritative `run:*` and `side_effect:*` streams
- permanent `mfm-sdk` compatibility facade
- compatibility lowering into the current scheduler as the first typed executor
- legacy allowlists as a typed-core migration strategy
- byte-for-byte parity for internal event streams
- broad state/operation attribute macros in the first slice

### Migration Plan

Backward compatibility is not required. The migration must avoid cementing the old model as the new
foundation by building the typed kernel beside the old implementation and moving workflows onto the
new system.

Recommended order:

1. Treat this RFC as the normative contract while implementation lands.
2. Add `crates/kernel/ids`, `canonical`, `values`, `effects`, `capabilities`, `program`,
   `program-derive`, `spec`, `certify`, `events`, and `test-support`.
3. Enforce the kernel crate DAG so the new kernel cannot depend on old `crates/machine` or
   `crates/sdk`.
4. Add strong identity types, schema grammar, descriptor evidence, canonical hashing, and registry
   certification.
5. Add framework-owned executable-state evidence, capability-set evidence, typed handles,
   root/child scope branding, domain input lifting, public-output binding, bridge session evidence,
   and value lineage evidence.
6. Make `typed-kernel-contract` mandatory.
7. Add `crates/kernel/store`, `runtime`, and `replay`.
8. Add typed kernel events, store-owned event envelopes, typed commit API, value store,
   event-sourced retention, replay/resume validator, and side-effect ledger.
9. Add the new serial typed scheduler/executor.
10. Add the reference certified workflow, make `typed-certified-slice` mandatory, and add reusable
   proof implementation conformance for enabled proof implementations.
11. Rewrite `docs/design.md` and `docs/architecture.md` to match the implemented typed contracts.
12. Port portfolio with typed fanout/fanin, stable domain keys, and typed public outputs.
13. Port EVM deploy/configure/validate with typed lifecycle and side-effect contracts.
14. Delete or isolate old dynamic semantic APIs as workflows leave them.
15. Move `crates/sdk` to typed reexports only or delete it.

There is no legacy allowlist for typed-core semantics. If old code must remain temporarily for
unmigrated workflows, it is outside the typed kernel dependency graph and cannot submit or resume a
certified typed run.

### Old Run Operational Policy

Old dynamic runs are not silently migrated into typed certified runs. Existing dynamic runs may
remain inspectable by legacy code, but the typed runtime refuses to resume an uncertified dynamic run
with a stable typed error. Old `PlannedOp` values are never accepted as typed-run authority. Any
data migration from old persisted shapes to typed shapes must be an explicit typed migration state or
operation with versioned input and output schemas.

### Deferred Design Decisions

The following decisions are intentionally not required for the `typed-kernel-contract` slice, but they
must remain explicit open design items:

- dynamic third-party plugin certification and loading
- long-term retention/deprecation policy beyond typed-certified-slice indefinite retention
- permanent side-effect ambiguity recovery beyond v1 block-and-surface semantics
- broad state/operation attribute macro ergonomics

These are deferred because the typed kernel and typed certified runtime slices can validate the core
without finalizing long-term platform policy. The `typed-kernel-contract` slice must define enough
version and artifact evidence in the spec/event schemas for the later runtime slice to replay and
resume the specs it certifies. These decisions must not be resolved by reintroducing dynamic erased
graphs, string ports, or JSON context dataflow as semantic surfaces.

Future dynamic plugins must either compile as typed Rust extensions or submit declarative typed specs
that certify against trusted descriptors. Arbitrary erased graph submission, plugin-provided runners
without descriptor/version/capability evidence, plugin bypass of public-output/schema/side-effect
ledger contracts, and plugin-minted production capability tokens are forbidden.

For permanent side-effect ambiguity, v1 behavior is deliberately conservative: block the run, persist
typed ambiguity evidence, expose a stable redacted error, and require manual resolution or a future
explicit recovery workflow. Ambiguity is never auto-retried.

### Risks And Tradeoffs

This design has real costs:

- generic APIs can become complex
- Rust error messages may be harder for workflow authors
- compile times may increase
- object safety is harder before certified lowering
- dynamic plugin support must be deferred until it can be constrained by certification
- schema evolution must be designed deliberately
- old state/adapter/connector versions must remain replayable, which creates version-retention costs
- effect-specific traits require more up-front framework design
- stable id derivation becomes part of the public correctness contract
- compile-time guarantees apply mainly to Rust-authored APIs; persisted specs, future dynamic
  plugins, and registry/runtime resolution still require strong certification
- the type system cannot prove absence of arbitrary ambient IO in arbitrary Rust code, so capability
  typing must be paired with lint and crate-boundary policy
- derive and attribute macros can obscure the model if they hide the typed builder surface or emit
  poor diagnostics
- any future manual implementation escape hatch for value/config/output/descriptors can become a
  correctness hole unless it remains framework-owned, narrow, audited, and covered by compile-fail
  and runtime tests

These costs are acceptable because the alternative is worse: a reproducible execution platform whose
core semantic contract is enforced mostly by string names, JSON shape checks, runtime validation,
and code review.

The proposal should avoid encoding an entire DAG as nested generic types. That would be too rigid
for MFM's dynamic workflows. Typed handles are the better fit: they preserve practical compile-time
producer/consumer, lifecycle, effect, and scope guarantees while allowing deterministic dynamic
expansion and runtime scheduling.

### Minimal First Slice

The smallest proof of the architecture is the `typed-kernel-contract` milestone. It is intentionally
narrower than the full platform rewrite and narrower than the later `typed-certified-slice` runtime
gate.

Included crates and APIs:

1. kernel crate skeleton for ids, canonicalization, values, effects, capabilities, program,
   program-derive, spec, certify, events, and test support
2. `MfmValue`, `MfmConfig`, strong identity types, canonical hashing, schema descriptors, and
   derive-only no-float/no-secret enforcement for reference value/config/output types
3. typed handles, root seed specs, root public output binding, child-scope bridge session evidence,
   and value lineage evidence
4. `StateSpec`, registered state evidence, `StateInput`, `IntoStateInput`, `v1::InputBindingSpec`,
   `Operation`, registered operation evidence, `OperationOutput`, `PublicOutputs`, and
   `ScopeBuilder`
5. stable author keys, stable domain keys, derived scope/operation/node/cell identities, operation
   lineage, value lineage, and golden vectors
6. effect declarations and capability-set evidence for `Pure`, `ReadExternal`,
   `ManagedPlatformWrite`, and `ApplySideEffect`, including typed intent, idempotency input,
   submission, receipt, and confirmation descriptors
7. `v1::TypedExecutionSpec`, `SpecHash`, `RunStarted`, typed kernel event schemas,
   one-output-per-state cells, seed artifact evidence, and public-output evidence schemas
8. compile-pass and compile-fail fixtures for the reference API and every blocker class in
   `RFC_STATE_OPS_PROBLEM.md`

Excluded and deferred from this first slice:

- Postgres stream store and filesystem artifact store implementations
- the serial typed scheduler/executor
- replay/resume execution
- full side-effect crash matrix execution
- retention projection completeness and garbage-collection refusal
- public-output renderer runtime
- portfolio, EVM, and real proof backend migrations
- dynamic plugins
- broad state/operation attribute macros
- S3 or remote artifact stores
- parallel scheduling
- deletion of old APIs beyond proving the typed kernel has no dependency on them

Exit gate:

The reference program expands, certifies, and hashes deterministically without semantic JSON context
dataflow, generic IO, public erased DAG construction, or hand-authored dependency edges. The gate
must reject invalid topology, invalid interface wiring, invalid semantic transition, invalid data
shape, invalid data meaning, and invalid terminal shape.

CI evidence:

```text
nix run .#check
  crate-dag
  typed-boundary-firewall
  deterministic-expansion-lint
  typed-kernel-contract summary schema

nix run .#test
  typed-core-trybuild
  schema-canonicalization
  stable-id-determinism
  descriptor-registry-certification
  lowering-certification
  typed-store-events schema golden fixtures
  no-secret-no-float-derives
  typed-kernel-contract
```

If this slice works without context keys, generic IO, or hand-authored edges for semantic
correctness, the architecture is viable enough to implement the later `typed-certified-slice` and
then expand to portfolio and EVM workflows. If it still depends on those dynamic surfaces, it has
not solved the core problem.
