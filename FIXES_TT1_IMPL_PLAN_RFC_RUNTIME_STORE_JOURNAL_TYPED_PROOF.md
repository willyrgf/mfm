# TT1 implementation fixes for the Runtime, Journal, and Store proof path

Status: implementation-ready corrective plan for the tree at
`ea0ac115b8d82750e49211b58fb277067e735024`

Authority:

- [`RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md`](RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md) owns the
  approved architecture and persisted contracts.
- [`IMPL_PLAN_RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md`](IMPL_PLAN_RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md)
  remains the completed cutover plan.
- This document owns only the TT1 corrections discovered by the post-implementation review. Where
  it names a different private implementation mechanism, this document supersedes that mechanism,
  not the RFC contract.
- [`AGENTS.md`](AGENTS.md), [`docs/code-quality.md`](docs/code-quality.md), and
  [`docs/build-and-verification.md`](docs/build-and-verification.md) remain mandatory.

The submitted two-commit cutover is not to be rewritten, squashed, or supplemented with
compatibility code. Apply the fixes below as ordered, compile-green follow-up commits on top of the
submitted tree.

---

## 0. Decision and material uncertainties

The review found two proof-path correctness failures, several boundary-validation defects, and a
small number of missing regressions. Fix them at their existing owners:

- typed IDs own fixed digest algorithms;
- Values owns one descriptor-derived schema authority and one exact value qualification path;
- Runtime Match owns schema-driven extraction of exact retained payload bytes;
- Runtime assembly owns root and ABI association;
- domain planning owns declaration order;
- Journal and Store retain their existing distinct proof transitions;
- PostgreSQL owns baseline classification;
- the cutover scanner owns exact deleted-path absence.

The corrections deliberately add no new proof wrapper, registry, reducer, Journal record, Store
method, error variant, schema, table, migration, wire version, workspace member, or internal
dependency edge. The Runtime Match correction deletes the derive-generated visitor rather than
repairing a second projection authority.

### Material uncertainties

none.

The following choices are explicit and closed:

- `RunId` and `ArtifactId` are fixed to `Sha256JcsV1`; only `ContentDigest` accepts a caller-selected
  digest algorithm.
- Frame heads remain `content:sha256-v1:...`; this plan does not change Journal hashing.
- Generic `PersistedSchema` derives remain supported without a global cache. Correctness and one
  implementation path take precedence over a speculative cache; add a cache only after a measured
  future need and a separately reviewed per-concrete-type design.
- Match projects an external or adjacent newtype payload from its exact retained canonical JSON
  token, then invokes the already registered payload codec once. Match projection never serializes
  the typed selector or typed payload.
- Internally tagged, unit, named-field, and multi-field Match payloads remain unsupported because
  their arm shape does not provide the single embedded nominal payload contract required by
  association.
- The three Journal entry points `qualify`, `from_genesis`, and `extend_inserted` remain separate;
  they prove different cold/hot transitions and must not be collapsed.

## 1. Completion rule and hard simplicity budget

The TT1 work is complete only when all five commits in Section 2 exist in order and every commit:

1. leaves one coherent current design and passes its focused checks;
2. contains the owning regression for every behavior it changes;
3. deletes the superseded path in the same commit;
4. has a non-author architect `APPROVE` under Section 9; and
5. introduces no compatibility alias, fallback, legacy parser, dual behavior, or deferred cleanup.

The integrated tree must satisfy all of these budgets:

| Measure | Submitted tree | TT1 requirement |
| --- | ---: | ---: |
| Workspace members | 18 | exactly 18 |
| Normal internal MFM edges | 45 | no more than 45; expected exactly 45 |
| Workspace top-level public declarations/reexports | 179 | must not increase |
| Proof-path top-level public declarations/reexports | 134 | must decrease or stay lower; no new item |
| Proof-path source LOC | 11,573 | report again; production portion must decrease |
| Journal/Store/PostgreSQL non-test production LOC delta | 0 | no more than +1 in TT1-4 |
| Journal/Store/PostgreSQL logical public items | 18 | exactly 18 |

The previously reported `2,365` storage number counted Rust only. The correct submitted-tree total
raw entry-file footprint is `2,407`: 977 Journal Rust lines, 445 Store Rust lines, 943 PostgreSQL
Rust lines, and 42 migration SQL lines. That raw count includes inline `#[cfg(test)]` code. Report
it again in the final handoff, but assess the TT1-4 production limit after separating test-only
blocks. Do not move tests between files merely to game either number.

Required tests may add test LOC. Report production Rust, inline tests, and external tests
separately; do not claim simplification by moving tests between locations. Across production Rust,
the five commits must be net negative. The Match visitor deletion is expected to pay for the small
boundary checks elsewhere.

The only permitted manifest expansion is enabling serde_json's existing `raw_value` feature in
`mfm-runtime`. This is not a new dependency edge. Any other new dependency, feature, public item,
or error variant is an architect `BLOCK` unless the RFC is first amended.

## 2. Ordered commit sequence

| Commit | Exact subject | Primary owner | Expected production effect |
| --- | --- | --- | ---: |
| TT1-1 | `enforce foundational value invariants` | IDs, Values, derive | negative LOC; narrower API |
| TT1-2 | `close typed runtime projection and association gaps` | Program/Runtime | materially negative LOC |
| TT1-3 | `derive evm control indices from program order` | EVM/Portfolio | negative LOC |
| TT1-4 | `close journal and store boundary checks` | Journal/Store/PostgreSQL | approximately +1 production LOC |
| TT1-5 | `detect dangling deleted paths in cutover scan` | cutover scanner | neutral production LOC |

Do not split a row into preparation and cutover commits. Do not merge TT1-5 into a runtime/storage
commit: repository-absence enforcement is an independent owner and rollback unit. Documentation
corrections belong to the commit that proves the affected contract.

## 3. TT1-1 — enforce foundational value invariants

### 3.1 Objective

Make invalid fixed-algorithm identities unconstructible, remove the duplicate `MfmValue` schema-ID
authority, restore exact ContentRef grammar validation, and delete the generic-static schema cache
whose value leaks across monomorphizations.

Primary paths:

- `crates/kernel/ids/src/identity.rs`
- `crates/kernel/ids/src/lib.rs`
- `crates/kernel/ids/src/tests.rs`
- `crates/kernel/ids/README.md`
- `crates/kernel/values/src/lib.rs`
- `crates/kernel/values/src/persisted.rs`
- `crates/kernel/program-derive/src/lib.rs`
- `crates/kernel/program-derive/tests/derive_contract.rs`
- `crates/kernel/journal/tests/frame_contract.rs`
- the eight current `RunId::from_digest` call sites
- direct callers of the deleted `MfmValue::schema_id`

### 3.2 Specialize digest construction by identity category

The current generic constructor accepts an arbitrary algorithm for every digest-only category:

```rust
Identity::<RunIdKind>::from_digest(DigestAlgorithm::Sha256V1, digest)
```

