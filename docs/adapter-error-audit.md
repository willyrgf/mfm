# Adapter error-chain audit

This inventory follows the current-state auditability cutover. It distinguishes the completed
Part 1 boundaries below from remaining source-owner gaps. Only the producers explicitly selected
in [Part 2](../RFC_CAUSAL_ERROR_PRESERVATION.md) belong to its separate implementation scope. This inventory does
not claim that preserving an error received by Runtime repairs a cause discarded upstream.

Part 1 is accepted at production `87f198a9`, recorded in `0a7543ec`. Part 2 refinement selects
three remaining cutovers: run Store, execution authority, and executing keystore signing. Its
sections 2–6 freeze producers, fields and stopping boundaries. The findings and remediation ideas
below distinguish implemented R1–R3 from excluded source-owner gaps; they do not expand those cutovers.

## Result

Runtime commits declared originals before recovery and accepted Effect settlement before
interpretation. Native callback, checked Object admission, recording, and reporting failures
retain reviewed causes through Application and the run transport surfaces. Journal seals opaque
frames and Store loads admission/latest/optional-probe snapshots; neither reconstructs run semantics.

The repository still has first-loss gaps in excluded PostgreSQL, keystore startup/import, general
cryptographic construction, bootstrap, and non-run transport paths. These are not Part 1 completion
conditions; bootstrap and non-run transport enrichment are outside both RFCs. Core acceptance
and its limits are recorded in the
[core evidence](measurements/auditability-core-acceptance.md).

