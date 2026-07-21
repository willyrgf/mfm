# mfm-facts

Typed kernel crate for fact descriptors, checked semantic fact-content identity, field extraction
contracts, visibility, and query evidence.

`FactContentIdentity` identifies verified descriptor, subject-material, response-schema, and
response-content bytes without inheriting any claim, artifact, run, or store-occurrence identity.
Collectors and report hydration must derive it from hydrated canonical material; compact claim and
reference hashes are checked against that material rather than trusted as a constructor input.
Direct deserialization is rejected because the compact serialized fields alone cannot establish
that verification.

`FactContentIdentityEvidence` is the fact-layer-owned opaque persisted carrier for a verified
identity when a typed receipt must deserialize before its later consumer can hydrate the fact.
It cannot expose an identity; consumers must rederive it from descriptor, typed subject, and
hydrated response, accepting the evidence only on an exact match. It is not a domain-specific
receipt identity or a claim-occurrence identifier.

Canonical fact-query v2 can bind this evidence as an exact provider-side narrowing filter. The
provider checks trusted indexed-reference components before ordering and limiting; that compact
check never replaces subsequent artifact hydration and typed identity rederivation.

`docs/design.md` is the normative typed-core authority contract. Portfolio collectors and
fact-backed reporting authority live in `docs/design.md`. This crate is
framework-owned and must remain domain-free.
