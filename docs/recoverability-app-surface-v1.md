# Recoverability App, CLI, and REST Surface v1

Status: target contract for the atomic cutover described by
[`RFC_REFACTOR_RECOVERABILITY.md`](../RFC_REFACTOR_RECOVERABILITY.md)

Contract id: `mfm.recoverability-app-surface.v1`

This document freezes the minimal application and transport surface for recoverability v1. It is a
contract-only companion to
[`recoverability-cutover-gates-v1.md`](recoverability-cutover-gates-v1.md), not a description of
the pre-cutover implementation. The cutover replaces the old surface atomically and retains no
compatibility commands, routes, DTOs, or readers.

## Canonical annex boundary

The field names, variants, route shapes, command shapes, authorization purposes, and disclosure
rules below are fixed. The canonical schema annex still owns:

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
CLI reads bounded credential bytes, removes at most one terminal newline, and passes them in a
zeroizing secret type that is not cloneable, serializable, or formattable.

The app owns one injected authentication and authorization boundary:

```text
authorize(
    credential: SecretCredential,
    grant: RunAccessGrant,
    target: AccessTarget,
) -> AuthorizedTenant

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
journal. Revocation causes a later policy call to fail. Every app operation calls the policy
again; neither CLI nor REST retains an authenticated session.

After authorization, the app mints a sealed, transient authority:

- `RunAccessAuthority<Admit>` binds store scope, tenant scope, stable entry-point operation id, and
  invocation identity.
- Every other `RunAccessAuthority<G>` binds store scope, tenant scope, run id, and the exact grant
  `G`.

The authority has private fields and is not `Clone`, `Copy`, `Debug`, `Display`, serializable, or
deserializable. Kernel and store boundaries validate every binding. There is no generic object
grant and no standalone object reader.

For a cross-run trace source, the app performs a separate exact `InspectTrace` policy decision and
mints a separate source-run authority. A denied source remains redacted. A portable semantic
export performs a separate exact `Export` decision for every dependency run and fails rather than
emitting a bundle that claims complete verification without its required proof closure.

Authentication failures use these public classifications:

| HTTP | Code | Meaning |
| --- | --- | --- |
| `401` | `AuthenticationRequired` | The credential is missing, malformed, expired, revoked, or otherwise invalid. |
| `403` | `GrantDenied` | The credential is valid but the exact grant and target are denied. |
| `404` | `RunNotFound` | The caller has the grant, but no matching run exists in its authenticated tenant. This also covers a run id owned by another tenant. |

Errors never echo credential bytes, subjects, ACL entries, tenant membership, or backend
diagnostics.

## Published entry point and planning profile

V1 publishes exactly one entry point:

```text
entry_point_id           = "mfm.portfolio/snapshot@1"
entry_point_operation_id = "mfm.portfolio/snapshot"

PlanningProfile {
    planner_contract_ref,
    planner_implementation_ref,
    framework_policy_refs: [],
    canonical_profile_parameters: {},
}

