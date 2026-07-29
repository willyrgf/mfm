# Recoverability App, CLI, and REST Surface v1

Status: implemented schema-frozen current contract established by the atomic cutover described by
[`RFC_REFACTOR_RECOVERABILITY.md`](../RFC_REFACTOR_RECOVERABILITY.md)

Contract id: `mfm.recoverability-app-surface.v1`

This document records the frozen minimal application and transport surface for recoverability v1.
Its current encodings and vectors are fixed by
`contracts/recoverability/v1/annex.json`, `contracts/recoverability/v1/corpus.json`, and
`contracts/recoverability/v1/README.md`; exact artifact hashes and counts are recorded by
`COMMIT2_ARTIFACT_METADATA` in the RFC's
[Canonical Schema and Golden-Vector Gate](../RFC_REFACTOR_RECOVERABILITY.md#canonical-schema-and-golden-vector-gate).
It is the exact current application and transport contract. The retained pre-cutover inventory and
gate evidence live in
[`recoverability-cutover-gates-v1.md`](recoverability-cutover-gates-v1.md). No compatibility
commands, routes, DTOs, or readers survive.

## Canonical annex boundary

The field names, variants, route shapes, command shapes, authorization purposes, and disclosure
rules below are fixed. The frozen canonical schema annex owns:

- textual encodings of typed identifiers, content digests, journal heads, and references;
- canonical bytes and content addresses of planning profiles and other hashed structures;
- the transport representation of retained typed values that are not already reviewed JSON;
- opaque page-cursor bytes; and
- the media type, byte encoding, manifest encoding, and external digest-value encoding of portable
  exports.

This contract refers to those values by their logical types and does not invent provisional byte
encodings. A cursor, identifier, digest, or export manifest is never bearer authority.

Every semantic identity uses the one universal hash derivation:

```text
H(domain, value) =
    SHA-256(JCS({
        "domain": exact_versioned_domain,
        "value": exact_schema_preimage,
    }))
```

The preimage is schema-validated and float-free. Prefix/NUL framing, alternate envelopes, and
compatibility decoding are illegal. Plain content addressing alone is
`SHA-256(exact retained bytes)`.

The executor's lightweight reference is exactly
`mfm_ids::ContentRef { schema_id, content_digest }`. It conveys content identity only. A retained
journal object instead uses the full producer-bound
`mfm_journal::ValueRef { artifact_id, content_digest, evidence_hash, schema_id,
semantic_type_id, role, byte_length, media_type, producer_binding }`. Neither reference is bearer
authority, and a `ContentRef` cannot substitute for a `ValueRef`.

## Authentication, tenant derivation, and grants

REST accepts one credential form:

```http
Authorization: Bearer <opaque credential>
```

Every CLI run command requires the global option:

```text
--access-token-file PATH
```

There is no raw-token argument, tenant argument, tenant header, cookie, or application session. The
CLI reads bounded credential bytes, removes at most one terminal LF or CRLF, and passes them in a
zeroizing secret type that is not cloneable, serializable, or formattable.

The app owns one injected authentication and authorization boundary:

```text
authorize(
    credential: &SecretCredential,
    grant: RunAccessGrant,
    target: &AccessTarget,
) -> Result<AuthorizedTenant, AccessPolicyError>

AccessTarget =
    AdmitTarget {
        store_scope_id,
        entry_point_operation_id,
        invocation_identity,
    }
  | RunTarget {
        store_scope_id,
        run_id,
    }

AuthorizedTenant {
    tenant_scope_id: TenantScopeId,
}
```

The policy authenticates and authorizes in one call. Each accepted credential resolves to exactly
one immutable tenant. `tenant_scope_id` is the scalar `TenantScopeId`, not a content reference. It
comes only from the trusted policy result; request paths, headers, bodies, CLI arguments,
configured targets, and run payloads cannot select or override it. The policy ACL maps its
authenticated caller and tenant to the exact closed grants:

```text
Admit
Drive
Replay
ReadPublic
InspectTrace
InspectAudit
Export
```

ACLs, principals, credentials, membership, and revocation state remain outside the semantic run
journal. Revocation causes a later policy call to fail. Every protected app operation calls the
policy again; neither CLI nor REST retains an authenticated session.

After authorization, the app mints a sealed, transient authority:

- `RunAccessAuthority<Admit>` binds store scope, tenant scope, stable entry-point operation id, and
  invocation identity.
- Every other `RunAccessAuthority<G>` binds store scope, tenant scope, run id, and the exact grant
  `G`.

The authority has private fields and is not `Clone`, `Copy`, `Debug`, `Display`, serializable, or
deserializable. Kernel and store boundaries validate every binding. There is no generic object
grant and no standalone object reader.

For a cross-run trace source, the app performs a separate exact `InspectTrace` policy decision and
mints a separate source-run authority. `GrantDenied` or a wrong-tenant decision supplies no source
authority and remains redacted; `AuthenticationRequired` is fatal for the request. A portable
semantic export performs a separate exact `Export` decision for every dependency run and fails
rather than emitting a bundle that claims complete verification without its required proof
closure.

Authentication failures use these public classifications:

| HTTP | Code | Meaning |
| --- | --- | --- |
| `401` | `AuthenticationRequired` | The credential is missing, malformed, expired, revoked, or otherwise invalid. |
| `403` | `GrantDenied` | The credential is valid but the exact grant and target are denied. |
| `404` | `RunNotFound` | The caller has the grant, but no matching run exists in its authenticated tenant. This also covers a run id owned by another tenant. |

Errors never echo credential bytes, subjects, ACL entries, tenant membership, or backend
diagnostics.

## Published entry points and planning profiles

V1 publishes exactly two entry points:

| Entry point id | Stable operation id | Configured root | Public output |
| --- | --- | --- | --- |
| `mfm.portfolio/snapshot@1` | `mfm.portfolio/snapshot` | `PortfolioConfig` selected by `{ target }` | `PortfolioPublicOutputs` |
| `mfm.evm/submit-transaction@1` | `mfm.evm/submit-transaction` | `EvmSubmitTransactionRequest` selected by `{ target }` | `EvmSubmitTransactionPublicOutputs` |

Each mapping owns one exact profile:

```text
PlanningProfile {
    version: "mfm.planning-profile.v1",
    planner_contract_ref,
    planner_implementation_ref,
    framework_policy_refs: [],
    canonical_profile_parameters: {},
}

planning_profile_ref = content address of that exact PlanningProfile
input                 = { target }
```

The versioned `entry_point_id` selects the compiled registration. The stable, unversioned
`entry_point_operation_id` participates in run identity. The ordered empty
`framework_policy_refs` list and empty parameter object are part of the exact profile content.
There is no minimum-profile, compatible-profile, or implicit-superset rule.

Generic Rust program-authoring helpers grant no admission authority. All product admission passes
through one of these compiled entry-point/profile registrations.

Entry-point discovery is unauthenticated because it exposes no run, fact, credential, or tenant
history. Production bootstrap self-attests the executable, seals the qualified registry, and
builds the exact contracts once; the resulting `Application` caches that immutable slice.
Calling `application.entry_points()` performs no I/O and no authorization:

```text
EntryPointContract {
    version: "mfm.entry-point-contract.v1",
    entry_point_id,
    entry_point_operation_id,
    planning_profile_ref,
    planning_profile: {
        version: "mfm.planning-profile.v1",
        planner_contract_ref,
        planner_implementation_ref,
        framework_policy_refs,
        canonical_profile_parameters,
    },
    input_schema_id,
    public_output_schema_id,
}
```

## Invocation identity and admission

Admission requires a caller-generated invocation identity in canonical lower-case, hyphenated
UUIDv4 form:

```text
xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx
```

It is a non-secret logical-start idempotency value and is persisted. Missing values, non-v4 UUIDs,
upper-case or non-hyphenated forms, whitespace, and arbitrary strings are rejected. The app and
transports never generate a replacement. A caller must retain the identity and reuse the exact
admission request after a timeout or connection ambiguity.

Run identity is:

```text
run_id = H(
    "mfm.run-id.v1",
    {
        store_scope_id,
        tenant_scope_id,
        entry_point_operation_id,
        invocation_identity,
    },
)
```

Admission is admit-only. It may resolve current configuration, plan and certify, self-attest the
current executable, retain root objects, and append `RunAdmitted`. It never invokes a semantic
capability and never calls `drive_once`.

Repeating the same identity and exact root candidate returns `attached`. Different root material
for the same identity returns `AdmissionConflict`; it never creates a second run. A connection
ambiguity returns `outcome_unknown` with the derived run id so the caller can retry the exact
request.

The admission statuses are:

```text
newly_admitted
attached
outcome_unknown
```

`AlreadyActive` does not exist.

## Application facade

The complete run-facing application facade is:

```text
application.entry_points() -> &[EntryPointContract]

admit_run(credential, AdmitRunRequest) -> AdmitRunResponse
drive_once(credential, run_id) -> DriveResponse
read_public_run(credential, run_id) -> PublicRunView
replay_run(credential, run_id, ReplayRequest) -> ReplayResponse
read_transition_trace(credential, run_id, PageRequest) -> TransitionTracePage
read_access_audit(credential, run_id, PageRequest) -> AccessAuditPage
export_run(credential, run_id, ExportRequest) -> PortableRunExport
```

There is no public fact service, cross-run run list/watch, generic stream reader, resume service,
manual-resolution service, runner factory, or arbitrary object reader.

## REST routes and CLI commands

| Purpose | REST | CLI | Grant |
| --- | --- | --- | --- |
| Entry-point discovery | `GET /v1/entry-points` | `mfm ops list` | none |
| Admit | `POST /v1/runs` | `mfm run admit` | `Admit` |
| Execute one action | `POST /v1/runs/{run_id}/drive` | `mfm run drive` | `Drive` |
| Status and public output | `GET /v1/runs/{run_id}` | `mfm run show` | `ReadPublic` |
| Replay | `POST /v1/runs/{run_id}/replay` | `mfm run replay` | `Replay` |
| Transition trace | `GET /v1/runs/{run_id}/trace` | `mfm run trace` | `InspectTrace` |
| Safe access audit | `GET /v1/runs/{run_id}/audit` | `mfm run audit` | `InspectAudit` |
| Portable export | `POST /v1/runs/{run_id}/exports` | `mfm run export` | `Export` |

Health and readiness remain unauthenticated. Health is process liveness only. Readiness performs
bounded PostgreSQL writable-lineage probes against the already opened run store and independently
fenced executor ledger; both are mandatory. Success uses the normal envelope with data exactly
`{ "ok": true }` and exposes no component checks. Every probe failure collapses to the fixed
`503 NotReady` response with `The service is not ready`. It performs no EVM request, DNS lookup,
signer or unlock-file access, provider check, semantic callback, or run operation; the
bootstrap-qualified sealed process capabilities are sufficient without requalification.
`GET /v1/runs` does not exist.
The CLI does not carry a second static discovery catalog: `mfm ops list` accepts the same database
and runtime-configuration connection inputs, performs the same qualified production
`Application` bootstrap as run commands, and then reads the cached slice. The repository
standalone binary has no deployment writer-fence provider, so this bootstrap fails with the same
redacted `AuthoritativeWriterFenceUnavailable` error. A deployment-composed application may serve
the unauthenticated cached discovery method without further I/O or authorization.
`HEAD` is not an alias for any listed route. Every listed path rejects it with
`405 MethodNotAllowed` without invoking GET authentication or application work. The response
retains the reviewed status and headers, including `Allow` with only the route's real `GET` or
`POST` method, but, as required for HEAD, has an empty body.

JSON REST endpoints retain one envelope:

```json
{
  "status": "success",
  "data": {}
}
```

Errors retain one redacted error envelope. CLI JSON renders the corresponding `data` DTO directly;
CLI text is presentation only and cannot add semantic fields.

Every application-bound command accepts `--database-url URL` and `--runtime-config PATH`; the
database URL otherwise comes from `DATABASE_URL`. Every run command additionally requires the
global `--access-token-file PATH`. The exact semantic argument shapes are:

```text
mfm ops list
mfm run admit ENTRY_POINT_ID --invocation-identity UUID --target TARGET
mfm run drive RUN_ID
mfm run show RUN_ID
mfm run replay RUN_ID --mode verify
mfm run replay RUN_ID --mode reproduce|compare-current \
  --portable-export PATH --portable-export-ref-file PATH
mfm run trace RUN_ID [--cursor CURSOR] [--limit 1..500]
mfm run audit RUN_ID [--cursor CURSOR] [--limit 1..500]
mfm run export RUN_ID --kind semantic|audit --output PATH --ref-output PATH
```

### Admission request and response

```json
{
  "version": "mfm.admit-run-request.v1",
  "entry_point_id": "mfm.portfolio/snapshot@1",
  "invocation_identity": "de305d54-75b4-431b-adb2-eb6b9e546014",
  "input": {
    "target": "portfolio-name"
  }
}
```

```text
AdmitRunResponse {
    version: "mfm.admit-run-response.v1",
    run_id,
    admission: "newly_admitted" | "attached" | "outcome_unknown",
    entry_point_id,
    entry_point_operation_id,
    invocation_identity,
    planning_profile_ref,
}
```

REST returns `201` for `newly_admitted`, `200` for `attached`, and `202` for
`outcome_unknown`. A changed root returns `409 AdmissionConflict`.

The CLI shape is:

```text
mfm run admit mfm.portfolio/snapshot@1 \
  --invocation-identity de305d54-75b4-431b-adb2-eb6b9e546014 \
  --target portfolio-name
```

### Drive request and response

The REST request has no body. The CLI accepts only the run id and connection/authentication
options. Neither transport accepts a node, request, observation, hook, result, retry mode, or
scheduler choice.

```text
DriveResponse =
    Advanced {
        run_id,
        kind: "advanced",
        journal_head,
    }
  | Waiting {
        run_id,
        kind: "waiting",
        journal_head,
        reason:
            "retryable_evidence_gap" |
            "operational_block" |
            "integrity_block",
    }
  | Closed {
        run_id,
        kind: "closed",
        closure_ref,
    }
```

V1 has no drive-until-waiting transport convenience. A host repeats `drive` explicitly.
`drive_once` performs at most one semantic transition or one audited application-protocol
operation.

### Public status and public output

`GET /v1/runs/{run_id}` and `mfm run show` are the only ordinary run reads. Separate status and
public-output surfaces do not exist.

Production mints one exact `ReadPublic` run authority and makes one
`store.read_public_run(authority)` call. The store returns only a sealed
`VerifiedPublicRunView`; the application consumes that already annex-validated projection through
its crate-private `PublicRunView::from_verified` constructor after checking the exact schema
contract, without reconstructing or re-decoding it. The application and transports do not load a
raw journal or generic verified run view, invoke a resolver, or reconstruct status and public
outputs through separate paths.

```text
PublicRunView {
    version: "mfm.public-run-view.v1",
    run_id,
    status: "active" | "succeeded" | "failed",
    journal_head: Head,
    semantic_head: Head,
    active: ActiveRunView | null,
    public_outputs: [PublicOutput],
}

Head {
    run_sequence,
    commit_digest,
}

ActiveRunView {
    ready_node_ids: [node_id],
    pending_effects: [PendingEffect],
}

PendingEffect {
    node_id,
    effect_key,
    executor_status:
        "not_authorized" |
        "authorized_unobserved" |
        "returned" |
        "did_not_enter" |
        "indeterminate",
}

PublicOutput {
    name,
    schema_id,
    value_digest,
    value,
}
```

Node and output arrays use certified graph/binding order. `active` is non-null exactly while
`run_phase = Open`. Terminal status requires `RunClosed`; `succeeded` requires every certified
required-success node to have succeeded and the certified public-output binding to exist.
Otherwise the closed run is `failed`.

The store derives `ready_node_ids` and `pending_effects` structurally from the verified fold.
Access eligibility, candidate selection, and wait classification remain runtime-owned and appear
only in the separately authorized `drive_once` response; the callback-free public store read does
not duplicate or approximate them.

The store verifies and dereferences the one folded aggregate public-output assembly, then projects
each certified destination binding in binding order. `PublicOutput.name` is the exact certified
destination field, `schema_id` is that binding's certified schema, `value_digest` is the raw
content digest of the exact canonical JSON subtree, and `value` is that subtree. It never
dereferences producer outputs as a second authority path. Event ids, artifact ids, transition
inputs, non-public outputs, facts, observations, evidence, audit chronology, source scope, and
timestamps are absent.

### Replay request and response

```text
ReplayRequest =
    { mode: "verify" }
  | {
        mode: "reproduce",
        portable_export_ref: ContentRef,
        portable_export_base64url,
    }
  | {
        mode: "compare_current",
        portable_export_ref: ContentRef,
        portable_export_base64url,
    }
```

`portable_export_base64url` is canonical unpadded base64url. Its encoded length is at most
22,369,622 characters and its decoded canonical export is at most 16,777,216 bytes. Padding,
noncanonical base64url, an audit export, a wrong schema or raw-byte digest, or a bundle bound to a
different store, tenant, run, semantic head, or closure is rejected before any resolver callback.
The two portable-export fields are required for `reproduce` and `compare_current` and forbidden for
`verify`. The complete REST replay body is bounded to 22,373,718 bytes. The CLI reads the export
incrementally and bounds its strictly canonical `ContentRef` sidecar to 4,096 bytes.

Malformed, noncanonical, missing, forbidden, digest-mismatched, or store/tenant/run/head/closure
binding-mismatched caller artifacts return `400 ReplayArtifactInvalid` with the exact safe message
`The replay artifact is invalid.` Any encoded, decoded, sidecar, or complete replay-body limit
violation returns `400 ReplayArtifactTooLarge` with
`The replay artifact exceeds the allowed size.` These caller errors are distinct from an
authenticated store or recorded-history integrity failure, which returns the fixed internal
`500 ReplayVerificationFailed` contract.
Candidate recorded-history, execution, and comparison-integrity failures collapse into that same
fixed response. A missing sealed current candidate instead returns
`503 RuntimeCatalogUnavailable` with `The exact admitted runtime catalog is unavailable`; it is
distinct from an unavailable exact historical executable, which remains a successful canonical
`reproduced` result with `result: "unavailable"`.

Non-verify replay independently requires both `Replay` and same-run `Export` policy decisions.
The supplied bytes must have been obtained through an explicit semantic export; replay never
fetches, generates, or silently upgrades authority to obtain a bundle. `compare_current` always
self-attests the serving executable and catalog. A caller can supply evidence bytes but cannot
name, label, upload, or supply an executable.

The current production composition has no historical executable resolver or isolated sandbox. Its
`reproduce` result is therefore the annex `unavailable` variant, with no live-runtime fallback.

```text
ReplayResponse =
    Verified {
        kind: "verified",
        run_id,
        journal_head,
        fact_selections: [FactSelectionCompleteness],
    }
  | Reproduced {
        kind: "reproduced",
        run_id,
        result: "matched" | "mismatch" | "unavailable",
        transition_ref?, # present only when the frozen schema permits it
    }
  | CandidateComparison {
        kind: "candidate_comparison",
        run_id,
        candidate_executable_identity_ref,
        report,
    }
```

All replay modes perform zero live semantic-capability, provider, executor, domain filesystem, or
signer IO and append nothing. Invalid recorded history fails before reproduction or comparison.
The reproduced `unavailable` result deliberately has no reason, diagnostic, stage, or other
extension field. The canonical annex owns the closed result shape.

### Transition trace request and response

The exact query parameters are `cursor` and `limit`. The default limit is `100`; the maximum is
`500`. The first page fixes the current journal head. The opaque cursor binds that head and the
next record coordinate but carries no access authority. Every page independently requires
`InspectTrace`, including a fresh separate authorization for every source run named by that page.
Source `GrantDenied` and wrong-tenant decisions omit that source authority; source
`AuthenticationRequired` fails the page.
The application alone decodes and encodes the purpose-bound cursor. It constructs a sealed
`TransitionTracePageRequest` from only the optional fixed head, a `u32` start, and a bounded `u16`
limit. Replay first calls `discover_transition_trace_sources`, which returns an opaque
`TransitionTraceSourceRequirements` and the canonical sorted, duplicate-free direct source-run ids
for that exact fixed-head page. The application performs a fresh policy decision for each returned
source id in that order. Replay then consumes the opaque requirements with only the canonical
sorted, duplicate-free same-store and same-tenant source authorities that were granted; extra,
reordered, duplicate, or wrong-scope authorities are rejected.

Discovery and rendering use the same fixed-head page token. They do not cache source decisions,
scan the whole run at its current head, or reuse requirements across pages. A denied, wrong-tenant,
missing, or authorized-but-absent source produces the same redacted lineage. Corrupt retained
source material fails integrity verification. The application hides the returned `has_more` and
`next_index` scalars inside `next_cursor`.

```text
PageRequest {
    cursor: string | null,
    limit: integer | null,
}

TransitionTracePage {
    run_id,
    at_journal_head,
    transitions: [TransitionTrace],
    next_cursor: string | null,
}

TransitionTrace {
    version: "mfm.transition-trace.v1",
    transition_ref,
    containing_commit_digest,
    node_id,
    canonical_expansion_path,
    transition_kind:
      "pure_settled" |
      "read_settled" |
      "effect_requested" |
      "effect_settled" |
      "dependency_skipped",
    before: TransitionBefore,
    inputs: [NamedTraceInput],
    request: RetainedValue | null,
    consumed_observation: RetainedValue | null,
    result: RetainedValue | null,
    outputs: [NamedRetainedValue],
    facts: [TraceFact],
    evidence: [NamedRetainedValue],
    terminal_outcome:
      { kind: "succeeded" } |
      { kind: "failed", typed_failure_ref } |
      { kind: "skipped", blocking_sources } |
      null,
    after: TransitionAfter,
    closure_ref: closure_ref | null,
}

TransitionBefore {
    journal_head,
    run_state_digest,
    run_phase: "open" | "closed",
    node_phase: "unstarted" | "awaiting_effect" | "terminal",
}

TransitionAfter {
    run_state_digest,
    run_phase: "open" | "closed",
    node_phase: "unstarted" | "awaiting_effect" | "terminal",
    binding_delta,
}

NamedTraceInput {
    field_path,
    lineage,
    source_field_path: field_path | null,
    value: RetainedValue | null,
}

NamedRetainedValue {
    name,
    ordinal: integer | null,
    value: RetainedValue,
}

TraceFact {
    emission_ordinal,
    fact_descriptor_ref,
    claim: RetainedValue,
    subject: RetainedValue,
    response: RetainedValue,
}

RetainedValue {
    schema_id,
    digest,
    media_type,
    content: AnnexEncodedValue,
}
```

`AnnexEncodedValue` is a closed tagged representation frozen by the canonical schema annex; it is
not an arbitrary serialization fallback. Input lineage is exactly run admission, same-run
transition output, same-run transition fact, cross-run value/redaction, config, qualified support,
seed, or context. `source_field_path` retains the exact persisted selection and is null only for a
whole root or transition fact. A denied cross-run source exposes only:

```text
CrossRunRedactedLineage {
    kind: "cross_run_redacted",
    source_ref_digest,
}
```

and has `value: null`. An authorized same-tenant source is dereferenced only after the separate
source-run `InspectTrace` decision. If that separately authorized source is absent, it is rendered
with the same redacted lineage as a denied source; the trace does not disclose which condition
occurred. Every retained value is inlined through this enclosing reader; there is no follow-up
object endpoint.

`source_ref_digest` is the `mfm.cross-run-source-redaction.v1` semantic digest of the complete,
annex-validated `mfm.cross-run-source-ref.v1` tagged value:

```text
SHA-256(JCS({
    "domain": "mfm.cross-run-source-redaction.v1",
    "value": exact_cross_run_source_ref,
}))
```

The preimage includes the selected source-reference variant and all of its fields. It excludes the
destination input's separately rendered `source_field_path`. The destination reader derives the
digest from its verified input manifest before source authorization or dereference; it never
substitutes a source-run id, source-closure digest, retained-content digest, or selected source
object.

A retained `ValueRef` is inlined only after the reader rechecks its selected reachability,
producer binding, certified contract, schema, media type, byte length, and raw content digest.
The rendered `schema_id`, `digest`, and `media_type` are the exact verified reference fields.
`application/json` bytes must already be canonical JSON and use the `canonical_json`
`AnnexEncodedValue` variant. Every other admitted media type uses `bytes` with canonical unpadded
base64url. Invalid JSON never falls back to the byte variant.

`consumed_observation` is not the returned or diagnostic object alone. For `ReadSettled` and
`EffectSettled`, it is the exact consumed `mfm.external-access-observed.v1` journal payload wrapped
for this trace as a `RetainedValue`: that contract's annex schema id, the raw content digest of the
payload's exact canonical bytes, media type `application/json`, and canonical-JSON content. This
preserves the `Returned`, `DidNotEnter`, or `Indeterminate` outcome and its safe-failure metadata.
The wrapper is not persisted, receives no synthetic `ValueRef`, and grants no object authority.

The remaining retained fields have one variant-specific projection:

| Transition | `request` | `result` | `evidence` |
| --- | --- | --- | --- |
| `PureSettled` | null | exact typed failure on failed settlement; otherwise null | empty |
| `ReadSettled` | exact `request_ref` value | exact typed failure on failed settlement; otherwise null | returned value at `outcome.result_ref`, plus an optional `fact_selection_scan_attestation_ref`; or the safe-failure diagnostic at `outcome.safe_failure.diagnostic_ref` |
| `EffectRequested` | exact `semantic_request_ref` value | null | empty |
| `EffectSettled` | exact semantic request from its verified referenced `EffectRequested` | exact typed failure on failed settlement; otherwise null | terminal ensure result, terminal evidence, and the complete verified executor retained closure reached through the consumed observation |
| `DependencySkipped` | null | null | empty |

An effect settlement accepts only its exact returned terminal ensure result. A safe failure,
pending ensure result, wrong request transition, or mismatched authorization, effect, or request
identity fails integrity verification. Its evidence contains each verified external-observation
producer path exactly once: `executor.ensure_result`, `executor.terminal_evidence`, and all reached
members under `executor.delivery_audit.<content-digest>`, `executor.frontier.<content-digest>`,
`executor.terminal_tombstone.<content-digest>`, `executor.terminal_proof.<content-digest>`, and
`executor.domain_evidence.<content-digest>`. It is a typed transitive closure, not a scan of
arbitrary same-authorization objects.

Every evidence item uses its exact external-observation producer field path as `name`, a null
`ordinal`, and annex `field:name` ordering. Output items use their certified output field path as
`name`, their exact output ordinal, and certified binding order. Facts use the exact emission
ordinal and dereference the exact claim envelope, subject, and response authorities. A successful
settlement has `result: null`; its success material exists only in outputs and facts. A failed
settlement has no outputs or facts, but retains the observation evidence that caused the failed
callback. Only the observation named by the settlement is projected; earlier, insufficient,
failed, or unmatched observations remain audit-only.

### External-access audit request and response

Audit paging uses the same `cursor` and `limit` contract and fixes the first page's journal head.
The application alone decodes the opaque cursor into its bound journal head and unsigned next
index, then calls the store projection with only that optional head, a `u32` start, and the bounded
`u16` limit. The store returns the owned reviewed entries plus `has_more` and `next_index`; the
application encodes those paging scalars into `next_cursor`. Neither internal scalar is exposed in
the public page:

```text
AccessAuditPage {
    run_id,
    complete_as_of_journal_head,
    entries: [AccessAuditEntry],
    next_cursor: string | null,
}

AccessAuditEntry {
    authorization_ref,
    observation_ref: observation_ref | null,
    authorization_journal_head: Head,
    observation_journal_head: Head | null,
    capability_binding_ref,
    capability_operation_id,
    request_ref: ValueRef,
    status:
        "authorized_unobserved" |
        "returned" |
        "did_not_enter" |
        "indeterminate",
    result_ref: ValueRef | null,
    failure: SafeFailure | null,
    effect_key: effect_key | null,
    delivery_audit_ref: ValueRef | null,
    delivery_audit_terminal: boolean | null,
}

SafeFailure<Code, DiagnosticRef> {
    version: "mfm.safe-failure.v1",
    safe_failure_contract_ref: ContentRef,
    stable_code: Code,
    failure_class: FailureClass,
    boundary_stage: BoundaryStage,
    coarse_size_class: CoarseSizeClass | null,
    diagnostic_ref: DiagnosticRef | null,
}
```

`delivery_audit_ref` is the sole current delivery-audit head exposed by the public projection.
`delivery_audit_terminal` is presentation-only: it is `true` for a verified terminal returned
ensure, `false` for a verified pending returned ensure, and `null` for reads or when no returned
ensure has been verified. Only the reference is persisted in the journal access-audit entry.

`mfm-capabilities` owns this generic envelope. A direct journal observation uses
`DiagnosticRef = ValueRef`; executor-internal evidence uses `DiagnosticRef = ContentRef` and must
pass ordinary journal object admission before it appears in this audit. The selected
`safe_failure_contract_ref` fixes every legal code, class/stage/size combination, and diagnostic
union. The universal enums are exactly:

```text
FailureClass =
    authorization
  | configuration
  | request
  | cancellation
  | transport
  | destination
  | unrepresentable_response
  | integrity
  | resource_conflict
  | unclassified

BoundaryStage =
    before_boundary_entry
  | boundary_entry
  | boundary_observation

CoarseSizeClass =
    zero             # exactly 0 bytes
  | up_to_16_kib     # 1..=16_384 bytes
  | up_to_1_mib      # 16_385..=1_048_576 bytes
  | over_1_mib       # at least 1_048_577 bytes
```

A null size means unavailable. It describes the discarded provider-controlled envelope, not the
diagnostic. Every retained diagnostic is an exact canonical union of at most 16,384 bytes.

The EVM read stable codes are exactly:

```text
routing_generation_unavailable
configuration_invalid
request_invalid
access_cancelled
transport_failed
http_status
json_rpc_error
response_invalid
response_missing_result
response_too_large
unclassified_failure
```

Bitcoin uses that same list plus `scan_busy`. The diagnostic unions are exactly:

```text
EvmSafeDiagnostic =
    HttpStatus { status: u16 }
  | JsonRpcError { code: i64 }
  | ResponseInvalid {
        kind:
            malformed_envelope
          | missing_result
          | invalid_result
          | too_large
    }

BitcoinSafeDiagnostic =
    HttpStatus { status: u16 }
  | JsonRpcError { code: i64 }
  | ResponseInvalid {
        kind:
            malformed_envelope
          | missing_result
          | invalid_result
          | too_large
    }
  | ScanBusy
```

The EVM and Bitcoin mappings are:

| Code or condition | Observation | Class | Stage | Diagnostic |
| --- | --- | --- | --- | --- |
| `routing_generation_unavailable` | `DidNotEnter` | `authorization` | `before_boundary_entry` | none |
| `configuration_invalid` | `DidNotEnter` | `configuration` | `before_boundary_entry` | none |
| `request_invalid` | `DidNotEnter` | `request` | `before_boundary_entry` | none |
| `access_cancelled` before entry | `DidNotEnter` | `cancellation` | `before_boundary_entry` | none |
| `access_cancelled` after possible entry | `Indeterminate` | `cancellation` | `boundary_entry` | none |
| `transport_failed` with proven no entry | `DidNotEnter` | `transport` | `before_boundary_entry` | none |
| `transport_failed` after possible entry | `Indeterminate` | `transport` | `boundary_entry` | none |
| `http_status` | `Indeterminate` | `destination` | `boundary_observation` | `HttpStatus` |
| `json_rpc_error` | `Indeterminate` | `destination` | `boundary_observation` | `JsonRpcError` |
| `response_invalid` | `Indeterminate` | `unrepresentable_response` | `boundary_observation` | `ResponseInvalid { malformed_envelope \| invalid_result }` |
| `response_missing_result` | `Indeterminate` | `unrepresentable_response` | `boundary_observation` | `ResponseInvalid { missing_result }` |
| `response_too_large` | `Indeterminate` | `unrepresentable_response` | `boundary_observation` | `ResponseInvalid { too_large }` |
| `unclassified_failure` | `Indeterminate` | `unclassified` | `boundary_observation` | none |

For EVM, the exact retryable HTTP set is `408`, `425`, `429`, `500`, `502`, `503`, `504`, and
`507`; the exact retryable JSON-RPC set is `-32603`, `-32001`, `-32002`, and `-32005`. The
reviewed busy/rate-limited values are HTTP `429` and JSON-RPC `-32005`.

Bitcoin uses the same retryable HTTP set. `scan_busy` is exactly
`Indeterminate/destination/boundary_observation/ScanBusy` and requires the reviewed Bitcoin Core
code `-8` and exact-message classifier; a near match is `json_rpc_error`. It is the only retryable
Bitcoin JSON-RPC result in v1.

The state callback policy is also closed; wrapper-shape validity alone does not decide semantic
sufficiency:

| Safe-failure condition | EVM callback verdict | Bitcoin callback verdict |
| --- | --- | --- |
| `routing_generation_unavailable`, `configuration_invalid`, or `request_invalid` | `InvalidEvidence` | `InvalidEvidence` |
| either legal `access_cancelled` or `transport_failed` stage | `InsufficientEvidence` | `InsufficientEvidence` |
| `http_status` in the exact retryable set | `InsufficientEvidence` | `InsufficientEvidence` |
| any other classifier-admitted HTTP `u16` | typed terminal read-validation failure | typed terminal read-validation failure |
| `json_rpc_error` in the exact EVM retryable set | `InsufficientEvidence` | typed terminal read-validation failure |
| any other classifier-admitted JSON-RPC `i64` | typed terminal read-validation failure | typed terminal read-validation failure |
| exact `scan_busy` | not admitted | `InsufficientEvidence` |
| `response_invalid`, `response_missing_result`, or `response_too_large` | `InvalidEvidence` | `InvalidEvidence` |
| `unclassified_failure` | `InsufficientEvidence` | `InsufficientEvidence` |

The route/configuration/request rows are post-authorization invariant violations: exact route and
configuration resolution must already have succeeded, and request authorship is total over a
checked frame. The response-invalid rows contain no schema-valid typed result. These rows block;
they do not become a retry or an invented domain failure. `response_invalid` admits only
`malformed_envelope` or `invalid_result`; `response_missing_result` and `response_too_large`
exclusively admit their homonymous diagnostic kinds. A wrong contract reference, code/class/stage,
diagnostic pairing, or size combination rejects structurally before the callback.

The only typed terminal read-failure values are:

```text
{ version: "mfm.evm-read-terminal-failure.v1",
  kind: destination_rejected | source_mismatch | anchor_changed }

{ version: "mfm.bitcoin-read-terminal-failure.v1",
  kind: destination_rejected | source_mismatch | scan_incomplete | anchor_changed }
```

They contain no numeric status/code, source, chain, network, anchor, provider text, or diagnostic;
the consumed observation remains the evidence. Only a nonretryable numeric destination rejection
uses `destination_rejected`. EVM chain mismatch and Bitcoin network mismatch use
`source_mismatch`; `scantxoutset.success = false` uses `scan_incomplete`.

Observed EVM source/chain mismatch and anchor drift, and Bitcoin source/network mismatch, anchor
drift, and `scantxoutset.success = false`, are bounded `Returned` typed semantic failures rather
than `SafeFailure` values. Bitcoin is deliberately unregistered under its closed repeat-work-safe
provider disposition and is outside the fixed implementation sequence.

Precise timestamps, source scope, provider routes, credentials, raw request or response bodies, raw
provider errors, executor vault references, and error source chains are absent.

### Portable export request and response

```text
ExportRequest =
    { kind: "semantic" }
  | { kind: "audit" }
```

`PortableRunExport` is the canonical bundle itself, not a byte-wrapper DTO:

```text
PortableRunExport {
    version: "mfm.portable-run-export.v1",
    media_type,
    manifest,
    members,
}
```

The canonical schema annex freezes the bundle, manifest, member, media-type, byte, and external
digest-value encodings. The REST response is the canonical serialization of this object, not a
base64 field inside the JSON success envelope. The canonical bytes contain no digest of themselves
in the manifest or a nested member. Once the final bytes exist, the app computes
`SHA-256(exact final canonical bytes)`. That external value is transport metadata, not a field of
`PortableRunExport`; REST returns it only in the exact `Mfm-Content-Digest` header and sets
`Content-Type` from the bundle's `media_type`. There is no separate manifest digest, bundle
digest, domain-hashed export identity, compatibility checksum, or second serialized byte-wrapper.
The CLI requires:

```text
mfm run export RUN_ID --kind semantic|audit --output PATH --ref-output PATH
```

It preflights both new paths, then atomically writes each file: the exact canonical bundle bytes to
`--output` and the bundle's strict canonical JSON `ContentRef` object to `--ref-output`. Neither
path is overwritten. It never renders the bundle through normal JSON/text output.

A semantic export is fixed at the current semantic head, or the closure coordinate for a closed
run. An audit export is complete only as of its recorded journal head and records the greatest
delivery-audit head committed to the MFM journal for each effect at that head. The manifest records
the store scope, tenant scope, run id, export kind, exact coordinate, dependency-run identities,
and closure membership, but contains no digest of itself and grants no later access.

A missing root returns the ordinary tenant-indistinguishable `RunNotFound`. A dependency
`AuthenticationRequired` decision, including revocation during recursive authorization, is fatal
and returns `AuthenticationRequired`. A denied or wrong-tenant dependency returns
`SourceRunExportDenied` before any source authority is minted; the app does not emit an
unverifiable partial semantic bundle. Once an exact dependency has been separately authorized, any
absence of that source run named by verified append-only history is an integrity failure and
returns the fixed `ReplayVerificationFailed` response rather than `RunNotFound`, both during
recursive dependency discovery and final export. The sealed exporter rejects missing, duplicate,
reordered, wrong-scope, and extra dependency authorities. Credentials and ACL state never appear in
either export. Offline verification of caller-held bytes requires no live credential and grants no
store or object access.

## Exact deletion scope

Delete without aliases or hidden compatibility paths:

- `crates/app/src/public_facts/*`;
- app fact, cross-run list/watch, generic stream, manual-resolution, and resume methods;
- standalone `PublicOutputReadAuthority`;
- `RunModeStatus`, all saga/manual/remediation/resource-lane/attempt DTOs,
  `RunStreamResponse`, `RunEventRef`, old `ReplayResponse`, and `AlreadyActive`;
- projection-backed status/read contexts, replay-broker use, execution claims, leases, heartbeats,
  and manual-resolution execution;
- `bin/cli/src/commands/facts.rs` and `bin/cli/src/commands/facts_tests.rs`;
- the old CLI run modules `list`, `manual_resolution`, `stream`, `resume`, `start`, `status`, and
  `public_output`, replacing their registration with `admit`, `drive`, and `show`, not aliases;
- CLI commands `run list`, `run start`, `run resume`, `run status`, `run stream`,
  `run public-output`, and `run manual-resolution`;
- REST `/v1/facts/*`, `GET /v1/runs`, `/v1/runs/start`, and the run-scoped `/resume`,
  `/manual-resolution`, `/status`, `/stream`, and `/public-output/{schema_id}` routes;
- `RestProcessRole`; purpose grants are the route admission contract, while deployments may omit
  live capability credentials as independent defense in depth;
- generic runner-factory, adapter-lifecycle, saga, worker-attempt, resource-lane, and
  side-effect-phase app/transport DTOs; and
- `docs/saga.md` plus every old public fact/list/saga/attempt/lane/manual-resolution contract in
  current app, CLI, and REST documentation.

No v1 route or command browses facts, lists runs across a tenant, watches cross-run changes,
accepts manual evidence, marks an effect successful/not-applied/abandoned, prints a generic event
stream, or dereferences an arbitrary object.

## Contract tests

The current contract tests prove:

- each grant independently permits only its exact app, kernel, and store operation;
- missing and wrong-purpose, wrong-store, wrong-tenant, and wrong-run authorities fail;
- credential bytes cannot be cloned, serialized, formatted, logged, persisted, or included in
  public errors;
- requests cannot select tenant, and the same entry point/invocation under two tenants derives
  different run ids;
- invocation identity accepts only canonical UUIDv4, exact retry attaches, changed root conflicts,
  and an ambiguous acknowledgement is retried without a new identity;
- admission performs no live capability call and never drives;
- EVM transaction admission accepts only the authorized tenant and selected configured target,
  and its effect can settle only from exact qualified executor terminal evidence;
- two app processes can alternate `drive_once` calls without process semantic state;
- one drive performs at most one semantic transition or audited application-protocol operation and
  returns only the fixed outcome variants;
- every DTO variant has a golden JSON shape and CLI/REST parity;
- public read derives status from `Open`/`Closed`, required-success nodes, and the certified public
  output, and cannot expose transition inputs, facts, observations, evidence, audit data, or
  non-public objects;
- trace and audit paging remain fixed at the first-page head and cursors confer no authority;
- cross-run trace values require a separately minted source authority and denial produces the
  exact redacted lineage;
- audit serialization admits only the shared `SafeFailure` envelope, exact universal and
  capability-specific combinations, and at-most-16,384-byte closed diagnostics; adversarial
  provider diagnostics remain absent;
- replay modes perform no live IO or append, and `compare_current` binds the self-attested
  executable and exact candidate catalog;
- export bytes match the canonical annex, contain no internal/circular digest, return their one
  raw-byte SHA-256 value as external transport metadata and `Mfm-Content-Digest`, bind exact
  semantic/audit coordinates, and do not overstate delivery-audit qualification;
- offline bundle verification confers no live access;
- deleted commands and routes are rejected, including public facts, list/watch, manual resolution,
  generic stream, and arbitrary object reads; and
- compile-fail tests prevent authority construction, cloning, serialization, and grant
  substitution.

## Rollout assumptions

- Each deployment supplies the app-owned credential policy and ACL source that returns one stable
  tenant scope and supports revocation.
- Before schema reset, each deployment completes and verifies its selected export-or-destroy
  disposition from the cutover inventory.
- The registered EVM mutation executor is usable only when the deployment supplies its qualified
  keyed ledger/backend, exact wallet/resource owner, non-rollback generation, stale/sibling-writer
  exclusion, destination-convergence fence, and guarded signer binding. PostgreSQL persistence or
  co-location does not satisfy those external authorities, and the MFM store writer/WAL fence
  cannot substitute for them.

These assumptions affect deployment readiness, not the logical app or transport schema.

## Material uncertainties

none
