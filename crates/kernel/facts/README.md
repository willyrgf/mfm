# mfm-facts

Typed kernel crate for fact descriptors, checked semantic fact-content identity, field extraction
contracts, visibility, and query evidence.

`FactContentIdentity` identifies verified descriptor, subject-material, response-schema, and
response-content bytes without inheriting any claim, artifact, run, or store-occurrence identity.
Collectors and report hydration must derive it from hydrated canonical material; compact claim and
reference hashes are checked against that material rather than trusted as a constructor input.

`docs/design.md` is the normative typed-core authority contract. Portfolio collectors and
fact-backed reporting authority live in `docs/design.md`. This crate is
framework-owned and must remain domain-free.