That constructs a typed `RunId` which its own parser, Journal cold qualification, and PostgreSQL
constraint reject. Delete that public generic authority. Retain one private constructor:

```rust
impl<K> Identity<K>
where
    K: private::DigestOnlyCategory,
{
    pub(super) fn with_digest_algorithm(
        algorithm: DigestAlgorithm,
        digest: DigestBytes,
    ) -> Self {
        let raw = format!("{}:{algorithm}:{digest}", K::PREFIX);
        Self {
            raw,
            canonical_name: None,
            algorithm,
            digest,
            _kind: PhantomData,
        }
    }
}
```

Expose only these concrete signatures, generated through the existing category macros if that
keeps the expansion smaller:

```rust
impl Identity<RunIdKind> {
    /// Constructs a run id with the fixed `Sha256JcsV1` algorithm.
    pub fn from_digest(digest: DigestBytes) -> Self {
        Self::with_digest_algorithm(DigestAlgorithm::Sha256JcsV1, digest)
    }
}

impl Identity<ArtifactIdKind> {
    /// Constructs an artifact id with the fixed `Sha256JcsV1` algorithm.
    pub fn from_digest(digest: DigestBytes) -> Self {
        Self::with_digest_algorithm(DigestAlgorithm::Sha256JcsV1, digest)
    }
}

impl Identity<ContentDigestKind> {
    /// Constructs a content digest with one explicitly selected supported algorithm.
    pub fn from_digest(
        algorithm: DigestAlgorithm,
        digest: DigestBytes,
    ) -> Self {
        Self::with_digest_algorithm(algorithm, digest)
    }
}
```

Keep `ContentDigest` unrestricted because both `Sha256V1` and `Sha256JcsV1` are valid content
digest categories. Do not add `try_from_digest`, a checked wrapper, a second marker trait, or a
runtime algorithm branch to fixed categories.

Every generated concrete `pub fn from_digest` must carry category-specific rustdoc. The fixed
constructors document that they always use `Sha256JcsV1`; the `ContentDigest` constructor documents
caller selection among the supported algorithms. This is required by `#![warn(missing_docs)]` and
strict Clippy, not optional prose.

Update all current `RunId::from_digest` call sites to omit the algorithm. There is no current
`ArtifactId::from_digest` caller. Remove imports that become unused.

### 3.3 Make the ContentRef digest grammar exact

`StringGrammar::ContentDigest` is the grammar embedded by `SchemaShape::content_ref()` and its
documented spelling is `content:sha256-v1:<64 lowercase hex>`. General `ContentDigest::parse`
correctly accepts both algorithms, so constrain the grammar rather than the type:

```rust
StringGrammar::ContentDigest => ContentDigest::parse(value)
    .is_ok_and(|digest| digest.algorithm() == DigestAlgorithm::Sha256V1),
```

Do not introduce a new grammar variant or rename `content_digest`; either would reset schema
identities for no benefit. Do not restrict `ContentDigest::parse`.

### 3.4 Delete `MfmValue::schema_id`

Delete this overridable default method from `MfmValue`:

```rust
fn schema_id() -> Result<SchemaId>;
```

`PersistedSchema::schema_id` remains; it is a separate persisted-owner contract.

Delete every call to `MfmValue::schema_id`. Within a function that already obtains a descriptor,
derive its schema ID from that same descriptor. `canonicalize_mfm_value` must invoke
`schema_descriptor` exactly once. Independent owners such as Program's `nominal_contract_ref`
retain their own descriptor lookup; do not duplicate Program's nominal-reference policy inside
Runtime merely to share one call across owners.

Update at least:

- `canonicalize_mfm_value` in `mfm-values`;
- `GenericArgumentDescriptor::for_value` in `mfm-values`;
- `nominal_contract_ref` in `mfm-program`;
- `RuntimeAssemblyBuilder::register_value` in `mfm-runtime`; and
- the `EvmPhysicalTarget` schema contract test.

The canonical helper's required order is:

```rust
pub fn canonicalize_mfm_value<T: MfmValue>(
    value: &T,
) -> std::result::Result<(PlainCanonicalJsonBytes, ContentRef), ValueError> {
    let descriptor = T::schema_descriptor()?;
    let semantic_id = T::semantic_id()?;
    if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic_id) {
        return Err(ValueError::Descriptor(
            "value descriptor does not match its Rust owner".to_owned(),
        ));
    }
    let schema_id = descriptor.schema_id()?;

    let json = serde_json::to_string(value)
        .map_err(|_| ValueError::SchemaShapeMismatch)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| ValueError::SchemaShapeMismatch)?;
    if canonical.as_bytes().len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
        return Err(ValueError::Capacity);
    }
    descriptor
        .identity()
        .validate_canonical_value(canonical.as_bytes())?;

    let content_ref = ContentRef::new(
        schema_id,
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            canonical.digest_bytes(),
        ),
    )
    .map_err(|error| ValueError::Identity(error.to_string()))?;
    Ok((canonical, content_ref))
}
```

The size check intentionally precedes descriptor shape walking. An oversized value is `Capacity`
even if a later field would fail shape validation. This avoids another full parse/walk of an input
already known to exceed the object contract.

Update the error rustdoc to match that order:

```rust
/// Canonical value bytes exceeded the retained object ceiling before shape qualification.
Capacity,
```

Delete the direct `sha256_digest_bytes` import from this helper when it becomes unused.

`GenericArgumentDescriptor::for_value` must preserve the same owner agreement without another
trait method:

```rust
let descriptor = T::schema_descriptor()?;
let semantic_type_id = T::semantic_id()?;
if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic_type_id) {
    return Err(ValueError::Descriptor(
        "generic descriptor does not match its Rust owner".to_owned(),
    ));
}
Ok(Self {
    schema_id: descriptor.schema_id()?,
    semantic_type_id,
})
```

`nominal_contract_ref` and `register_value` must retain semantic-owner agreement but delete the
tautological comparison between `descriptor.schema_id()` and another call through `T`.

### 3.5 Delete the cross-monomorphization `PersistedSchema` cache

Rust function-local statics inside generic functions are shared across monomorphizations. The
generated `OnceLock<SchemaIdentity>` and `OnceLock<SchemaId>` therefore let the first
`Owner<A>` call poison `Owner<B>`.

Delete from the `PersistedSchema` derive expansion:

- generated inherent `__mfm_persisted_schema_identity`;
- both `OnceLock`s;
- generated `schema_shape`, `validate_canonical_bytes`, and `schema_id` overrides;
- the cached/prevalidated validation branch and its performance comment.

Emit only the identity and validation owned by the trait:

```rust
impl #impl_generics ::mfm_values::PersistedSchema
    for #ident #ty_generics #where_clause
{
    fn schema_identity(
    ) -> ::mfm_values::Result<::mfm_values::SchemaIdentity> {
        #identity_body
    }

    fn validate(&self) -> ::mfm_values::Result<()> {
        let identity =
            <Self as ::mfm_values::PersistedSchema>::schema_identity()?;
        ::mfm_values::validate_derived_persisted_owner(self, &identity)
    }
}
```

