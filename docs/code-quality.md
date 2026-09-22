# Code Quality Policy

This policy applies to every code, test, documentation, build, and workflow change.

## One Current Design and Complete Cutovers

MFM is in clean-slate, pre-release development: keep one current design. Superseded revisions and
data have no compatibility claim unless an explicit repository-wide policy creates one; versioning
alone does not. Historical invariant and hostile-input fixtures are not compatibility.

Optimize for correctness, dexterity, and simplicity: place guarantees at the strongest practical
boundary, make changes local and complete, and retain only the smallest current design.

Minimize concepts, code paths, public types and schemas, duplicated responsibilities, future change
sites, and LOC. Give each responsibility one owner and implementation. Breaking APIs, CLI/REST
contracts, schemas, persisted formats, and documented behavior is allowed.

Cut over completely in one logical change: update every current producer, consumer, test, fixture,
and document, then delete everything superseded. Do not deprecate or hide it; Git history is the
archive. Never add legacy decoders or migrations, compatibility shims, dual or mixed-version paths,
downgrade or software-version rollback support, or fallbacks.

For a changed persisted contract, update or reset its baseline and reject old data. Never reinterpret
old bytes, rewrite append-only history, or retain a legacy reader. Keep version identifiers only for
a current hashing, domain-separation, or hostile-input requirement.

Fix the design or add missing support completely; never add hacks, partial workarounds, schema shims,
or parallel implementations. Report a blocker instead. Preserve correctness, security, data
integrity, design invariants, testability, and operability. Remove LOC by deleting duplication,
indirection, and obsolete behavior—not readability, validation, controls, tests, or necessary
documentation. Update `docs/design.md`, architecture documentation, and contract tests when their
contracts change.

## Correctness by Construction

Make illegal states unrepresentable. Move correctness obligations from repeated control flow and 
programmer discipline into representation and construction. A trusted-core value should carry evidence 
of the facts its consumers rely on.

Use the smallest Rust mechanism that expresses the guarantee:

- Use structs for coexisting facts (product types) and enums for alternatives (sum types), with
  state-specific data in its variant. Avoid a status plus correlated flags or `Option` fields.
  Match exhaustively when a new variant requires a decision.
- Use newtypes for semantic distinctions and private fields with fallible constructors for value
  invariants. Any public constructor, conversion, `Default`, or deserialization path must preserve
  them.
- Prefer structural proof over a checked wrapper, such as a head and tail for a non-empty collection.
- Use ownership for resource and single-use correctness, typestate for state-specific operations,
  and const generics for stable compile-time dimensions.

Parse untrusted HTTP, CLI, storage, and other IO data into domain types at the boundary; do not
validate a primitive and keep passing it through the core. Fallible construction returns a typed,
redaction-safe error, methods preserve the invariant, and internal APIs accept the domain type.
Treat data as untrusted whenever it crosses a trust boundary.

Use runtime checks only for ambient or changing facts that one value cannot prove. Put each check at
its owning capability and return an explicit checked outcome that downstream APIs require. Do not add
type machinery more complex than the invalid states, branches, or change sites it removes.

Test constructor rejection and invariant-preserving transformations. Use compile-fail tests when a
compile-time exclusion is an intentional API contract.

## Error Provenance and Auditability

Preserve the complete available causal error chain through adapters, library ports, Runtime,
Application and transport boundaries. "Error stack" means the chain of causes and boundary
context, not a captured Rust backtrace. Classification, retry decisions and public error codes
are projections of the retained error; none is a substitute for it.

- Retain concrete sources while they are safely held within their owning boundary. At an audit
  boundary, retain the selected representation of each available cause layer, operation/stage,
  protocol or OS code, and reviewed diagnostic fields. Use source-preserving conversions where
  appropriate; `Error::source()` alone neither serializes a chain nor proves complete capture.
- Do not silently discard a source with `map_err(|_| ...)`, a catch-all mapper, a unit replacement,
  or a success-shaped fallback. A terse public message is legitimate only when the causal audit
  representation remains available. Expected absence and Pending require their explicit protocol
  meaning; they cannot hide a failed observation.
- Do not deliberately append MFM secrets, full request/connection objects or panic payloads to
  diagnostics. Selected dependency-supplied JSON/parser, EVM transport/provider and CLI IO messages
  follow the diagnostic trust contract in `docs/design.md`: retain supplied text without generic
  credential detection, sanitization, certification, diagnostic quotas or omission ledgers. This
  trust is not a promise that dependencies cannot disclose sensitive content. Ordinary Program and
  context validation and actual Object/frame/run/report limits remain in force.
- Keep concrete errors within interfaces that support them. Selected heterogeneous internal
  boundaries construct immutable `InvocationDiagnostic` data once; receivers forward it without
  recapturing or reconstructing classifiable originals from JSON. Declared operational failures
  retain their concrete owner data before admission. Do not introduce opaque native-error custody,
  stored projectors, a second logging system or universal error bags.
