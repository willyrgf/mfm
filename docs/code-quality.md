# Code Quality Policy

This policy applies to every code, test, documentation, build, and workflow change.

## One Current Design

MFM is in clean-slate, pre-release development. Superseded revisions and their data have no
compatibility claim unless the current design deliberately retains a historical input for an
invariant or hostile-input test. Only an explicit repository-wide policy change creates
compatibility obligations; versioning alone does not.

Optimize the whole repository for the fewest concepts, code paths, public types and schemas,
duplicated responsibilities, and future change sites. Give each responsibility one owner and
implementation. Breaking APIs, CLI/REST contracts, schemas, persisted formats, and documented
behavior is allowed.

Never add legacy decoders or migrations, compatibility shims, dual or mixed-version paths,
downgrade or software-version rollback support, or fallbacks. The target must still be
implementable, testable, and operable while preserving correctness, security, data integrity, and
design invariants. Remove LOC by deleting duplication, indirection, or obsolete behavior—not
readability, validation, controls, tests, or necessary docs.

## Complete Changes

Complete each cutover in one logical change: update all current producers, consumers, tests,
fixtures, and docs; delete all superseded code, APIs, schemas, tests, fixtures, and docs. Do not
deprecate or hide them. Git history is the archive.

For a changed persisted contract, update or reset its baseline and reject old data. Never
reinterpret old bytes, rewrite append-only history, or retain a legacy reader. Keep version
identifiers only for a current hashing, domain-separation, or hostile-input requirement.

Fix the underlying design or add missing support completely. Do not add hacks, partial workarounds,
schema shims, or parallel implementations; report a blocker instead. Update `docs/design.md`,
architecture documentation, and contract tests when their design changes.

## Commits and Reporting

Divide non-trivial work into ordered logical commits. Each must contain one coherent, internally
consistent design plus its tests and documentation. Keep inseparable cutovers together and
unrelated cleanup separate.

Report what changed and was deleted, verification run, and remaining unverified risks or blockers.