Delete `validate_derived_persisted_owner_prevalidated` and its reexport when no caller remains.
Update `PersistedSchema::schema_shape` rustdoc to describe its one default derivation, not a cache.

Keep generic derives supported and add a two-monomorphization regression. Do not add a `TypeId`
map, global registry, leaked allocation, per-type cache crate, or special rejection for generics.
There is no production generic owner and no measured performance problem; the trait defaults are
the smaller correct implementation.

### 3.6 Documentation corrections

Update IDs rustdoc and `crates/kernel/ids/README.md` to state:

- `RunId` and `ArtifactId` are fixed to `sha256-jcs-v1`;
- `ContentDigest` admits both supported algorithms; and
- `ContentRef` proves interpretation and exact-byte identity, while Journal separately qualifies
  retained frame-local bytes.

Delete the obsolete sentence claiming Journal uses the removed `ValueRef`.

### 3.7 Regression matrix

Prefer extensions to existing tests over new harnesses.

Required tests:

1. `RunId::from_digest(bytes)` and `ArtifactId::from_digest(bytes)` both:
   - report `Sha256JcsV1`;
   - round-trip through their parser; and
   - serialize to their fixed tag.
2. Compilation of every updated call site proves the illegal algorithm argument is gone.
3. A hostile retained Journal frame spelling `run:sha256-v1:...` remains rejected. Construct raw
   hostile bytes; do not reintroduce an invalid typed constructor for the test.
4. `ContentDigest::parse` accepts a JCS digest, while `StringGrammar::ContentDigest` rejects it and
   accepts the raw SHA-256 tag.
5. A manual counting `MfmValue` proves one call to `schema_descriptor` during
   `canonicalize_mfm_value`.
6. An oversized, otherwise shape-invalid value returns `ValueError::Capacity` before shape
   validation.
7. `Retained<A>` and `Retained<B>` generic `PersistedSchema` derives produce distinct correct
   identities and schema IDs regardless of call order.
8. State and capability static identity failures still map to `ProgramError::InvalidContract` at
   Program authoring. Runtime association coverage belongs to TT1-2.

### 3.8 Deletion and review gate

The architect must `BLOCK` if any of these remain:

- public algorithm selection for `RunId` or `ArtifactId`;
- `MfmValue::schema_id`;
- generic-static schema identity or schema-ID caches;
- `validate_derived_persisted_owner_prevalidated` without a remaining owner;
- a new digest grammar, cache registry, proof wrapper, or dependency; or
- descriptor value-shape validation before the fixed 8 MiB check in the shared qualifier.

Expected production result: approximately 35–65 LOC deleted, at least one hidden/public helper
deleted, no public name added, and no schema or wire identity changed.

Focused verification:

```bash
nix develop -c cargo test \
  -p mfm-ids -p mfm-canonical -p mfm-values \
  -p mfm-program-derive --all-targets

nix develop -c cargo test \
  -p mfm-program -p mfm-journal -p mfm-runtime -p mfm-evm \
  --all-targets

nix develop -c cargo check --workspace --all-targets
git diff --check
```

Do not run the composed CI gate yet.

## 4. TT1-2 — close typed Runtime projection and association gaps

### 4.1 Objective

Fix the high-severity hot/cold proof failure by deleting the typed Match visitor. A qualified
selector already contains the exact canonical bytes and Runtime already has a pre-associated
closed-sum descriptor plus payload codecs. Those are the only projection authorities needed.

This commit also closes Runtime's missing root-codec association. Its ABI, Never, recovery, and
hostile-ingress tests are omitted regressions from the same frozen Runtime/Program contract, not
new product behavior.

Primary paths:

- `crates/kernel/values/src/lib.rs`
- `crates/kernel/program-derive/src/lib.rs`
- `crates/kernel/program-derive/tests/derive_contract.rs`
- `crates/kernel/runtime/Cargo.toml`
- `crates/kernel/runtime/src/assembly.rs`
- `crates/kernel/runtime/src/assembly/tests.rs` — required private projection regressions; add no
  test-only public getter or visibility expansion; wire it from `assembly.rs` with
  `#[cfg(test)] mod tests;`
- `crates/kernel/runtime/tests/runtime_contract.rs`
- `crates/kernel/runtime/README.md`
- hostile Program tests in `crates/kernel/program/tests/contracts.rs`

### 4.2 Delete the second Match authority

Delete from `mfm-values`:

```text
MfmValue::__MFM_MATCH_PROJECTION_SUPPORTED
MfmValue::__mfm_visit_match_payload
MatchPayloadVisitor
```

Delete from `mfm-program-derive`:

```text
match_projection_impl
enum_match_projection_tokens
boxed_inline_value_depth
is_inline_value_type
all generated Match visitor hooks
```

Delete from Runtime:

```text
MatchProjector
ValueCodec::match_projection_supported
ValueCodec::project_match
project_typed
RuntimeMatchVisitor
impl MatchPayloadVisitor for RuntimeMatchVisitor
the Match-local qualify_hot(payload) path
```

Remove all now-unused imports and hook-only tests. Do not replace them with another public visitor,
`ProjectedValue`, `MatchPayloadProof`, bytes-only `QualifiedValue` variant, second decoder, or Match
Journal record.

### 4.3 Carry only schema-owned Match projection data

Enable `RawValue` on Runtime's existing serde_json dependency:

```toml
serde_json = { workspace = true, features = ["raw_value"] }
```

Reduce the private projection representation to:

```rust
pub(crate) struct MatchProjection {
    selector_contract_ref: ContentRef,
    tagging: EnumTagging,
    variants: Vec<VariantProjection>,
}

struct VariantProjection {
    tag: String,
    entry_index: u16,
    codec: Arc<ValueCodec>,
}
```

`ExecutableProgram` already retains the immutable `AssemblyInner`, so Match does not retain a
selector codec only to retain a function pointer. The selector's complete `QualifiedValue` enters
projection and its typed member is dropped after the exact raw payload is qualified.

### 4.4 Extract the exact canonical payload token

Use `serde_json::value::RawValue` to borrow the selected nested token. Do not build
`serde_json::Value` and do not serialize it again.

The private parser should have this behavior:

```rust
use serde_json::value::RawValue;

fn selected_match_payload<'a>(
    tagging: &EnumTagging,
    selector: &'a PlainCanonicalJsonBytes,
) -> Result<(String, &'a [u8])> {
    let fields: BTreeMap<String, &'a RawValue> =
        serde_json::from_slice(selector.as_bytes())
            .map_err(|_| RuntimeError::Internal)?;

    match tagging {
        EnumTagging::External if fields.len() == 1 => {
            let (tag, payload) = fields
                .into_iter()
                .next()
                .ok_or(RuntimeError::Internal)?;
            Ok((tag, payload.get().as_bytes()))
        }
        EnumTagging::Adjacent { tag, content } if fields.len() == 2 => {
            let selected = serde_json::from_str::<String>(
                fields.get(tag).ok_or(RuntimeError::Internal)?.get(),
            )
            .map_err(|_| RuntimeError::Internal)?;
            let payload = fields
                .get(content)
                .ok_or(RuntimeError::Internal)?;
            Ok((selected, payload.get().as_bytes()))
        }
        EnumTagging::External
        | EnumTagging::Adjacent { .. }
        | EnumTagging::Internal { .. } => Err(RuntimeError::Internal),
    }
}
```

