# mfm-certify

Typed kernel crate for certification of typed program drafts and execution specs.

This crate mints `CertifiedTypedSpec`, the non-forgeable in-memory authority returned only by
registry-backed certification or persisted bundle verification. Persisted spec bytes and
`CertifiedSpecCertificate` bytes are evidence only; parsing a bundle returns untrusted data until
`verify_certified_bundle` validates hashes, registry digest, descriptor evidence, lowering and
canonicalizer identity, and the typed spec against the registry.

`RFC_TYPED_CORE_PROPOSAL_1.md` is the authority for this crate during the typed-core rewrite.
This crate is framework-owned and must remain domain-free. It must not depend on old dynamic
machine or SDK crates.
