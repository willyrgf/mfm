# mfm-events

Typed kernel crate for closed versioned kernel event schemas.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

Events carry append-only artifact-reference facts. Storage requirements, retained-artifact reads,
and verified artifact bytes are derived and owned by `mfm-store`.