- Failed first encoding of a declared original reports the encoding cause, known execution/contract
  context and explicitly unavailable original detail/identity. Do not retry the original serializer
  or keep a parallel native payload. Once admitted, the complete Failure/Object supplies persistence
  and reporting. Neither unavailable details nor invocation-only data prove durable recording.
- Carry retained causes into acknowledged operational failure records and cold observations.
  Internal failures remain internal; do not manufacture a recoverable domain/provider incident to
  obtain a Journal record. Store, startup and transport failures need their own explicit durability
  contract: returning a causal error does not prove it was durably audited.
- Preserve definite failure versus ambiguous acknowledgement, retained command identity, and
  cancellation semantics. If recording fails, report the audit failure without asserting that the
  candidate or underlying outcome was committed. Do not recursively try to audit a failed Store
  through that same Store or silently fall back to plaintext logs.

Malformed persisted framework data has a deliberately narrower decoding contract: preserve parser
category, available location and rejection reason. Ordinary Serde conversion need not retain nested
constructor ancestry or structured size facts. This exception does not apply to direct typed
construction, postdecode slot admission, declared execution originals or selected native hooks.
JSON parser/serializer diagnostics retain dependency-supplied text. This trusted text is not a
certification that dependencies cannot disclose sensitive content; MFM must not deliberately append
its own secrets, full requests or connection objects as diagnostic context.

For each changed adapter, tests must inject distinguishable nested causes and assert retained
layers/fields, unchanged classification semantics where applicable, hot/cold audit preservation,
and exclusion of deliberately appended MFM secrets. Exercise recording failure at the affected
boundary. Size-bound rejection uses the cheap comparison tests in Test Value, not a
production-maximum payload.
Document unresolved loss at the first lossy conversion, with affected callers and remediation;
existing lossy implementations are gaps to fix within a coherent cutover, not allowed patterns.

See [the adapter review](adapter-error-audit.md) for the current inventory and implementation gaps.

## Test Value

Every test must be able to fail because an observable capability regressed.

Make every test's purpose clear in short, plain English. Name the condition and observable result
in the test name, for example `append_with_stale_head_leaves_history_unchanged`. When the name alone
does not explain the guarantee or why a regression matters, add a concise purpose comment, such as
"A stale writer must not overwrite acknowledged history." Explain the specific behavior protected;
avoid generic wording such as "tests append functionality", repeating the name, or narrating setup.
For parameterized tests, explain the shared guarantee once and give cases meaningful labels.
Check that the assertions establish the stated purpose; narrow the claim or strengthen the test
when they do not. Apply this convention when adding or changing tests.

Prefer one scenario through a crate public API that a consumer can call. Cover one useful success
path and its material boundary failures. Product tests call Application methods and production
domain types. They do not rebuild a parallel Operation, State, or adapter model.

Framework composition and extension tests may author Operations or minimal new States when those
public contracts are the behavior under test. Use existing components for the surrounding scenario;
assert useful outcomes and boundary failures rather than duplicating engine coverage. Distinguish
these tests from product selection tests and deliberate fault/recovery tests. The
[acceptance ownership table](build-and-verification.md#acceptance-scenarios-and-independent-oracles)
identifies the consuming boundary and independent oracle for each current scenario.

Do not add runtime assertions for facts already proved by Rust's type checker, trait bounds, private
fields, or an infallible constructor. Do not freeze helper call counts, internal declaration
indices, duplicated associated-type declarations, or constants that the test reads from the
implementation itself. A schema, wire, hash, or public error assertion is valuable only when it
independently states an interoperability, hostile-input, redaction, or persistence contract.

Judge the assertion, not the test category: an infallible constructor's public encoding or a
roundtrip through an independently specified persisted wire can still protect behavior Rust does
not prove. When deleting overlap, identify the retained scenario and the exact guarantee it owns;
generic engine coverage does not replace a consuming boundary's mapping or rendering assertions.

Do not allocate production size maxima to prove a hardcoded ceiling. `to_json_bounded` and
`SizeLimitExceeded::check` own that comparison; test them with small explicit limits. Production
constants remain the runtime policy.

Keep setup visible in the scenario. Inline ordinary value construction with production constructors.
A helper is valid only when it stands in for an external boundary: a hostile Store, a hostile
config repository, a scripted or loopback provider, a compile-fail UI case, or a managed e2e
service. Do not add wrappers that only shorten construction or assertions.

Retain small unit tests for complex deterministic algorithms when an input/output table is the
clearest contract. Retain compile-fail tests for deliberate authority and ownership exclusions.

Kernel Runtime may keep one minimal synthetic program because `mfm-runtime` cannot depend on domain
crates. Product tests never use that program. Each unique engine fact has one owner.

## Commits and Reporting

Divide non-trivial work into ordered logical commits. Each must contain one coherent, internally
consistent design plus its tests and documentation. Keep inseparable cutovers together and unrelated
cleanup separate.

Report what changed and was deleted, verification run, and remaining unverified risks or blockers.
