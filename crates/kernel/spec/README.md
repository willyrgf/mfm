# mfm-spec

Typed kernel crate for typed execution spec data contracts.

`mfm_spec::v1::HashedSpecEnvelope` is a hash-only envelope for canonical spec bytes and
non-semantic audit metadata. It is not certification authority; callers must use
`mfm-certify` to obtain or verify a non-forgeable certified typed-spec authority.

Parsed persisted typed spec bytes are typed data only and remain hostile until verified by
`mfm-certify` against a registry and certificate. Hash matches, audit metadata, summaries, or
source scans do not certify this data.

`docs/design.md` is the normative typed-core authority contract. This crate is framework-owned and
must remain domain-free. It must not depend on old dynamic machine or SDK crates.
