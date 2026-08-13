# mfm-ids

Typed kernel crate for strong identity primitives.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

Identity grammars distinguish schema identities using `sha256-jcs-v1` from exact-byte content
identities using `sha256-v1`. `ContentRef` is only a schema-qualified content address; it is not
retained-object, producer-lineage, reachability, or access authority. `SequentialControlAddress`
is the sole callback-free State/Match control address. `TenantScopeId` is an app-owned, non-secret
scalar ownership scope.