planning_profile_ref = content address of that exact PlanningProfile
input                 = { target }
public_output_schema  = PortfolioPublicOutputs::public_schema_id()
```

The versioned `entry_point_id` selects the compiled registration. The stable, unversioned
`entry_point_operation_id` participates in run identity. The ordered empty
`framework_policy_refs` list and empty parameter object are part of the exact profile content.
There is no minimum-profile, compatible-profile, or implicit-superset rule.

Generic Rust program-authoring helpers grant no admission authority. All product admission passes
through this one compiled entry-point/profile registration.

Entry-point discovery is unauthenticated because it exposes no run, fact, credential, or tenant
history:

```text
EntryPointContract {
    entry_point_id,
    entry_point_operation_id,
    planning_profile_ref,
    planning_profile: {
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

Portfolio admission requires a caller-generated invocation identity in canonical lower-case,
hyphenated UUIDv4 form:

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
        entry_point_operation_id: "mfm.portfolio/snapshot",
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
entry_points() -> [EntryPointContract]

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

Health and readiness remain unauthenticated. `GET /v1/runs` does not exist.

JSON REST endpoints retain one envelope:

```json
{
  "status": "success",
  "data": {}
}
```

Errors retain one redacted error envelope. CLI JSON renders the corresponding `data` DTO directly;
CLI text is presentation only and cannot add semantic fields.

The exact CLI argument shapes, in addition to global connection and `--access-token-file`, are:

```text
mfm run admit ENTRY_POINT_ID --invocation-identity UUID --target TARGET
mfm run drive RUN_ID
mfm run show RUN_ID
mfm run replay RUN_ID --mode verify|reproduce|compare-current
mfm run trace RUN_ID [--cursor CURSOR] [--limit 1..500]
mfm run audit RUN_ID [--cursor CURSOR] [--limit 1..500]
mfm run export RUN_ID --kind semantic|audit --output PATH
```

### Admission request and response

```json
{
  "entry_point_id": "mfm.portfolio/snapshot@1",
  "invocation_identity": "de305d54-75b4-431b-adb2-eb6b9e546014",
  "input": {
    "target": "portfolio-name"
  }
}
```

```text
AdmitRunResponse {
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
        outcome: "advanced",
        journal_head,
    }
  | Waiting {
        run_id,
        outcome: "waiting",
        journal_head,
        reason:
            "retryable_evidence_gap" |
            "operational_block" |
            "integrity_block",
    }
  | Closed {
        run_id,
        outcome: "closed",
        closure_ref,
    }
```

V1 has no drive-until-waiting transport convenience. A host repeats `drive` explicitly.
`drive_once` performs at most one semantic transition or one audited application-protocol
operation.

### Public status and public output

`GET /v1/runs/{run_id}` and `mfm run show` are the only ordinary run reads. Separate status and
public-output surfaces do not exist.

```text
PublicRunView {
    run_id,
    status: "active" | "succeeded" | "failed",
    journal_head: Head,
    semantic_head: Head,
    active: ActiveRunView | null,
    public_outputs: [PublicOutput],
}

Head {
    sequence,
    digest,
}

ActiveRunView {
    ready_node_ids: [node_id],
    access_eligible_node_ids: [node_id],
    next_candidate_node_id: node_id | null,
    pending_effects: [PendingEffect],
    waiting_reason:
        "retryable_evidence_gap" |
        "operational_block" |
        "integrity_block" |
        null,
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

`PublicOutput.value` is the reviewed JSON rendering defined by its certified public-output
contract. Event ids, artifact ids, transition inputs, non-public outputs, facts, observations,
evidence, audit chronology, source scope, and timestamps are absent.

### Replay request and response

```text
ReplayRequest =
    { mode: "verify" }
  | { mode: "reproduce" }
  | { mode: "compare_current" }
```

`compare_current` always self-attests the serving executable and catalog. A caller cannot name,
label, upload, or supply an executable.

```text
VerifiedReplay {
    mode: "verify",
    run_id,
    result: "verified",
    journal_head,
    semantic_head,
    public_status: "active" | "succeeded" | "failed",
    fact_selections: [
        {
            transition_ref,
            completeness: "same_store_verified" | "unverified",
        }
    ],
}

ReproductionReplay {
    mode: "reproduce",
    run_id,
    recorded_history: "verified",
    result:
        { status: "matched" }
      | {
            status: "mismatch",
            transition_ref: transition_ref | null,
            stage,
            safe_diagnostic,
        }
      | {
            status: "unavailable",
            reason,
        },
}

CandidateReplay {
    mode: "compare_current",
    run_id,
    recorded_history: "verified",
    admitted_executable_identity_ref,
    candidate_executable_identity_ref,
    candidate_planning_profile_ref,
    candidate_planner_contract_ref,
    candidate_planner_implementation_ref,
    candidate_state_implementation_manifest_ref,
    candidate_capability_binding_manifest_ref,
    plan: "agrees" | "differs" | "not_comparable",
    transitions: [
        {
            transition_ref,
            result: "agrees" | "differs" | "not_comparable",
        }
    ],
}
```

All replay modes perform zero live semantic-capability, provider, executor, domain filesystem, or
signer IO and append nothing. Invalid recorded history fails before reproduction or comparison.

### Transition trace request and response

The exact query parameters are `cursor` and `limit`. The default limit is `100`; the maximum is
`500`. The first page fixes the current journal head. The opaque cursor binds that head and the
next record coordinate but carries no access authority. Every page independently requires
`InspectTrace`.

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
    transition_ref,
    commit: Head,
    transition_kind:
        "pure_settled" |
        "read_settled" |
        "effect_requested" |
        "effect_settled" |
        "dependency_skipped",
    node_id,
    occurrence,
    before: StatePosition,
    inputs: [NamedTraceInput],
    request: RetainedValue | null,
    consumed_observation: RetainedValue | null,
    result: RetainedValue | null,
    outputs: [NamedRetainedValue],
    facts: [NamedRetainedValue],
    evidence: [NamedRetainedValue],
    terminal_outcome: "succeeded" | "failed" | "skipped" | null,
    after: StatePosition,
    closure_ref: closure_ref | null,
}

StatePosition {
    state_digest,
    run_phase: "open" | "closed",
    node_phase: "unstarted" | "awaiting_effect" | "terminal",
}

NamedTraceInput {
    name,
    lineage,
    value: RetainedValue | null,
}

NamedRetainedValue {
    name,
    value: RetainedValue,
}

RetainedValue {
    schema_id,
    digest,
    media_type,
    content: AnnexEncodedValue,
}
```

`AnnexEncodedValue` is a closed tagged representation frozen by the canonical schema annex; it is
not an arbitrary serialization fallback. Input lineage is a closed root/config/seed,
same-run-transition, or cross-run-source reference. A denied cross-run source exposes only:

```text
CrossRunRedactedLineage {
    kind: "cross_run_redacted",
    source_ref_digest,
}
```

and has `value: null`. An authorized same-tenant source is dereferenced only after the separate
source-run `InspectTrace` decision. Every retained value is inlined through this enclosing reader;
there is no follow-up object endpoint.

### External-access audit request and response

Audit paging uses the same `cursor` and `limit` contract and fixes the first page's journal head.

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
    authorization_commit: Head,
    observation_commit: Head | null,
    capability_id,
    operation_id,
    request_identity,
    status:
        "authorized_unobserved" |
        "returned" |
        "did_not_enter" |
        "indeterminate",
    result_identity: result_identity | null,
    failure: SafeFailure | null,
    effect_key: effect_key | null,
    executor_frontier_ref: executor_frontier_ref | null,
    executor_frontier_sealed_terminal: boolean | null,
}

SafeFailure<Code, DiagnosticRef> {
    safe_failure_contract_ref: ContentRef,
    stable_code: Code,
    failure_class: FailureClass,
    boundary_stage: BoundaryStage,
    coarse_size_class: CoarseSizeClass | null,
    diagnostic_ref: DiagnosticRef | null,
}
```

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
| `response_invalid` | `Indeterminate` | `unrepresentable_response` | `boundary_observation` | `ResponseInvalid` |
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

Observed EVM source/chain mismatch and anchor drift, and Bitcoin source/network mismatch, anchor
drift, and `scantxoutset.success = false`, are bounded `Returned` typed semantic failures rather
than `SafeFailure` values. Bitcoin remains unregistered pending its repeat-work-safe provider
qualification.

Precise timestamps, source scope, provider routes, credentials, raw request or response bodies, raw
provider errors, executor vault references, and error source chains are absent.

### Portable export request and response

```text
ExportRequest =
    { kind: "semantic" }
  | { kind: "audit" }
```

The REST response is the raw `PortableRunExport.bytes`, not a base64 field inside the JSON success
envelope:

```text
PortableRunExport {
    media_type,
    digest,
    bytes,
}
```

The canonical schema annex freezes `media_type`, the byte-level bundle and manifest schema, and the
external digest-value encoding. The canonical bytes contain no digest of themselves in the
manifest or a nested member. Once the final bytes exist, the app computes
`SHA-256(exact final canonical bytes)`. That one external value is
`PortableRunExport.digest`; REST sets `Content-Type` from `media_type` and returns the same value
only in the exact `Mfm-Content-Digest` header. There is no separate manifest digest, bundle digest,
domain-hashed export identity, or compatibility checksum. The CLI requires:

```text
mfm run export RUN_ID --kind semantic|audit --output PATH
```

It atomically writes `bytes` to the explicit path and never renders the bundle through normal
JSON/text output.

A semantic export is fixed at the current semantic head, or the closure coordinate for a closed
run. An audit export is complete only as of its recorded journal head and records the greatest
executor frontier committed to the MFM journal for each effect at that head. The manifest records
the store scope, tenant scope, run id, export kind, exact coordinate, dependency-run identities,
and closure membership, but contains no digest of itself and grants no later access.

A missing dependency-run `Export` authority returns `SourceRunExportDenied`; the app does not emit
an unverifiable partial semantic bundle. Credentials and ACL state never appear in either export.
Offline verification of caller-held bytes requires no live credential and grants no store or
object access.

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

The cutover must add or replace tests that prove:

- each grant independently permits only its exact app, kernel, and store operation;
- missing and wrong-purpose, wrong-store, wrong-tenant, and wrong-run authorities fail;
- credential bytes cannot be cloned, serialized, formatted, logged, persisted, or included in
  public errors;
- requests cannot select tenant, and the same entry point/invocation under two tenants derives
  different run ids;
- invocation identity accepts only canonical UUIDv4, exact retry attaches, changed root conflicts,
  and an ambiguous acknowledgement is retried without a new identity;
- admission performs no live capability call and never drives;
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
  raw-byte SHA-256 value as both `PortableRunExport.digest` and `Mfm-Content-Digest`, bind exact
  semantic/audit coordinates, and do not overstate executor-frontier qualification;
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
- A mutation executor is registered only after its generic keyed ledger/backend and the separate
  deployment generation, stale-writer, destination-convergence, and resource fences qualify.
  PostgreSQL persistence or co-location does not satisfy those external authorities, and the MFM
  store writer/WAL fence cannot substitute for them.

These assumptions affect deployment readiness, not the logical app or transport schema.

## Material uncertainties

None. The canonical annex and shared golden-vector corpus remain required artifacts before schema
freeze, but the ownership and logical choices they must encode are fixed above. The rollout
assumptions block deployment or capability registration rather than opening an alternate app,
CLI, REST, digest, reference, or executor contract.
