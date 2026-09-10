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
  boundary, retain a bounded typed representation of each available cause layer, operation/stage,
  protocol or OS code, and reviewed diagnostic fields. Use source-preserving conversions where
  appropriate; `Error::source()` alone neither serializes a chain nor proves complete capture.
- Do not silently discard a source with `map_err(|_| ...)`, a catch-all mapper, a unit replacement,
  or a success-shaped fallback. A terse public message is legitimate only when the causal audit
  representation remains available. Expected absence and Pending require their explicit protocol
  meaning; they cannot hide a failed observation.
- Secrets never enter Program, admitted context, Journal, public output, logs or error details.
  Do not capture generic Debug/Display dumps, request-bearing channel errors, arbitrary provider
  messages, database details or panic payloads as a shortcut. Preserve reviewed causal facts and
  explicitly identify any withheld, unavailable or size-limited evidence. Partial capture must
  never be labeled raw, complete or lossless.
- Literal byte-exact external evidence and safe causal diagnostics are different contracts. If
  full retention requires restricted custody, use the architect rule to settle that contract
  before implementation. Encryption or moving a blob to another file is not implicit permission
  to persist credentials. Do not introduce a universal opaque error bag or a second logging system
  to conceal a missing design.
- Carry retained causes into acknowledged operational failure records and cold observations.
  Internal failures remain internal; do not manufacture a recoverable domain/provider incident to
  obtain a Journal record. Store, startup and transport failures need their own explicit durability
  contract: returning a causal error does not prove it was durably audited.
- Preserve definite failure versus ambiguous acknowledgement, retained command identity, and
  cancellation semantics. If recording fails, report the audit failure without asserting that the
  candidate or underlying outcome was committed. Do not recursively try to audit a failed Store
  through that same Store or silently fall back to plaintext logs.

For each changed adapter, tests must inject distinguishable nested causes and assert retained
layers/fields, unchanged classification semantics where applicable, hot/cold audit preservation,
and secret exclusion. Exercise retention bounds and audit-write failure at the affected boundary.
Document unresolved loss at the first lossy conversion, with affected callers and remediation;
existing lossy implementations are gaps to fix within a coherent cutover, not allowed patterns.

See [the adapter review](adapter-error-audit.md) for the current inventory and implementation gaps.

## Test Value

Every test must be able to fail because an observable capability regressed. Prefer one scenario
through a crate's public API that covers a useful success path and its material boundary failures.
Cross-crate tests should exercise the same entry points and concrete domain types that a consumer
uses; do not rebuild a parallel model of the implementation in test-only fixtures.

Do not add runtime assertions for facts already proved by Rust's type checker, trait bounds, private
fields, or an infallible constructor. Do not freeze implementation detail such as helper call
counts, internal declaration indices, duplicated associated-type declarations, or constants that
the test reads from the implementation itself. A schema, wire, hash, or public error assertion is
valuable only when it independently states an interoperability, hostile-input, redaction, or
persistence contract.

Keep setup visible in the scenario. Introduce a test helper only when it represents a reusable
external boundary (for example a hostile Store or a loopback provider), not to shorten ordinary
value construction or assertions. Prefer production constructors and functions over fixture files.
Retain small unit tests for complex deterministic algorithms when their input/output table is the
clearest contract. Retain compile-fail tests for deliberate authority and ownership exclusions.

## Commits and Reporting

Divide non-trivial work into ordered logical commits. Each must contain one coherent, internally
consistent design plus its tests and documentation. Keep inseparable cutovers together and unrelated
cleanup separate.

Report what changed and was deleted, verification run, and remaining unverified risks or blockers.