The selector codec has already proved canonical JSON, exact schema shape, field count, and closed
tag membership. The local field-count checks keep trusted projection drift fail-closed.

Project through the existing payload codec:

```rust
impl MatchProjection {
    pub(crate) fn project(
        &self,
        selector: QualifiedValue,
    ) -> Result<(u16, QualifiedValue)> {
        if selector.contract_ref != self.selector_contract_ref {
            return Err(RuntimeError::Internal);
        }

        let (selected_tag, payload_bytes) =
            selected_match_payload(&self.tagging, &selector.canonical)?;
        let variant = self
            .variants
            .iter()
            .find(|variant| variant.tag == selected_tag)
            .ok_or(RuntimeError::Internal)?;

        let value_ref = ContentRef::new(
            variant.codec.contract_ref.schema_id().clone(),
            raw_content_digest(payload_bytes),
        )
        .map_err(|_| RuntimeError::Internal)?;
        let payload = variant
            .codec
            .qualify(&value_ref, payload_bytes)
            .map_err(|_| RuntimeError::Internal)?;
        Ok((variant.entry_index, payload))
    }
}
```

This one qualifier retains the exact supplied bytes, validates their exact schema, verifies their
content reference, and decodes the registered Rust payload type. Capacity cannot arise from a
nested payload whose already-qualified selector is within the same object bound, so delete the old
Match-local `ValueError::Capacity` mapping.

This deliberately decodes the newly selected nested payload once through the selected payload
codec on both hot and cold paths. It supersedes the old implementation-plan mechanism that moved
an already-decoded hot payload. It does not change ordinary State-to-State hot handoff, add another
decoder, or weaken the RFC's single-qualification/no-reserialization rule.

### 4.5 Associate ordinary, boxed, and generic payload contracts

At assembly time, resolve one standalone payload descriptor after unwrapping the existing
wire-transparent tuple/Box representation. Accept:

- `SchemaShape::InlineValue`; and
- exact `SchemaShape::Generic` with constructor `mfm/generic-value` and exactly one argument.

Return all three facts needed to compare the embedded payload claim with the registered codec:

```rust
fn match_payload_descriptor(
    shape: &SchemaShape,
) -> Option<(&SchemaId, &SemanticTypeId, &SchemaShape)> {
    let payload = match shape {
        SchemaShape::Tuple(elements) if elements.len() == 1 => &elements[0],
        other => other,
    };

    match payload {
        SchemaShape::InlineValue {
            schema_id,
            semantic_type_id,
            serialized_shape,
        } => Some((schema_id, semantic_type_id, serialized_shape.as_ref())),
        SchemaShape::Generic {
            constructor,
            arguments,
            serialized_shape,
        } if constructor == "mfm/generic-value" => {
            let [argument] = arguments.as_slice() else {
                return None;
            };
            Some((
                &argument.schema_id,
                &argument.semantic_type_id,
                serialized_shape.as_ref(),
            ))
        }
        _ => None,
    }
}
```

`associate_match` must:

1. require an external or adjacent enum descriptor;
2. require exact Program-arm/descriptor tag equality and exhaustiveness;
3. derive the nominal payload contract from the embedded schema ID;
4. resolve the target declaration and require a State whose input contract is that payload
   contract;
5. resolve the registered payload codec;
6. require embedded schema ID, semantic ID, and serialized shape to agree with the codec; and
7. store the descriptor tagging, exact tag, target index, and codec in `MatchProjection`.

After resolving the codec, freeze the semantic/shape comparison explicitly:

```rust
let codec_shape = codec
    .descriptor
    .identity()
    .canonical_json_shape()
    .map_err(|_| RuntimeError::IncompatibleAssembly)?;

if &codec.semantic_id != semantic_id || codec_shape != serialized_shape {
    return Err(RuntimeError::IncompatibleAssembly);
}
```

Lookup by the derived `ContentRef` already proves the schema ID. Do not replace the semantic/shape
comparison with another contract lookup.

Reject internal tagging, unit variants, named-field variants, multi-field tuple variants, malformed
generic constructors/arity, and all non-enum selectors before Store access in `start`. Box and
nested Box remain supported because their wire representation is the payload token itself.

The existing manual-selector test should become a successful association/execution case when its
registered closed-sum descriptor is complete. Derive ownership is no longer a hidden prerequisite;
the registered schema is the single authority.

### 4.6 Resolve both Program roots during association

Immediately after resolving the admitted-context codec, prove both root codecs exist:

```rust
for contract_ref in [
    program.root_success_contract_ref(),
    program.root_failure_contract_ref(),
] {
    self.inner
        .values
        .get(contract_ref)
        .ok_or(RuntimeError::IncompatibleAssembly)?;
}
```

No root codec needs to be retained as another `ExecutableProgram` field; terminal fold already
compares the actual qualified value contract with the Program root. This check only closes the
otherwise-unreferenced root registration gap.

`start` must reject a missing root codec before any Store call. `resume` and `read` necessarily load
before decoding/association, but still reject before adapter/provider or append IO.

### 4.7 Runtime and Program regression matrix

Keep fixtures table-driven and reuse builders; do not add one large program per negative.

#### Exact Match regressions

Add a payload with asymmetric Serde behavior: its serialized wire retains `"MiXeD"`, while its
manual `Deserialize` normalizes the typed field to `"mixed"`.

For generic external and adjacent selectors, assert on both hot and cold projection:

```rust
let hot_selector = qualify_hot(selector).expect("hot selector");
let selector_ref = hot_selector.value_ref.clone();
let selector_bytes = hot_selector.canonical.clone();
let cold_selector = selector_codec.qualify(
    &selector_ref,
    selector_bytes.as_bytes(),
)
.expect("cold selector");

let (hot_entry, hot_payload) = projection
    .project(hot_selector)
    .expect("hot payload");
let (cold_entry, cold_payload) = projection
    .project(cold_selector)
    .expect("cold payload");
```

Then assert:

```rust
assert_eq!(hot_entry, expected_entry);
assert_eq!(hot_entry, cold_entry);
assert_eq!(hot_payload.contract_ref, cold_payload.contract_ref);
assert_eq!(hot_payload.value_ref, cold_payload.value_ref);
assert_eq!(hot_payload.canonical, cold_payload.canonical);
assert_eq!(hot_payload.canonical.as_bytes(), br#"{"value":"MiXeD"}"#);
```

Also assert both typed payloads contain the normalized value. The retained bytes/ref must come from
the nested selector token; the typed value must come from one codec decode. This fails the current
decode/re-serialize projection.

