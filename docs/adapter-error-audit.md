# Adapter error-chain audit

Reviewed: 2026-09-10, following the intrinsic-classification and pending-Effect audit cutover
`e71d744f`. This is a source review and remediation inventory, not a claim that the gaps below have
been fixed. The review also inspected the current shared working tree; unrelated Runtime edits
were left untouched. Adapter source anchors below identify the reviewed conversion points.

## Result

The repository does not yet preserve the whole causal error chain across every adapter.
The RPC/EVM cutover now retains reviewed provider evidence through the existing admitted failure
path. SQLx, signer, custody and application boundaries still have the gaps below, and Runtime's
execution/context and recovery commit cutover remains outstanding. Adding durable pending failure frames fixed
one persistence gap; it could not recover evidence discarded before Runtime entry.

The new repository requirement is in [AGENTS.md](../AGENTS.md) and
[code quality](code-quality.md#error-provenance-and-auditability). Preserve causal provenance across
every boundary; classification and public rendering must be projections. Preserve secret-free
contracts and explicitly identify any missing evidence instead of calling partial capture complete.

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
| Send/body-to-unit mapper | Method/stage, bounded exposed source prefix, OS kind/code and explicit URL/source-detail omissions | `send_failure_retains_os_ancestry_without_request_credentials`, `body_deadline_retains_headers_and_parser_location_is_reviewed` |
| HTTP status collapse | Received status retained separately from ancestry | `response_status_survives_body_failure_and_size_refusal`, existing 429/deadline test |
| RPC envelope disposal | Strict envelope with present-null distinction, retained numeric code and withheld message/data sizes | `rpc_codes_remain_distinct_in_committed_and_cold_domain_failures` |
| Untagged decoding fallback | One raw-field envelope admission followed by typed decoding; parser category/location captured at the failing stage | `only_complete_exact_rpc_error_objects_parse`, parser-location regression |
| Body bound collapse | Exact declared size or observed streamed lower bound, inclusive limit and Body stage | `response_status_survives_body_failure_and_size_refusal`, `streamed_body_overflow_records_a_lower_bound_without_an_invented_source` |
| Checked result collapse | Result field, numeric range, ABI size, receipt shape and transaction mismatch facts | `local_range_failure_retains_field_and_checked_observation`, receipt/submission tests |

Tests above are in [JSON-RPC tests](../crates/live/evm/src/json_rpc_tests.rs).
The shared [diagnostic tests](../crates/kernel/diagnostics/src/tests.rs) cover constructor and
wire bounds, omission ownership, source ordering, cyclic traversal, and opaque intermediate sources.
The [consuming domain contract test](../crates/domains/evm/tests/provider_failure_contract.rs)
freezes exact error identities and checks a near-budget diagnostic plus full-width owner facts.

Client sources without reviewed downcasts remain opaque while accessible deeper sources are kept.
Raw URL, body, RPC message/data and client formatting are withheld. Complete means the exposed
source chain ended, not that hidden client attempts or raw diagnostic custody were recovered.
Classification is unchanged; timeouts do not establish nonacceptance. The cold Read test proves
retention through the current committed path, not the RFC's pending commit-before-handler cutover.

### 2. Read registration bridges preserve the cause they receive

Source: [live EVM registration](../crates/live/evm/src/lib.rs), `read` and `read_anchored`.

The chain/anchor/balance and anchored-call bridges pass provider errors through without replacing
them. They now carry the provider's reviewed causal value without a second persistence object.

Wrong local route, chain or capability family returns a unit `AdapterInvariantError`. That correctly
stays Internal and prevents provider entry; it still lacks causal diagnostic detail under the new
rule. Retain reviewed mismatch/stage facts in an internal diagnostic path without manufacturing
authenticated integrity evidence or a recoverable provider error.

### 3. Transaction adapters preserve provider categories but erase other layers

Source: [transaction.rs](../crates/live/evm/src/transaction.rs), `reserve_nonce`,
`prepare_transaction`, `execute_transaction`, `map_provider_error` and `map_authority_error`.

- `map_provider_error` retains the complete reviewed provider carrier alongside the originating
  chain-verification, nonce-observation, receipt, canonical-block or submission operation. The
  transaction classification remains OutcomeUnknown.
- `map_authority_error` maps the custody port to AuthorityUnavailable or a unit invariant. The
  port itself exposes only Unavailable/Internal, so SQLx/operation detail is already gone.
- `signer.sign(...).await.map_err(|_| SignerUnavailable)` discards even the signer's existing
  Invalid/Failed distinction and source provenance.
- Submitted-hash and canonical-block mismatches now retain the checked expected/observed values
  in the provider carrier while remaining Unavailable/OutcomeUnknown.
- Codec validation and blocking task join errors become unit invariants. Preserve their internal
  causal category and stage; do not copy arbitrary panic payloads or relabel them operational.

Remediation: preserve nested provider/custody/signer causes with the transaction stage added once
at this boundary. Keep the same prepared command/EffectId, no renonce or replacement command, and
the existing operational-versus-invariant distinction. Classification remains independent of the
diagnostic payload. Avoid a separate generic exception framework for each transaction stage.

### 4. PostgreSQL run Store loses SQLx causes, including at COMMIT

Source: [postgres/lib.rs](../crates/storages/postgres/src/lib.rs), connection setup,
`load_run`, append transaction code, `classify_precommit_sql` and `run_pure_blocking`.

Most SQLx acquisition/query/read errors become StoreError::Unavailable. Precommit classification
uses the SQLSTATE class `23` to select CorruptPhysicalState, then discards the SQLSTATE and source.
COMMIT distinguishes a database error from an uncertain acknowledgement but discards both concrete
causes. Pure blocking join/allocation paths also lose their particular failure category.

Remediation: preserve query/transaction stage, SQLx category, safe SQLSTATE and nested reviewed
causes alongside the existing semantic Store disposition. Do not weaken Unavailable versus
Indeterminate, synchronous COMMIT, exact-head append, or repeatable-read load guarantees.
Database message/detail fields can contain values or private connection information; raw SQLx
formatting is not an approved audit representation.

The Store cannot guarantee persisting its own failure into that same unavailable Store. Its
invocation error must retain the cause chain. A durable independent failure sink would need an
explicit ownership and failure contract; neither recursive append nor plaintext logging solves it.

### 5. PostgreSQL transaction custody is a second source-erasing boundary

Source: [evm_tx.rs](../crates/storages/postgres/src/evm_tx.rs), `unavailable(_: impl Sized)`,
`internal(_: impl Sized)`, transaction commit and retained-record decoding.

These generic helpers deliberately accept and discard any error. Connection, SQL, acknowledgement,
conversion and retained-value errors become two unit variants in
[AuthorityError](../crates/domains/evm/src/custody.rs). The later transaction wrapper cannot restore
their provenance. Ambiguous custody acknowledgement currently remains Unavailable and is resolved
through a later load; preserve that established semantic contract while adding its actual cause.

Remediation: replace source-erasing helpers with operation-specific, source-preserving conversion.
Keep domain custody ports independent of SQLx: adapt the source once at the PostgreSQL boundary
into a reviewed representation. Do not move PostgreSQL types into Program or introduce a second
transaction authority mechanism for diagnostics.

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

### 8. Keystore/signing loses the causal chain before transaction handling

Sources: [keystore/lib.rs](../crates/keystore/src/lib.rs), `KeystoreOwner`,
`KeystoreSigner::sign`; [signing/lib.rs](../crates/signing/src/lib.rs).

Thread spawn errors become KeystoreUnavailable; channel closure, response closure, owner failures
and signing failures are successively reduced to Internal/Unavailable and then SigningFailed.
The transaction adapter finally converts that to SignerUnavailable. Checked cryptographic
construction/recovery errors similarly retain only Invalid/Failed.

Remediation: preserve reviewed causal layers such as owner-start, command-channel-closed,
response-channel-closed, owner-signing-failed and checked-signature-invalid. Record a structured
OS code where one exists. A library can intentionally expose an opaque crypto error; record that
the source supplies no additional detail rather than inventing one.

Never retain `SendError<Command>` wholesale: import commands can own a private scalar. Do not
format panic payloads or secret-bearing objects to obtain a source chain. Keep the thread-affine
keystore design and strengthen secret-exclusion tests in any implementation change.

### 9. In-memory backends have fewer source layers but are not universal exemptions

Sources: [MemoryStore](../crates/kernel/store/src/lib.rs), allocation/ownership helpers and
blocking joins; [memory configuration](../crates/config/src/memory.rs).

Memory Store/index retain numeric capacity facts where their public errors carry them, but map
allocation and task-join failures to Unavailable and checked physical failures to Corrupt. The
in-memory configuration backend has no network/client error chain to preserve; ordinary absent or
unchanged outcomes are not dropped errors.

Remediation: retain actual available memory/task causes and local stage without fabricating an
RPC/SQL chain. Preserve mechanical complete-prefix and atomic append responsibilities. A source
API that exposes no details should be represented honestly as such.

### 10. Adjacent bootstrap and transport conversions lose additional provenance

Sources: [deployment.rs](../crates/app/src/deployment.rs), `Deployment::load` and parsing;
[app/lib.rs](../crates/app/src/lib.rs), `Application::open`, repository/index error maps and RunId
generation; [CLI](../bin/cli/src/main.rs), bounded input/output; [REST main](../bin/rest-api/src/main.rs)
and [REST server](../bin/rest-api/src/server.rs).

Deployment file-open, read, UTF-8, TOML and task failures share one code. Composition then flattens
locator/provider/Postgres errors again. Config and index translation keep only client-facing
categories. CLI read/write errors and REST listener errors discard OS causes; request rejection
rendering intentionally returns reviewed messages without an independent causal audit path.
Entropy failure loses the underlying OS/library category too.

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

Read and pending Effect operational records retain the qualified original cause and State context.
`RuntimeError::Store(#[from] StoreError)` preserves the supplied Store error, and
`RunRequestError::Invocation(#[source] InvocationFailure)` keeps its typed wrapper. These are useful
preservation paths, but their sources are often already coarse values.

`AdapterError<E>` hides operational payloads in Display/Debug. Its conditional `Error`
implementation now exposes nested sources when `E: Error`; ordinary typed payload access remains
available without that bound. This preserves non-Error `MfmValue` capability contracts rather than
requiring unsafe formatting or claiming every value implements `Error`. The capability boundary
tests cover both unformattable non-Error payloads and nested standard sources.

Runtime maps internal preparation, callback, codec and task failures to reviewed internal codes.
Retain those distinctions in an appropriate internal causal representation; do not reclassify
them as operational to force them into existing failure frames. Journal corruption errors also
need internal provenance without echoing untrusted stored bytes.

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

## Minimal implementation direction and deletion scope

Keep one causal audit representation per actual error boundary, with typed nested sources where
they remove ambiguity. Add reviewed fields to existing error contracts, rather than adding a
parallel classifier, generic exception registry, logging framework or `serde_json::Value` bag.
The source owner performs the safe conversion once; outer layers add their operation context
without replacing the cause. Literal raw bytes, error.source() chains and structured RPC error
objects must not be treated as interchangeable evidence.

Complete boundary cutovers should replace/delete:

1. RPC source-loss paths have been replaced as recorded above. Complete the remaining typed
   internal adapter/Runtime cutover without changing the retained provider classifications.
2. SQLx-to-unit helpers, generic `unavailable(_: impl Sized)`/`internal(_: impl Sized)` source sinks
   and repeated category-only translations. Preserve storage ports and commit semantics.
3. Signer/keystore-to-unit conversions through the full signer-to-transaction chain, with explicit
   safe categories instead of retaining secret-bearing channel objects.
4. Application/startup and binary translations that replace the causal error instead of merely
   rendering it. Retain the shared transport surface; do not add a second public diagnostics API
   without a demonstrated consumer need.

Change exact schema/implementation identities when error contracts or classification semantics
change, update all consumers and docs in each coherent cutover, and reject superseded formats.
Do not add migrations, compatibility wrappers, or blanket suppression exceptions. Report LOC
removed from lossy plumbing separately from necessary audit additions. The rule/report commit is
documentation-only; it does not alter current public or persistence contracts.

## Required evidence for remediation

- Inject distinct RPC errors with equal classifications; verify their original codes and reviewed
  details remain distinguishable in hot and cold records.
- Distinguish HTTP status, send/body timeout, parser category/location and response-size failures.
  Verify method/stage survives transaction wrapping.
- Inject SQLx connection/query/decode/COMMIT failures; verify cause preservation and unchanged
  Unavailable/Indeterminate behavior, including an audit sink failure.
- Exercise signer channel closure, owner failure and cryptographic rejection separately. Use
  synthetic secret sentinels to prove exclusion without printing real secrets or source payloads.
- Verify public codes remain reviewed projections while typed invocation/audit causes remain
  available. Assert explicit omission metadata for any withheld or bounded evidence.
- Verify admitted sizes, exact schema identity, append ambiguity, concurrent candidates and
  cancellation. Never acknowledge an error record that did not commit.
- Audit every mapper touched by a boundary cutover. A grep match alone is not a defect, and a
  successful grep for `source` is not proof of end-to-end provenance.

Use [build and verification](build-and-verification.md) for scope-selected tests. Runtime/domain
or persistence changes require focused consuming tests and one final CI on the exact candidate;
keystore changes require strengthened tests explicitly. This review and rule update require local
link review and `git diff --check`; no Rust behavior changes or external IO tests are performed.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Whole stack means complete available causal provenance, with explicit accounting for withheld data | Arbitrary provider/SQL messages and channel error objects can contain secrets | Lossless raw capture requires a restricted custody contract beyond the secret-free Journal | Set concrete evidence acceptance cases before implementing raw-message retention; do not equate redaction with losslessness |
| Client APIs expose enough structured source detail | A library may discard evidence before MFM receives an error | Capturing Error::source() alone still misses the real provider error | Inspect each concrete client capture point and inject nested/malformed protocol failures |
| Acknowledged operational outcomes remain the run-audit boundary | Store/bootstrap failure or process death can prevent the audit write itself | Independent durable failure capture or physical-attempt auditing needs additional authority and protocol | Define those guarantees separately; preserve causal invocation errors and never claim failed persistence succeeded |
