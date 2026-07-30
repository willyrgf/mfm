# mfm-app

`mfm-app` is the purpose-authorized application boundary for recoverability v3. Process
transports receive one opaque `Application`; journal storage, authority issuance, planning,
certification, runtime catalogs, replay readers, and live capabilities remain private to
application composition.

The frozen contract is documented in
[`docs/recoverability-app-surface-v3.md`](../../docs/recoverability-app-surface-v3.md) and encoded
by `contracts/recoverability/v3/annex.json`. There are no compatibility run, fact, stream, manual
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

Recoverability v3 publishes exactly `mfm.portfolio/snapshot@1` and
`mfm.evm/submit-transaction@1`. Admission accepts a caller-generated canonical UUIDv4 invocation
identity and `{ "target": "..." }`. Portfolio targets resolve a `PortfolioConfig`; transaction
targets resolve an immutable `EvmSubmitTransactionRequest` whose tenant and target must match the
authorized admission and selector. Admission deterministically plans and certifies the selected
graph, self-attests the serving executable, retains the root proof objects, and submits one opaque
owned `AuthorizedAdmissionPlan` to `Runtime::admit`. Runtime alone prepares and appends
`RunAdmitted`. Admission performs no semantic capability, signer, executor, or RPC call and does
not drive the run.

`drive_once` advances at most one transition or audited protocol operation. The sole ordinary read
is `PublicRunView`: production performs one purpose-authorized
`RunHistoryReader::read_public_run` call and
converts only its sealed `VerifiedPublicRunView` projection. It does not load a raw journal or
assemble status and outputs through separate readers. Privileged trace and audit readers inline
reviewed retained values. Audit projects closed `NonDomainFailure` only through its
`non_domain_failure` field; it never reclassifies it as safe or domain evidence. Replay
verification is callback-free. Production exact reproduction is
deliberately `unavailable` in this cutover; no historical resolver or sandbox is composed, and
there is never a live-runtime fallback. Reproduction and current-candidate comparison accept one
affine caller-held semantic export reader with its exact stream `ContentRef`, require a separate
same-run `Export` decision before polling it, and verify the complete
store/tenant/run/head/closure binding before any callback. Framing, frames, and decoded chunks are
individually bounded, while total frames, sources, objects, authorities, bytes, steps, and elapsed
time have no validity ceiling. Replay never generates or fetches a replacement stream. Portable
export contains the complete authorized proof closure and returns one private-spool-backed affine
reader plus its external raw-byte identity.

Malformed, noncanonical, missing, forbidden, digest-mismatched, or run-binding-mismatched caller
streams return `ReplayArtifactInvalid` with `The replay artifact is invalid.` A CLI `ContentRef`
sidecar over 4,096 bytes returns `ReplayArtifactTooLarge` with
`The replay artifact exceeds the allowed size.` Export-stream I/O failures return only the fixed
internal `ExportStreamIoFailed` contract. Authenticated store or recorded-history integrity
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
- a deployment-supplied run-journal `AuthoritativeWriterFence`;
- an authoritative PostgreSQL store;
- an `EvmWalletDeployment` containing the exact executor contract/deployment/resource owner,
  selected route-generation reference, content-addressed initial-nonce descriptor, verified
  guarded signer binding, generation guard, and dedicated PostgreSQL executor pool;
- a separate deployment-supplied `ExecutorWriterGenerationFence`; and
- exact current executable, planning, state, and capability identities; and
- explicit runtime configuration for any live EVM routes.

Every configured EVM route is part of the qualified aggregate routing catalog and is validated
before the application accepts traffic. Each route requires `source_ref`, a positive `chain_id`,
an operator-controlled stable `generation_id`, and `rpc_url`; `auth_header` remains optional and
indirect-only. Endpoint or credential rotation requires a new generation id. The legacy
`expected_chain_id` field remains forbidden.

The runtime configuration also resolves the one process-local signer selected by the deployment's
public `SignerRef`. Its keystore and unlock paths remain indirect runtime values. They are never
support members, configured transaction bytes, retained evidence, or public diagnostics. The
verified signer binding fixes the keystore implementation, secp256k1 recoverable algorithm,
RFC6979 low-s profile, expected account, durable executor generation, destination-fence
attestation, and direct-sign exclusion proof. Qualification derives its public signer-descriptor
identity; deployment cannot supply a separate signer-binding reference.