Retain existing end-to-end hot/cold tests for both arms and nested Box. Add or retain pre-Store
association rejection for:

- ordinary non-enum selectors;
- internally tagged selectors;
- unit, named-field, and multi-field variants;
- missing, unknown, or non-exhaustive tags;
- wrong target payload contract;
- wrong generic constructor or argument count; and
- embedded payload semantic identity or serialized shape inconsistent with its codec.

Retained reordered and duplicate tags belong to Program hostile-decode tests because the public
`MatchDeclaration::new` sorts and deduplicates authored arms. Do not add a private constructor or a
test-only typed-Program bypass merely to manufacture reordered Runtime input.

#### Assembly regressions

Reuse one small fixture family. Table-drive only cases with one homogeneous setup and keep
monomorphic generic registration assertions explicit; do not build an enum/macro test framework
merely to force all cases into one table. Prove:

- the same capability contract ID cannot be registered for a different Rust capability type or a
  different Intent/Evidence ABI;
- the same State implementation ID cannot be registered once as Pure and once as Read;
- the same State implementation ID cannot be registered against two capability signatures;
- a Read State with `Failure = Never` registers, associates, and executes successfully;
- State/capability static ID errors map from `state_implementation_ref` and
  `capability_contract_ref` to `ProgramError::InvalidContract`, while builder registration maps
  them to `RuntimeError::IncompatibleAssembly`;
- association of a missing persisted implementation/capability reference is separately
  `RuntimeError::IncompatibleAssembly` before Store/provider/append IO; association does not rerun
  the static ID functions; and
- a Program with reachable States using `Failure = Never` but an otherwise-unreferenced root
  failure contract is rejected until that root value is explicitly registered.

The missing-root `start` test must use a counting Store and prove zero loads and zero appends.

#### Recovery regression

Add a one-shot Store double that:

1. delegates genesis insertion;
2. returns `StoreError::Indeterminate` for the first conclusion without inserting it; and
3. delegates the later retry.

Use a counting Pure State. Prove:

- the first invocation returns `RuntimeError::Indeterminate`;
- cold `read` still observes the genesis head and `Runnable`;
- `resume` reevaluates Pure exactly once more;
- the conclusion then commits at sequence 2; and
- the final hot/cold views agree.

Do not duplicate the already-present tests that an earlier `RunView` remains an immutable snapshot
after a conclusion commits indeterminately, that authoring Config/targets can be dropped before
cold App read, or that child-success/handler-success rejoins agree hot and cold. Preserve those
tests unchanged.

#### Hostile Program ingress

Extend Program decode tests with table-driven canonical mutations containing each retired inline
field: `execution_binding`, `physical_target_ref`, and `adapter_implementation_ref`. Every mutation
must fail `Program::decode_canonical` as `ProgramError::Canonical`. These are hostile-wire
regressions, not compatibility readers.

### 4.8 Documentation, deletion, and review gate

Add one Runtime README sentence:

> The Match projection selects and qualifies the exact nested canonical payload bytes through the
> registered payload codec; it never serializes a typed selector or payload.

The architect must trace both hot and cold Match projection from selector bytes to target input and
`BLOCK` if:

- any typed Match visitor/hook survives;
- any Match path invokes `canonicalize_mfm_value` on the payload;
- any projection uses `serde_json::Value` plus serialization;
- a second qualifier, decoder trait, payload wrapper, or Journal record appears;
- generic descriptor identity is accepted without schema, semantic, and serialized-shape agreement;
- root codecs are looked up during execution instead of once during association; or
- a missing assembly regression is replaced only by an absence scan.

Expected production result: roughly 80–120 LOC deleted, `MatchPayloadVisitor` and two hidden
`MfmValue` hook members deleted, zero public additions, and zero new dependency edges.

Focused verification:

```bash
nix develop -c cargo test \
  -p mfm-values -p mfm-program-derive -p mfm-program -p mfm-runtime \
  --all-targets

nix run .#run -- --task capacity-runtime
git diff --check
```

Do not run the composed CI gate yet.

## 5. TT1-3 — derive EVM control indices from Program order

### 5.1 Objective

Remove the duplicate caller-asserted fragment start and ignored per-State indices. The declaration
vector position is the only control identity; EVM must derive its first inserted index from the
vector it mutates.

Primary paths:

- `crates/domains/evm/src/lib.rs`
- the existing `#[cfg(test)]` module in `crates/domains/evm/src/lib.rs`
- `crates/domains/portfolio/src/lib.rs`
- `crates/domains/portfolio/tests/planning_contract.rs`
- `crates/app/tests/portfolio_runtime.rs`
- `crates/app/Cargo.toml`
- relevant App/Portfolio rustdoc

### 5.2 Remove caller-supplied start authority

Change the cross-domain helper from:

```rust
pub fn append_balance_fragment<K: MfmValueTrait>(
    declarations: &mut Vec<Declaration>,
    start_index: u16,
    source_count: usize,
    target: &EvmPhysicalTarget,
    failure_next_index: u16,
    completion_next_index: Option<u16>,
) -> Result<(), EvmDomainError>
```

to:

```rust
pub fn append_balance_fragment<K: MfmValueTrait>(
    declarations: &mut Vec<Declaration>,
    source_count: usize,
    target: &EvmPhysicalTarget,
    failure_next_index: u16,
    completion_next_index: Option<u16>,
) -> Result<(), EvmDomainError> {
    let start_index =
        u16::try_from(declarations.len()).map_err(|_| EvmDomainError::Program)?;
    // Existing checked source-count and forward-index arithmetic follows.
}
```

Retain checked `u16` arithmetic and the existing 1..=64 source bound. Do not infer positions from a
separate counter, add an address type, return an index proof, or defer mismatches to Runtime.

Update `append_balance_fragment` rustdoc to say that the fragment start is derived from the current
declaration-vector length and that callers supply only the external failure/completion routes.
After removing the parameter, delete obsolete `#[allow(clippy::too_many_arguments)]` attributes
from the public helper and private `read_state` if their signatures no longer trigger the lint.

Delete the unused `_index` parameter from private `pure_state` and `read_state`, then delete every
corresponding call argument. Those helpers construct declarations; they do not own control
placement.

### 5.3 Simplify Portfolio layout

Delete `CollectionLayout.fragment`. Portfolio still needs `enter`, `resume`, and `mapper` because
earlier forward edges target future declarations, but the fragment itself begins immediately after
the collection-entry State.

Calculate layout without a separately stored fragment:

```rust
let enter = cursor;
let fragment_len = source_count_u16
    .checked_mul(8)
    .and_then(|count| count.checked_add(1))
    .ok_or(PortfolioError::Program)?;
let resume = enter
    .checked_add(1)
    .and_then(|fragment| fragment.checked_add(fragment_len))
    .ok_or(PortfolioError::Program)?;
```

When authoring the entry State, its success edge is the immediately following index. After pushing
that State, call `append_balance_fragment`; the EVM helper derives the same position from
`declarations.len()`.

