# mfm-program-derive

Typed kernel crate for derive macros that generate value, config, input, operation-output, and
public-output descriptor evidence.

`RFC_TYPED_CORE_PROPOSAL_1.md` is the authority for this crate during the typed-core rewrite.
This crate is framework-owned and must remain domain-free. It must not depend on old dynamic
machine, SDK, runtime, or domain crates.
