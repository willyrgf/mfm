# mfm-app

`mfm-app` is the purpose-authorized application boundary for recoverability v1. Process
transports receive one opaque `Application`; journal storage, authority issuance, planning,
certification, runtime catalogs, replay readers, and live capabilities remain private to
application composition.

The frozen contract is documented in
[`docs/recoverability-app-surface-v1.md`](../../docs/recoverability-app-surface-v1.md) and encoded
by `contracts/recoverability/v1/annex.json`. There are no compatibility run, fact, stream, manual
resolution, or arbitrary object APIs.

## Public run facade

The complete run-facing surface is:

```text
application.entry_points() -> &[EntryPointContract]
Application::admit_run(credential, request)
Application::drive_once(credential, run_id)
Application::read_public_run(credential, run_id)
Application::replay_run(credential, run_id, request)
Application::read_transition_trace(credential, run_id, page)
Application::read_access_audit(credential, run_id, page)
Application::export_run(credential, run_id, request)
```

Production bootstrap self-attests the executable, seals the qualified registry, and builds the
published contracts once. Each `Application` caches that immutable entry-point slice;
`entry_points` performs no I/O and no authorization. Every other method consumes a
`SecretCredential`, calls the injected `RunAccessPolicy` afresh, derives the tenant only from that
decision, and mints one store-bound, purpose-specific authority. Credentials are zeroized and are
not cloneable, serializable, or formattable; empty values and values larger than 64 KiB are
rejected at this boundary.

V1 publishes exactly `mfm.portfolio/snapshot@1`. Admission accepts a caller-generated canonical
UUIDv4 invocation identity and `{ "target": "..." }`. It resolves current configuration,
deterministically plans and certifies the graph, self-attests the serving executable, retains the
root proof objects, and appends only `RunAdmitted`. It performs no semantic capability call and
does not drive the run.

`drive_once` advances at most one transition or audited protocol operation. The sole ordinary read
is `PublicRunView`: production performs one purpose-authorized `store.read_public_run` call and
converts only its sealed `VerifiedPublicRunView` projection. It does not load a raw journal or
assemble status and outputs through separate readers. Privileged trace and audit readers inline
reviewed retained values. Replay verification is callback-free. Production exact reproduction is
deliberately `unavailable` in this cutover; no historical resolver or sandbox is composed, and
there is never a live-runtime fallback. Reproduction and current-candidate comparison accept only
caller-held semantic export bytes bounded to 16,777,216 bytes with their exact `ContentRef`,
require a separate same-run `Export` decision, and verify the complete store/tenant/run/head/closure
binding before any callback. The REST base64url representation is bounded to 22,369,622
characters. Replay never generates or fetches a replacement bundle. Portable export contains the
complete authorized proof closure and returns exact canonical bytes plus the raw-byte digest.

Malformed, noncanonical, missing, forbidden, digest-mismatched, or run-binding-mismatched caller
artifacts return `ReplayArtifactInvalid` with `The replay artifact is invalid.` Any encoded,
decoded, sidecar, or complete replay-body size violation returns `ReplayArtifactTooLarge` with
`The replay artifact exceeds the allowed size.` Authenticated store or recorded-history integrity
failures remain the distinct internal `ReplayVerificationFailed` contract.
Candidate recorded-history, callback/certification, and comparison-integrity failures use that
same fixed 500 response. Absence of the sealed current candidate uses
`RuntimeCatalogUnavailable` with `The exact admitted runtime catalog is unavailable`; absence of
an exact historical executable remains the successful canonical `unavailable` reproduction
result.

Cross-run trace and export dependencies are independently authorized with the same borrowed
credential. Export policy denial or a wrong-tenant dependency uses the redacted dependency-denial
contract. Once that exact dependency is authorized, absence of a run named by verified append-only
history is an integrity failure and uses `ReplayVerificationFailed`. Trace renders a denied or
wrong-tenant source as redacted lineage, and renders an authorized-but-absent source identically,
without disclosing another tenant's run. A fresh source decision of `AuthenticationRequired`
instead fails the request.

## Production composition

`connect_production_application` requires:

- a deployment-supplied `Arc<dyn RunAccessPolicy>`;
- a deployment-supplied `AuthoritativeWriterFence`;
- an authoritative PostgreSQL store;
- exact current executable, planning, state, and capability identities; and
- explicit runtime configuration for any live EVM routes.

Every configured EVM route is part of the qualified aggregate routing catalog and is validated
before the application accepts traffic. Each route requires `source_ref`, a positive `chain_id`,
an operator-controlled stable `generation_id`, and `rpc_url`; `auth_header` remains optional and
indirect-only. Endpoint or credential rotation requires a new generation id. The legacy
`expected_chain_id` field remains forbidden.

Bootstrap assembles one exact qualified support graph with `52 + N` members, where `N` is the
configured generation count in `1..=4096`. The fixed closure contains the executable-bound
12-component qualification; the aggregate catalog, every generation, reviewed source scope,
failure contract, classifier, and read binding; framework unit configuration and portfolio
routing; state and capability manifests; and the EVM balance fact descriptor and evidence
contracts. The graph scope is derived from the complete field-path-ordered member identities and
contracts, so any executable, route, or support-contract change selects a new scope. The scope
preimage is not itself retained.

The app admits that graph once and moves the resulting non-cloneable authority into one
`QualifiedProgramRegistry`. The private application backend and runtime share only the same
registry `Arc`; neither constructs a second registry or support graph. Endpoint, authorization,
transport, configured portfolio, per-run input, certificate, fact, and output material are not
support members.

The writer fence is not a boolean or command-line switch. It is a deployment-owned proof that this
process is the sole authoritative writer for the store lineage. Library callers must pass it
through unchanged to `mfm-storage-postgres::open_authoritative`.

`Application::check_ready` performs only one bounded PostgreSQL writable-lineage probe against
that already opened authoritative store. It does not resolve DNS, call an EVM/provider endpoint,
execute a semantic callback, or requalify the process; the sealed bootstrap capability is
sufficient. The REST adapter maps every probe failure to its one fixed `503 NotReady` response.

The repository's standalone CLI and REST binaries do not own such a fence and therefore fail
closed with `AuthoritativeWriterFenceUnavailable` for every application-bound operation,
including entry-point discovery. A deployment embeds the transport library and injects its
qualified application composition.

Configured values are provisioned by deployment/migration ownership outside the runtime app role.
The application can resolve only the exact tenant, entry-point, and target binding authorized by
an admission authority; it exposes no target-only publication, listing, or export surface.

## Other app services

Keystore import, public metadata/listing, deletion, and the existing standalone transaction
signing library service remain outside the run facade. No CLI or REST route exposes direct
signing. EVM mutation stays unregistered until the separate wallet-executor qualification
commit replaces that library seam with generation-guarded signing after durable effect and
resource authorization.

Secret key material is consumed through zeroizing inputs and is never persisted in manifests,
journal records, retained objects, public errors, or transport outputs.