Do not make Portfolio recalculate or pass the EVM fragment's internal eight-State positions. The
EVM helper continues to own that reusable unrolling.

### 5.4 Regression and documentation

Add a focused EVM authoring test with preexisting declarations. Call the helper without a start
index and prove all fragment-internal State success edges, Match arm targets, the cross-source
transition, and the consolidate position derive from the actual prefilled vector length. The
caller-provided external failure/completion successors must remain exact. A malformed external
fragment start can no longer be expressed.

Retain the current two-source mixed native/token hot/cold test, second-source failure mapping,
64-source capacity success, and 65-source rejection. Do not rewrite the unrolling algorithm again.
Record a representative Portfolio Program's canonical bytes and ContentRef before/after and require
identity: removing duplicate authoring authority must not alter the valid emitted Program.

Replace stale tenant language:

- `crates/app/Cargo.toml` description becomes a Portfolio Runtime facade, not a fixed-tenant
  facade;
- `PortfolioSnapshotSelector` rustdoc describes selecting the admitted Portfolio target, not a
  fixed tenant.

No tenant code or API is added.

### 5.5 Deletion and review gate

The domain architect must `BLOCK` if:

- `append_balance_fragment` still accepts a caller-supplied fragment start or fragment-internal
  index/address assertion;
- private State constructors retain ignored index arguments;
- Portfolio owns any of the fragment's internal State positions;
- a new layout wrapper or validation pass replaces the deleted argument;
- Program, Runtime, Journal, Store, or App production Rust/API or any schema changes for this
  domain-local correction; App tests and package-description metadata are the named exceptions; or
- the multi-source public behavior or the three frozen public Portfolio result fixtures change.

Expected production result: negative LOC, no public item added, one public function parameter
removed, no Program bytes changed except where the prior invalid caller authority could have
mis-authored a graph.

`failure_next_index` and `completion_next_index` remain Portfolio-owned external continuation
routes; they are not fragment-start authority and must not be deleted or derived inside EVM.

Focused verification:

```bash
nix develop -c cargo test \
  -p mfm-evm -p mfm-portfolio -p mfm-evm-live -p mfm-app \
  --all-targets

nix run .#run -- --task capacity-app
git diff --check
```

Do not run the composed CI gate yet.

## 6. TT1-4 — close Journal and Store boundary checks

### 6.1 Objective

Reject an empty physical Memory frame, classify a wrong PostgreSQL marker column type as
`Incompatible`, close exact Journal boundary coverage, and correct two stale wire literals. Keep
all public Store/Journal APIs, hashing, SQL, and transaction semantics unchanged.

Primary paths:

- `crates/kernel/journal/src/lib.rs` tests only where private construction access is necessary
- `crates/kernel/journal/tests/frame_contract.rs`
- `crates/kernel/store/src/lib.rs`
- `crates/kernel/store/src/tests.rs`
- `crates/storages/postgres/src/lib.rs`
- `crates/storages/postgres/src/tests.rs`
- `docs/design.md`
- `docs/persisted-public-surfaces.md`

### 6.2 Memory rejects an empty retained frame

Inside `validate_physical`, extend the existing per-frame predicate:

```rust
for frame in frames {
    if frame.bytes.is_empty()
        || frame.bytes.len() > MAX_FRAME_BYTES
        || frame_head_digest(&frame.bytes) != frame.head_digest
    {
        return Err(StoreError::CorruptPhysicalState);
    }
    // Existing checked total accumulation remains unchanged.
}
```

Do not add a separate `total_bytes == 0` branch. Once the prefix and each frame are nonempty, the
existing checked sum and exact head-total comparison prove a positive total. Do not change
`StoredRunBytes`, `StoreError`, append eligibility, or Memory's dense frame representation.

Add the regression to `private_empty_and_corrupt_shells_are_classified`: install a `MemoryRun`
containing one empty `StoredFrame`, `frame_head_digest(&[])`, sequence 1, and total 0; call
`MemoryStore::load_run` and require `StoreError::CorruptPhysicalState`. Do not test only
`validate_physical`. Using the correct digest ensures the test reaches the emptiness check rather
than a digest mismatch.

An empty private Memory synchronization shell with no head and no frames remains `None`/absent. A
frame without a head, a head without a complete nonempty prefix, or an empty retained frame is
corrupt.

### 6.3 PostgreSQL proves column shape before typed marker decoding

Move the existing typed marker query; do not add another query or classifier.

Required order inside `verify_connection`:

1. durability settings;
2. exact MFM relation set;
3. exact column names, types, nullability, and collation;
4. only then `SELECT schema_contract ...` into `Vec<String>` and require the exact one row;
5. indexes, constraints, and remaining readiness checks.

The moved block remains:

```rust
let markers: Vec<String> =
    sqlx::query_scalar(
        "SELECT schema_contract FROM public.mfm_store_schema ORDER BY 1",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| {
        if is_undefined_schema_object(&error) {
            GateError::Incompatible
        } else {
            GateError::Unavailable
        }
    })?;
if markers != [SCHEMA_CONTRACT] {
    return Err(GateError::Incompatible);
}
```

The catalog shape check guarantees `schema_contract` is non-null `TEXT COLLATE "C"` before SQLx
decodes it. A `BIGINT` marker column therefore returns `StoreOpenError::Incompatible`, not a decode
failure mapped to `Unavailable`.

Add a managed database regression that replaces the marker table with
`schema_contract BIGINT PRIMARY KEY`, inserts `1`, and asserts the checked connect is
`Incompatible`. Keep reconnect behavior unchanged: a connection that becomes incompatible after
Runtime construction is rejected and the Store operation reports `Unavailable`.

Do not edit the migration, schema marker, errors, table set, SQL constraints, or public constructor.

### 6.4 Close exact Journal capacity and recurrence coverage

Tests only; do not change the three public proof transitions or add a test-only public constructor.

1. Refactor, rather than duplicate, the existing 25 MiB three-maximum-object fixture. In one private
   Journal test, call the existing private constructor with:
   - `run_sequence = MAX_RUN_FRAMES`;
   - a present `Sha256V1` predecessor;
   - the fused Read record carrying all three references;
   - three exact 8 MiB canonical objects;
   - maximum-width 512-byte schema identities; and
   - distinct references ordered by the existing frame-local closure logic.

   Require sequence 65,536 to encode, retain the existing `MAX_FRAME_BYTES` assertion, and measure
   `canonical.len() - payload.len()` at or below `MAX_FRAME_NON_PAYLOAD_ENVELOPE`. Build a private
   `JournalHistory` from that frame and require the ordinary next-successor path to return
   `JournalError::Capacity`, proving sequence 65,537 without appending 65,536 frames. Remove the old
   sequence-2 three-maximum-object fixture when moving this coverage; never allocate two equivalent
   25 MiB fixtures in one test run.
