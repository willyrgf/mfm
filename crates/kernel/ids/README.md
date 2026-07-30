# mfm-ids

Typed kernel crate for strong identity primitives.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

The recoverability v1 contract distinguishes semantic identities using
`sha256-jcs-v1` from exact-byte content identities using `sha256-v1`.
`SemanticDigest`, `EffectKey`, and `AttemptId` hard-code the semantic algorithm
and do not accept or substitute a raw `ContentDigest`.
`ContentRef` is only a schema-qualified content address; it is not retained-object,
producer-lineage, reachability, or access authority. `TenantScopeId` is an
app-owned, non-secret scalar ownership scope.
