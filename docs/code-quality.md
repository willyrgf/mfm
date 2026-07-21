# Code Quality Policy

This policy applies to every code, test, documentation, build, and workflow change in this
repository.

## One Current Design

Optimize the repository as a whole for fewer concepts, code paths, public types and schemas,
duplicated responsibilities, and places a future change must touch. Each responsibility should
have one clear owner and one implementation.

Reducing lines of code is valuable when it removes duplication, indirection, or obsolete behavior.
Do not obtain a smaller codebase by compressing readable code or removing validation, security
controls, tests, or necessary documentation.

## Replace; Do Not Preserve

MFM maintains no backward compatibility. Breaking APIs, CLI/REST contracts, schemas, persisted
formats, and documented behavior is allowed. Prefer the best current design over preserving an
inferior previous one.

When a design changes, complete the cutover and delete the superseded implementation, types, entry
points, aliases, adapters, feature flags, readers/writers, tests, fixtures, and documentation. Do
not deprecate old paths, hide them, or retain compatibility shims, dual paths, or fallbacks for old
behavior. Git history is the source archive.

Update every current in-repository producer and consumer in the same logical change. For a changed
persisted contract, update or reset its baseline and reject old data explicitly; never reinterpret
old bytes, rewrite append-only history, or retain a legacy reader. Version identifiers may remain
when the current contract needs them for hashing, domain separation, or hostile-input rejection.

A breaking change never relaxes correctness, security, data-integrity, or design invariants.

## Complete Changes

Fix the underlying design or add missing support properly. Do not introduce hacks, monkey patches,
partial workarounds, fragile schema shims, or parallel implementations. If a correct complete
solution is not possible, report the blocker instead of approximating it.

`docs/design.md` describes the one current design contract. Update it, the architecture
documentation, and affected contract tests deliberately when that design changes; do not preserve
obsolete code merely because the current documentation describes it.

## Logical Commits

Divide non-trivial work into a sequence of logical commits. Each commit must represent one coherent
change, include its required tests and documentation, and leave the repository internally
consistent. Do not mix unrelated cleanup with behavior changes or create temporary compatibility
paths merely to stage a refactor; keep inseparable cutovers in one commit.

## Reporting

Report what changed, what was deleted, which verification ran, and any remaining unverified risk or
blocker.