2. Keep a separate exact one-byte-over object assertion. Use
   `MAX_RUN_OBJECT_CANONICAL_BYTES - 1` inner string bytes so the surrounding JSON quotes yield
   exactly `MAX_RUN_OBJECT_CANONICAL_BYTES + 1`, then require `JournalError::Capacity`.
3. Add a hot/cold recurrence test where a conclusion reuses a ContentRef and identical bytes from
   genesis. `extend_inserted` and later `qualify` must both accept the repeated cross-frame object;
   frame-local closure remains exact without a global object table.
4. Preserve TT1-1's hostile raw `run:sha256-v1:...` Journal-qualification regression; do not add a
   duplicate fixture in this commit.

Retain the distinct semantics:

- `StoredRunBytes::new` checks a nonempty 1..=`MAX_RUN_FRAMES` raw transfer and raw size ceilings;
- `JournalHistory::qualify` consumes an untrusted cold prefix;
- `JournalHistory::from_genesis` admits one sealed hot genesis; and
- `JournalHistory::extend_inserted` extends only an acknowledged hot successor.

### 6.5 Correct the authoritative wire spelling

Replace `mfm-run-frame@1` with the code/RFC identity `mfm.run.frame.v1` in exactly:

- `docs/design.md`; and
- `docs/persisted-public-surfaces.md`.

This is a documentation correction, not a wire or golden change.

### 6.6 Deletion and review gate

The Journal/Store boundary architect must review Journal, Memory, and PostgreSQL together and
`BLOCK` if:

- Memory can return `Some` for an empty retained frame;
- PostgreSQL performs typed marker decoding before exact column-shape proof;
- the wrong-column regression can return `Unavailable`;
- a new public validator, error, Store method, schema object, or compatibility path appears;
- Journal constructors are collapsed or Store starts parsing Journal semantics;
- frame-head hashing/tagging changes; or
- non-test Journal/Store/PostgreSQL/SQL production LOC increases by more than the single required
  Memory predicate.

Expected production result: approximately +1 Store line, a moved PostgreSQL block with zero net
production LOC, zero public/schema/dependency change, and focused regression additions only.

Focused verification:

```bash
nix develop -c cargo test -p mfm-journal --all-targets
nix develop -c cargo test -p mfm-store --all-targets
nix develop -c cargo test -p mfm-storage-postgres --lib
nix run .#run -- --task postgres-test
nix run .#run -- --task capacity-store
git diff --check
```

Do not run the composed CI gate yet.

## 7. TT1-5 — detect dangling deleted paths in the cutover scan

### 7.1 Objective

Make an exact deleted-path rule reject every directory entry, including a dangling symlink. Bash
`-e` alone returns false for a dangling symlink.

Primary path:

- `scripts/check-cutover-manifest.sh`

The manifest ledger, coverage rows, fingerprints, and rule count do not change.

### 7.2 Exact scanner correction

Change both the scan predicate and canary cleanup predicate:

```bash
if [[ -e "$repository_root/$pattern" || -L "$repository_root/$pattern" ]]; then
```

```bash
if [[ -n "$canary_path" ]] && [[ -e "$canary_path" || -L "$canary_path" ]]; then
```

Reuse the existing path canary instead of adding a new canary class. Create a dangling symlink at
one already forbidden path, for example:

```bash
path)
  canary_path="$repository_root/docs/btc-rpc-routing.md"
  ln -s "$canary_root/missing-target" "$canary_path"
  ;;
```

The canary must prove the scanner fails, cleanup removes the dangling link, and a subsequent clean
scan passes. Do not add a permanent stale fixture or scan ignored local surfaces such as `.hermes/`,
`runtime.local.toml`, or `setup.local.toml`.

### 7.3 Review gate and verification

The closure architect must `BLOCK` if:

- an exact forbidden path can survive as any symlink;
- canary cleanup leaves a dangling path;
- the manifest fingerprint/ruleset/coverage inventory changes for this correction;
- a broad repository-root scan replaces the existing supported-root scopes; or
- archival RFC/plan files or ignored local state become current scanner inputs.

Expected production result: zero net shell LOC, no manifest churn, and no task-graph change.

Focused verification:

```bash
nix run .#run -- --task negative-scan
git diff --check
```

Do not run `model-check` or `nix flake check --no-build` unless the implementation unexpectedly
changes `nixfied.nix`, the flake, or an output. Such a change is outside this commit's target and
requires architect review.

## 8. Cross-commit deletion ledger

The following superseded items must be absent after their owning commit:

| Owner | Delete |
| --- | --- |
| TT1-1 IDs | generic public algorithm-selecting digest constructor for fixed ID kinds |
| TT1-1 Values | `MfmValue::schema_id`; duplicate descriptor lookup/comparison path |
| TT1-1 derive/Values | both generic-static `OnceLock`s, generated cached overrides, prevalidated helper |
| TT1-2 Values | `MatchPayloadVisitor` and both hidden `MfmValue` Match hooks |
| TT1-2 derive | visitor code generation and hook-shape helper functions |
| TT1-2 Runtime | `MatchProjector`, visitor implementation, Match `qualify_hot` path, selector function pointer |
| TT1-3 EVM | caller `start_index`, ignored `_index` parameters, `CollectionLayout.fragment` |
| TT1-4 docs | both stale `mfm-run-frame@1` literals |
| TT1-5 scanner | path/cleanup checks that rely on `-e` alone |

Search by concept as well as literal. A renamed visitor, cached identity map, caller-provided
address, payload wrapper, or alternate Match qualification path fails the deletion gate even if the
old spelling is gone.

Do not add compile-fail tests whose only purpose is to repeat the deletion scanner. Use compile-fail
only when proving a surviving construction boundary, such as the narrowed fixed-ID constructor.

## 9. Architect review protocol

Keep review proportional: one non-author architect for each commit's owner boundary and one final
non-author cumulative reviewer. Do not recreate the fourteen-chunk process from the original
cutover.

Each reviewer receives:

- this fixes plan and the relevant RFC/architecture sections;
- the path-scoped diff and cumulative diff;
- the exact focused command output;
- before/after production and test LOC for the touched paths;
- public/dependency deltas; and
- the commit's deletion checklist.

Each decision uses this exact form in the PR or engineer handoff, not a permanent repository log:

```text
Commit/chunk:
Author revision:
Architect reviewer:
Decision: APPROVE | BLOCK
Correctness findings:
Simplicity / net-LOC findings:
Dexterity findings:
Deletion evidence:
Verification reviewed:
Material uncertainties: none | <blocking item>
```

The reviewer must explicitly answer:

1. Is the invariant enforced once at its strongest owner?
2. Did the patch delete the superseded authority and tests for that authority?
3. Is there a smaller implementation with fewer public concepts, branches, allocations, or future
   change sites?
4. Did production LOC decrease for TT1-1 through TT1-3, and is any increase in TT1-4/TT1-5 the
   minimum required validation?
5. Can a new ordinary MFM value, Pure State, Read State, Match selector, EVM source, Store backend,
   or deleted-path rule be added without editing unrelated owners?