The requirements are in [AGENTS.md](../AGENTS.md) and
[code quality](code-quality.md#error-provenance-and-auditability). Classification and public codes
are projections, while unavailable, withheld, or bounded evidence must be explicit.

## Scope and method

The review traced source construction, conversion and downstream observation for every first-party
outbound adapter family found in the repository:

- EVM chain identity, anchor, balance and anchored contract-call Reads, sharing the JSON-RPC client.
- EVM nonce reservation, transaction preparation and execution adapters, including their provider,
  transaction-authority and signer boundaries.
- PostgreSQL run Store, run index, configuration repository, transaction custody, connection gates
  and schema provisioning.
- Keystore signer/thread/channel bridge and reusable signing primitives.
- In-memory Store, run index and configuration implementations.

Adjacent deployment filesystem/environment IO, CLI input/output, REST listener/request handling,
Application translations and Runtime/Journal error boundaries were also inspected because they
can discard a cause after a concrete adapter returns it. They are listed separately rather than
misidentified as executable State adapters. Third-party client internals and arbitrary downstream
implementations of public traits were not audited.

Searches included source-erasing `map_err`, generic catch-all helpers, protocol error envelopes,
commit handling, typed source wrappers and error-to-absence conversions. Findings below distinguish
real loss from deliberate safe rendering and expected protocol absence. No provider requests,
database operations or real secret-bearing diagnostic dumps were needed for this review.

## Findings by boundary

### 1. Shared EVM HTTP/JSON-RPC ingress: reviewed provenance retained

Sources: [transport](../crates/live/evm/src/json_rpc.rs),
[client capture](../crates/live/evm/src/json_rpc/capture.rs), and
[domain carrier](../crates/domains/evm/src/provider_failure.rs).

| Original first-loss boundary | Replacement | Consuming evidence |
| --- | --- | --- |
| Send/body-to-unit mapper | Method/stage, exposed source messages and ordered ancestry, OS kind/code | `send_failure_retains_exposed_os_ancestry`, `body_deadline_retains_headers_and_parser_location_is_reviewed` |
| HTTP status collapse | Received status retained separately from ancestry | `response_status_survives_body_failure_and_size_refusal`, existing 429/deadline test |
| RPC envelope disposal | Strict envelope with present-null distinction, retained numeric code, message and original data text | `rpc_codes_remain_distinct_in_committed_and_cold_domain_failures` |
| Untagged decoding fallback | One raw-field envelope admission followed by typed decoding; parser category/location captured at the failing stage | `only_complete_exact_rpc_error_objects_parse`, parser-location regression |
| Body bound collapse | Exact declared size or observed streamed lower bound, inclusive limit and Body stage | `response_status_survives_body_failure_and_size_refusal`, `streamed_body_overflow_records_a_lower_bound_without_an_invented_source` |
| Checked result collapse | Result field, numeric range, ABI size, receipt shape and transaction mismatch facts | `local_range_failure_retains_field_and_checked_observation`, receipt/submission tests |

Tests above are in [JSON-RPC tests](../crates/live/evm/src/json_rpc_tests.rs).
The [Values diagnostic tests](../crates/kernel/values/tests/diagnostic_contract.rs) cover whole-owner
text/numeric admission and invocation conversion failures. The transport tests cover ordered and
cyclic exposed sources, native fields, and hot/cold originals and reports. The
[consuming domain contract test](../crates/domains/evm/tests/provider_failure_contract.rs) checks
full-width owner facts and diagnostic text beyond the deleted 8 KiB quota.


`source_cycle: true` means exactly “traversal stopped on a repeated interface pointer.” The local
source walker compares complete `dyn Error` pointers with `std::ptr::eq`; it does not establish
concrete-object identity. An inline child may share its parent's data address, and one concrete
error can have different interface representations. The latter may produce repeated cause entries
before termination; no exact cyclic-object visit count is promised across compiler configurations.
There is no identity registry, message comparison or diagnostic budget.

Unknown client sources retain their exposed messages and deeper links. RPC data retains its raw
JSON spelling as text; no extra body read is introduced. These selected source APIs do not expose
hidden client attempts or arbitrary foreign-library state. Dependency text follows the diagnostic
profile; MFM does not deliberately append its own secrets or full request objects.
Classification is unchanged; timeouts do not establish nonacceptance. Cold Read tests retain the admitted provider carrier. The separate Runtime recovery tests prove
that the original commits before classification or handler entry.

### 2. Read registration bridges preserve the cause they receive

Source: [live EVM registration](../crates/live/evm/src/lib.rs), `read` and `read_anchored`.

The chain/anchor/balance and anchored-call bridges pass provider errors through without replacing
them. They now carry the provider's reviewed causal value without a second persistence object.

Wrong local route, chain or capability family returns a native `AdapterFailure` with reviewed
expected/observed binding or subject facts. It stays Internal and prevents provider entry, without
manufacturing authenticated integrity evidence or a recoverable provider error.

### 3. Transaction adapters preserve selected provider, authority and signing causes

Source: [transaction.rs](../crates/live/evm/src/transaction.rs), `reserve_nonce`,
`prepare_transaction`, `execute_transaction`, `map_provider_error` and `map_authority_error`.

- `map_provider_error` retains the complete reviewed provider carrier alongside the originating
  chain-verification, nonce-observation, receipt, canonical-block or submission operation. The
  transaction classification remains OutcomeUnknown.
- R2 `map_authority_error` moves unavailable evidence into the v3 durable transaction owner and
  forwards internal InvocationDiagnostic unchanged. No receiver recapture or authority serializer remains.
- Signer Invalid/Failed now enter SignerUnavailable with their actual unit kind and sign/signer
  context. SignFailed evidence moves unchanged from executing keystore/channel/primitive producers.
- Codec, checked receipt/binding, and reached blocking-task failures now use native `AdapterFailure`
  with reviewed causes or task outcomes. Excluded checked primitive APIs can still expose only
  their unit result; this cutover does not recover earlier layers or authorize panic payloads.

Retained contract: preserve selected provider/custody/signer causes with operation context at
the owning boundary. Keep the same prepared command/EffectId, no renonce or replacement command, and
the existing operational-versus-invariant distinction. Classification remains independent of the
diagnostic payload. Avoid a separate generic exception framework for each transaction stage.

### 4. PostgreSQL run Store retains selected producing causes (R1)

[Run storage](../crates/storages/postgres/src/lib.rs) retains mandatory diagnostic data on
Unavailable, CorruptPhysicalState and Indeterminate. The private
[SQLx recipe](../crates/storages/postgres/src/diagnostic.rs) captures operation/stage, ordered
messages, SQLx variants, selected column facts, PostgreSQL SQLSTATE/severity/server message,
detail/hint/schema/table/column/constraint, and IO kind/code. It follows contained database,
boxed and IO children before ordinary source traversal, stopping on repeated full interface
pointers under the accepted information limit. Unknown native fields are not reconstructed.

Run load/append and row-decoding dispositions remain unchanged. A failed insert_frame/update_head
retains its already attempted rollback failure separately; rollback success adds no error.
Allocation/runtime-handle/task failures and local physical/identity checks retain their selected
facts. No SQL arguments, statement copies, connection objects or panic payloads are appended.
Dependency text follows the trust contract, without scans, certification or diagnostic quotas.

Run Store errors remain invocation-only and cannot prove they were recorded through the failed
Store. Comparison-only/local-rejection rollback, background cleanup, connection gates,
configuration and index enrichment remain outside this selected guarantee.

### 5. PostgreSQL execution authority retains selected causes (R2)

[Authority](../crates/storages/postgres/src/evm_tx.rs) uses the private PostgreSQL SQLx recipe for
load, reserve_or_compare and retain_prepared. Static operation/stage fields distinguish SQL calls;
ambiguous COMMIT remains Unavailable/OutcomeUnknown. The input-discarding unavailable/internal
helpers and AuthorityError's serializer are deleted. Local retained facts and selected identity,
domain, scalar and byte-length errors become one authority_internal invocation diagnostic, then
Live forwards it unchanged without append. No rejected bytes, signed wire or connection data is added.

AuthorityUnavailable and SignerUnavailable now carry mandatory DiagnosticEvidence in the v3
transaction error owner. Typed classification remains OutcomeUnknown/Retryable respectively.
Restoration reconstructs that owner, not a live SQLx error. Pool/gate failures stop at SQLx's actual
returned error, including existing Protocol markers; excluded bootstrap losses are not recovered.

### 6. PostgreSQL configuration and run-index adapters also flatten sources

Sources: [config.rs](../crates/storages/postgres/src/config.rs), `classify_revision_query` and
commit handling; [index.rs](../crates/storages/postgres/src/index.rs), `classify_index_query`.

SQLx decode categories are recognized then replaced with Corrupt; remaining failures become
Unavailable. Configuration COMMIT preserves only the definite/indeterminate category. Checked
retained-row parsing additionally drops which identity or field failed.

Remediation: preserve the same reviewed SQLx cause vocabulary as the run Store, with the relevant
config/index operation stage. Keep those ports independent from Runtime and keep configuration
immutability/exact delete and run-index pagination semantics unchanged. Diagnostic reuse should
remove duplicated source-loss mappers, not merge distinct storage responsibilities.

### 7. PostgreSQL bootstrap and provisioning discard both sources and failed checks

Sources: [postgres/lib.rs](../crates/storages/postgres/src/lib.rs), `GateError`,
`classify_open_error`, `classify_catalog_query`; [catalog.rs](../crates/storages/postgres/src/catalog.rs);
[provision.rs](../crates/storages/postgres/src/provision.rs); [locator.rs](../crates/storages/postgres/src/locator.rs).

Connect/query errors collapse into Unavailable. Incompatible schema, role, epoch or durability
checks collapse into Incompatible. The pool callback turns a gate failure into a Protocol marker;
`classify_open_error` recognizes a marker substring and discards everything else. Provisioning
retains the COMMIT ambiguity category but not its source.

Remediation: retain a reviewed check identifier and originating cause without exposing locator
values, passwords, SQL parameters or arbitrary database detail. Bootstrap/provisioning can fail
before any run or usable Store exists; do not promise their errors already have a Journal record.
Replace string-marker routing only as part of a concrete pool/open error cutover that preserves the
gate contract, not by adding an additional shadow connection registry.

### 8. Executing keystore signing retains selected producing causes (R3)

Sources: [keystore/lib.rs](../crates/keystore/src/lib.rs), `KeystoreSigner::sign` and private
`Keystore::sign`; [signing/lib.rs](../crates/signing/src/lib.rs).

Command::Sign replies now carry SigningError directly, deleting the KeystoreError roundtrip.
SignFailed retains operation sign and distinct request_send/reply_receive channel messages,
key_lookup/missing_key local facts, or sign_prehash_recoverable/signature_error primitive text.
The pinned primitive exposes no child source. Checked compact-signature conversion passes its
concrete result unchanged. Live moves supplied evidence into the durable SignerUnavailable owner.
The old blanket unavailable-detail serializer is removed; Invalid/Failed remain honest unit kinds.

Remaining excluded gaps: owner startup, import/shutdown and general checked cryptographic
construction/recovery retain their existing reduced errors. Remediation belongs at those producing
APIs in a separate cutover; executing sign enrichment cannot reconstruct earlier discarded causes.
Never retain SendError<Command>, panic payloads or secret-bearing objects. Real request/reply closure,
missing-slot and primitive tests accompany valid-signing and thread-affine custody regressions.

### 9. In-memory backends have fewer source layers but are not universal exemptions

Sources: [MemoryStore](../crates/kernel/store/src/lib.rs), allocation/ownership helpers and
blocking joins; [memory configuration](../crates/config/src/memory.rs).

R1 MemoryStore retains allocation messages/requested bytes, runtime-handle messages, task
cancelled/panicked facts and local physical-check reasons on its existing dispositions. Numeric
capacity errors are unchanged. RunIndex enrichment remains excluded. The
in-memory configuration backend has no network/client error chain to preserve; ordinary absent or
unchanged outcomes are not dropped errors.

Remediation: retain actual available memory/task causes and local stage without fabricating an
RPC/SQL chain. Preserve mechanical admission/latest/optional-probe loading and atomic append responsibilities. A source
API that exposes no details should be represented honestly as such.

### 10. Adjacent bootstrap and transport conversions lose additional provenance

Sources: [deployment.rs](../crates/app/src/deployment.rs), `Deployment::load` and parsing;
[app/lib.rs](../crates/app/src/lib.rs), `Application::open`, repository/index error maps and RunId
generation; [CLI](../bin/cli/src/main.rs), bounded input/output; [REST main](../bin/rest-api/src/main.rs)
and [REST server](../bin/rest-api/src/server.rs).

Deployment file-open, read, UTF-8, TOML and task failures share one code. Composition then flattens
locator/provider/Postgres errors again. Config and index translation keep only client-facing
categories. CLI input and REST listener errors discard OS causes; request rejection
rendering intentionally returns reviewed messages without an independent causal audit path.
Entropy failure loses the underlying OS/library category too. Run-response preparation, encoding,
and CLI write/flush failures now retain their original invocation and separate reporting cause.
REST retains response-construction failures through its body handoff; this is not proof of delivery
through a socket or an independent durable audit sink.

Remediation: retain causal invocation diagnostics while keeping the current redacted user-facing
codes. File paths, environment values, configuration snippets and request bodies need explicit
disclosure rules; line/column and operation names are useful without copying those contents.
Do not add adapter recovery logic to binaries. A disconnected response channel is not evidence
that no operation committed, and failure to print an error must not overwrite the primary cause.

### 11. Runtime and Journal preserve admitted causes, not erased upstream stacks

Sources: [capability errors](../crates/kernel/capabilities/src/lib.rs),
[Runtime errors](../crates/kernel/runtime/src/lib.rs),
[Runtime recovery association](../crates/kernel/runtime/src/assembly/recovery.rs),
[engine](../crates/kernel/runtime/src/engine.rs), [reports](../crates/kernel/runtime/src/report.rs),
[Journal](../crates/kernel/journal/src/lib.rs), [public view](../crates/app/src/run_view.rs).

Read, State-domain and pending Effect failures commit their original and complete operation facts
as AwaitingRecovery before classification, policy or mapping. Recovery is a separate commit.
Accepted Effect settlement commits as AwaitingInterpretation before deterministic interpretation.
Cold reads inspect those current facts without replaying earlier callbacks or operation frames.

`AdapterError<E>` retains either its typed operational value or Values-owned InvocationDiagnostic.
Concrete owners select fields once; Runtime adds execution operation/stage and consumers forward
immutable data without reopening a native error. Checked Object Deserialize and record decoding
retain parser category, location and reason. Nested constructor ancestry/size is outside that stored
route; direct construction and postdecode slot errors retain structured facts. Journal retains its
concrete canonical/JSON sources while owning only the opaque frame wire.

Failed first-original encoding reports known position/contract and unavailable original contents/
identity with the encoding cause. It retries no serializer and appends nothing. Once admitted, the
same Failure/Object serves recording and failed-append reporting. BeforeAppend holds that admitted
Failure and a boxed RuntimeError; Store and NotInserted retain submitted candidate identity and
separate concrete causes. Store errors cause no probe. NotInserted permits one snapshot probe;
its finding survives independent restoration/view failure. Only known insertion followed by failed
projection carries an acknowledged head.

App and transports borrow existing results through one final presentation. Invocation field
conversion has exactly two fallback markers, encoding_failed and panicked, while retaining supplied
code/operation/primary size. No native projector, diagnostic quota or generic reporting tree remains.
These changes do not repair layers already erased by remaining PostgreSQL, signer or other source
owners. The [core evidence](measurements/auditability-core-acceptance.md) tracks consuming verification.

### 12. Development-only funding shares the reviewed RPC boundary

The E2E's separate `funding_rpc`, `FundingResponse`, and `funding_response_body` were deleted.
`JsonRpcEvmProvider::fund_development_sender` performs unlocked-account discovery and one submission
through the same causal decoder, preserving its original 16 KiB response ceiling. The helper's
`FundingError` now retains build or provider causes. The
[funding regression](../crates/live/evm/src/json_rpc_tests.rs) checks exact requests, amount and fee
encoding, RPC code retention and secret exclusion. The
[funding limitations](known-gaps.md#development-node-funding) still apply: a returned hash is not
funded readiness, and lost acknowledgement never authorizes blind resubmission.

## Preserve the existing successful design choices

- Exact typed operational payloads survive Runtime/Journal once admitted.
- Classification does not replace the payload and cold reconstruction uses committed decisions.
- Provider wrapping in the transaction adapter already retains the supplied nested cause.
- Store/config/provisioning retain the critical definite-versus-indeterminate distinction.
- Read/Effect invariant failures do not manufacture authenticated external evidence.
- Expected receipt absence, valid missing-token-interface outcomes and explicit Pending have
  protocol meanings. Do not indiscriminately convert these into failures to increase log volume.
- Public redacted formatting is appropriate; silent loss of the audit cause behind it is the gap.

## Deferred work and verification

The remaining inventory identifies first-loss sites, not authorization for a repository-wide
migration. Part 1 implements only its E1–E6/shared-data recipes; the separately refined Part 2
selects additional producers. Follow those RFC contracts and current code-quality policy rather
than the superseded generic remediation directions from this audit. In particular, selected
upstream diagnostic text follows the explicit trust boundary, not a generic sanitizer or omission
ledger. Expected absence and Pending remain protocol outcomes, not invented errors.

Use [build and verification](build-and-verification.md) for scope-selected tests. The
[Part 1 acceptance packet](measurements/auditability-core-acceptance.md) records its corrected core,
focused evidence, G1 disposition and final CI. Future owner changes require their own affected
verification; they neither block nor supply deletion credit for Part 1.

## Material uncertainties

None for the Part 1 boundary. Remaining upstream evidence availability and producer designs belong
to the separate Part 2 refinement gate. A dependency may discard details before MFM receives them;
its selected owner must establish what is actually available. No inventory entry establishes
post-crash delivery or proves that a failed persistence operation was durably audited.
