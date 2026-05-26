# mfm-spec

Typed kernel crate for typed execution spec data contracts.

`mfm_spec::v1::HashedSpecEnvelope` is a hash-only envelope for canonical spec bytes and
non-semantic audit metadata. It is not certification authority; callers must use
`mfm-certify` to obtain or verify a non-forgeable certified typed-spec authority.

`RFC_TYPED_CORE_PROPOSAL_1.md` is the authority for this crate during the typed-core rewrite.
This crate is framework-owned and must remain domain-free. It must not depend on old dynamic
machine or SDK crates.
