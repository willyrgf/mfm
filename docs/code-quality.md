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

- Use structs for coexisting facts (products) and enums for alternatives (sums), with state-specific
  data in its variant. Avoid a status plus correlated flags or `Option` fields. Match exhaustively
  when a new variant requires a decision.
- Use newtypes for semantic distinctions and private fields with fallible constructors for value
  invariants. Any public constructor, conversion, `Default`, or deserialization path must preserve
  them.
- Prefer structural proof over a checked wrapper when simpler, such as a head and tail for a
  non-empty collection.
- Use ownership for resource and single-use correctness, typestate for state-specific operations,
  and const generics for stable compile-time dimensions.

Parse untrusted HTTP, CLI, storage, and other IO data into domain types at the boundary; do not
validate a primitive and keep passing it through the core. Fallible construction returns a typed,
redaction-safe error, methods preserve the invariant, and internal APIs accept the domain type.
Treat data as untrusted whenever it crosses a trust boundary.

Use runtime checks for ambient or changing facts that one value cannot prove. Put each check at its
owning capability and return an explicit checked outcome that downstream APIs require. Do not add
type machinery more complex than the invalid states, branches, or change sites it removes.

Test constructor rejection and invariant-preserving transformations. Use compile-fail tests when a
compile-time exclusion is an intentional API contract.

## Commits and Reporting

Divide non-trivial work into ordered logical commits. Each must contain one coherent, internally
consistent design plus its tests and documentation. Keep inseparable cutovers together and unrelated
cleanup separate.

Report what changed and was deleted, verification run, and remaining unverified risks or blockers.
