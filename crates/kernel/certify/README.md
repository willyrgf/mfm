# mfm-certify

Typed kernel crate for certification of typed program drafts and execution specs.

This crate mints `CertifiedTypedSpec`, the non-forgeable in-memory authority returned only by
registry-backed certification or persisted spec/certificate verification. Parsed typed spec JSON,
`mfm_spec::v1::HashedSpecEnvelope`, persisted `CertifiedSpecCertificate` bytes, and persisted
spec/certificate byte pairs are not runtime authority.

Persisted spec bytes and certificate bytes are hostile data until
`verify_persisted_spec_certificate` validates the spec hash, certificate hash, certifier identity,
registry digest, descriptor
identities and digests, lowering and canonicalizer identity, public-output schema id, audit
metadata, and the typed spec itself against the registry. Hash match alone is not certification.

`CertificationRegistry` is explicit certification authority. Registry assembly may register trusted
already-lowered descriptor identities, but persisted spec descriptors are not trusted registry input
until the certifier verifier has accepted the spec/certificate pair.

Transition-context metadata is certified here, not in domain crates or runtime routing. The
certifier checks context table content addressing, node context requirements, context-bound
input/output resource kind and stage contracts, approved producer descriptors, seed producer
authorization, same-value bridge preservation, side-effect verify preservation, and no-context
framework receipt outputs.

`docs/design.md` is the normative typed-core authority contract. This crate is framework-owned and
must remain domain-free.