For TOML, the selected process-local entries have this strict shape:

```toml
[signers.wallet]
provider = "keystore"
keystore_ref = "primary"
entry_id = "67e55044-10b1-426f-9247-bb680e5fe0c8"

[keystores.primary]
keystore_path = { direct = "/run/mfm/wallet.keystore" }
unlock_file_path = { direct = "/run/mfm/wallet.unlock" }
```

`signer_binding.signer_ref()` selects `signers.wallet` in this example. Each path supports the
reviewed runtime value-source forms; the unlock value itself belongs in the referenced protected
file, never in configuration.

Bootstrap assembles one exact qualified support graph with `68 + N` members, where `N` is the
configured generation count in `1..=4096`. The fixed closure contains the executable-bound
14-component qualification; the aggregate catalog, every generation, reviewed source scope,
failure contract, classifier, and read binding; the exact wallet executor contract,
implementation, deployment, resource ownership, target callback surface, and verified executor
binding; the derived guarded-signer descriptor, nonce policy, initial-nonce descriptor,
already-known classifier, finality policy, assurance policy, and sealed wallet-request
qualification; framework unit configuration and product routing; state and capability manifests;
and the EVM balance fact descriptor and evidence contracts. The live EVM closure is exactly
`15 + N`; the seven wallet leaves raise the product closure from `61 + N` to `68 + N`. The graph
scope is derived from the complete field-path-ordered member identities and contracts, so any
executable, route, executor, resource-owner, wallet-policy, or support-contract change selects a
new scope. The scope preimage is not itself retained.

The app admits that graph once through the pre-split `QualifiedRunStore` and moves the resulting
non-cloneable authority into one `QualifiedProgramRegistry`. It then consumes the assembly into
the writer owned by one shared `Arc<Runtime<_>>` and the cloneable reader retained for public,
inspection, replay, export, and readiness services. The app retains no run-history writer,
prepared append, raw backend, or PostgreSQL pool. The private application admission backend and
wallet executor also
share the one live-owned `Arc<EvmWalletRequestQualification>` created before that admission; they
cannot reconstruct or weaken its predicate. A configured wallet request must match it before
certification or any journal append, and the executor rechecks it before effect binding or nonce
allocation. Endpoint, authorization, transport, configured portfolio, per-run input, certificate,
fact, and output material are not support members.

Neither writer fence is a boolean or command-line switch. The run-store fence proves that this
process is the sole authoritative writer for the journal lineage. The executor fence independently
proves the exact tenant, executor binding, durable ledger generation, database/schema lineage, and
stale/sibling-writer exclusion required before wallet target entry. Library callers pass them
unchanged to their respective storage open functions; one can never substitute for the other.

`Application::check_ready` performs bounded writable-lineage probes against the already opened run
store and executor ledger/fence. It does not open a signer, read an unlock file, resolve DNS, call
an EVM/provider endpoint, execute a semantic callback, or requalify the process; the sealed
bootstrap capabilities are sufficient. The REST adapter maps every probe failure to its one fixed
`503 NotReady` response.

The repository's standalone CLI and REST binaries do not own the run-store fence or the wallet
deployment/executor fence and therefore fail closed with
`AuthoritativeWriterFenceUnavailable` before application composition for every application-bound
operation, including entry-point discovery. A deployment embeds the transport library and injects
its qualified application composition.

Configured values are provisioned by deployment/migration ownership outside the runtime app role.
The application can resolve only the exact tenant, entry-point, and target binding authorized by
an admission authority; it exposes no target-only publication, listing, or export surface.

## Other app services

Keystore import, public metadata/listing, and deletion remain standalone app services. Transaction
signing is not a standalone service: the qualified wallet executor alone may bind the
generation-guarded signer after durable effect and resource authorization. Secret key material is
consumed through zeroizing inputs and is never persisted in manifests, journal records, retained
objects, public errors, or transport outputs.