Any `BLOCK` must name the concrete counterexample or unnecessary concept and be rereviewed after
the correction. Do not call the work complete merely because tests pass.

The submitted handoff asserted that every original cutover chunk and the final cumulative review
were approved, but those records are not present in the two implementation commits. Attach the
existing original C1/C2 review records required by Section 9.5 of the original plan, including
reviewer, revision, findings, deletion evidence, dexterity, and verification. Do not fabricate or
reconstruct approvals from a passing CI log. If a required record cannot be produced, state that
process gap explicitly and obtain a fresh non-author review of the affected original boundary
before repeating the claim that all chunk architects approved. Keep these records in the PR or
engineer handoff, not as a permanent Runtime/Store log.

The final cumulative architect must trace:

- typed RunId construction -> hot Journal admission -> cold Journal qualification -> PostgreSQL
  grammar;
- hot and cold asymmetric-Serde Match selection -> exact nested bytes/ref -> target State input;
- Program root association and capability/State ABI collision rejection;
- EVM multi-source lowering from vector position through Program validation;
- empty/corrupt Memory load and wrong-shape PostgreSQL connect classification; and
- deleted-path canary creation, rejection, cleanup, and clean rerun.

## 10. Integrated verification and evidence

### 10.1 During implementation

Use the focused commands in each commit section. Start with a single failing regression filter when
iterating, then run the listed affected packages before architect review. All Cargo/Rust commands
run inside the default Nix development shell.

Do not run `.#check`, `.#test`, or `.#test-db` immediately before `.#ci`; final CI already composes
them. Do not run broad gates merely because a commit is about to be created.

### 10.2 Final candidate

After all five commit reviews approve and the focused checks above have passed, do not immediately
repeat gates already composed by CI. Run only:

```bash
git diff --check
nix run .#ci
```

Record the revision and wall time. If a subsequent edit changes code, tests, docs, manifests,
scripts, or workflows, rerun the scope-selected checks for that edit and treat the prior CI result
as stale.

### 10.3 Reproduce complexity evidence

Use the same commands and roots as Section 9.1 of the original implementation plan. Enter one
interactive default `nix develop` shell first so Cargo, jq, and ripgrep all come from the pinned
environment, then at minimum record:

```bash
metrics_dir=$(mktemp -d)
cargo metadata --format-version 1 --no-deps \
  > "$metrics_dir/metadata.json"

jq -r '.workspace_members | length' "$metrics_dir/metadata.json"

jq -r '
  [.packages[] as $package
   | $package.dependencies[]
   | select(.source == null and .kind == null)
   | select(.name | startswith("mfm-"))
   | [$package.name, .name]]
  | unique | sort | .[] | @tsv
' "$metrics_dir/metadata.json" \
  | tee "$metrics_dir/normal-edges.tsv"
wc -l < "$metrics_dir/normal-edges.tsv"

git ls-files \
  ':(glob)crates/**/src/**/*.rs' \
  ':(glob)bin/**/src/**/*.rs' \
  | xargs wc -l

git ls-files \
  ':(glob)crates/**/tests/**/*.rs' \
  ':(glob)bin/**/tests/**/*.rs' \
  | xargs wc -l
```

Also reproduce the workspace/proof-path public ledgers and proof-path LOC commands from the
original plan. Compare the exact sorted dependency list, not only its count.

For the five TT1 commits, additionally report `git diff --numstat ea0ac115..HEAD` grouped as:

- production Rust;
- inline/external test Rust;
- docs;
- shell/build metadata; and
- SQL.

The final narrative must say whether the production correction is net negative. It must not repeat
the incorrect `2,365 Rust/SQL` number.

## 11. Final definition of done

The TT1 correction is ready only when all of these are true:

1. `RunId` and `ArtifactId` cannot be constructed with the wrong algorithm tag.
2. General `ContentDigest` remains dual-algorithm, while ContentRef schema validation admits only
   `Sha256V1` digest strings.
3. `canonicalize_mfm_value` uses one descriptor and rejects capacity before shape walking.
4. `MfmValue::schema_id` and the generic-static persisted-schema caches are absent.
5. Two generic `PersistedSchema` monomorphizations cannot share identity/schema state.
6. Match projection derives tag, exact bytes, ContentRef, and typed value from the registered
   selector descriptor and selected payload codec through one projection/qualification authority,
   without serialization.
7. External, adjacent, Box, nested Box, generic, and manual closed-sum Match cases work; unsupported
   payload/tagging shapes reject before Store access.
8. Both Program root contracts resolve during association.
9. Capability ABI drift and Pure/Read State identity collisions reject deterministically.
10. State/capability static identity errors map to `ProgramError::InvalidContract` at authoring and
    `RuntimeError::IncompatibleAssembly` at builder registration; missing persisted associations
    reject before dependent IO.
11. Canonical Program ingress rejects `execution_binding`, `physical_target_ref`, and
    `adapter_implementation_ref` as unknown retired fields.
12. A Read State with `Failure = Never` works.
13. An indeterminate-but-absent Pure conclusion recomputes and commits on resume.
14. EVM derives fragment start and every fragment-internal position from `declarations.len()` and
    has no ignored/caller-supplied fragment-internal index authority; Portfolio still supplies the
    external failure/completion successors.
15. Memory rejects an empty retained frame while an empty synchronization shell remains absent.
16. PostgreSQL classifies a wrong marker column type as `StoreOpenError::Incompatible`.
17. Journal exact one-byte-over object, maximum sequence envelope, repeated cross-frame ref, and
    wrong RunId tag regressions pass.
18. Current design docs spell `mfm.run.frame.v1`, and current App/Portfolio text contains no
    fixed-tenant claim.
19. A dangling symlink at a forbidden path fails the cutover scanner and is removed by canary
    cleanup.
20. The workspace remains at 18 members and no more than 45 normal internal edges.
21. No new public API name, schema, wire, table, migration, public error type/variant, or
    compatibility path was added.
22. Production Rust is net negative across TT1, focused checks pass, and the one final CI passes on
    the exact reviewed tree.
23. Every required original-cutover and TT1 per-commit/cumulative architect record is attached with
    all blocks resolved; unavailable prior evidence is replaced by a named fresh review, not an
    unsupported approval claim.

## 12. Rollback and data boundary

These are code-correctness follow-ups, not a new deployment format. They do not change Program v2,
`mfm.run.frame.v1`, `mfm.run-history-postgres.v1`, frame-head algorithms, SQL tables, or public
Portfolio result fixtures.

Rollback is by whole follow-up commit only. Do not preserve a wrong-tag ID constructor, old Match
visitor, caller index, relaxed physical validator, pre-catalog marker decode, or symlink-blind scan
behind a feature flag. If a commit must be reverted, revert that complete coherent commit and do not
mix its producer and consumer halves.

No database migration, retained-data rewrite, compatibility reader, fixture reset, or rollout
coordination is required. The original fresh-baseline/no-mixed-version deployment rule remains in
force.
