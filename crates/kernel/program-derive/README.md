# mfm-program-derive

Typed kernel crate for derive macros that generate value, config, input, operation-output, and
public-output descriptor evidence.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free. It must not depend on old dynamic
machine, SDK, runtime, or domain crates.
