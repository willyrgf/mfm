# mfm-canonical

Typed kernel crate for canonical bytes, canonical JSON, and digest contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

The recoverability v2 target embeds the sole current versioned annex and exposes
its strict codec and registered-domain digest operations through
`RecoverabilityContractV2`.
Callers select annex contracts by name; they do not supply local schema
identities, arbitrary semantic domains, or claimed digests. Exact retained-byte
content addressing remains distinct from semantic envelope hashing.

The same embedded contract resolves exact registered `SchemaId` values and
projects terminal `ContentRef`/`ValueRef` edges by walking only annex-declared
schema structure. Reference paths are opaque RFC 6901 pointers; projection
never searches payload keys or text for reference-like values. A decoded root
`mfm.value-ref.v1` is transport-only under its incoming content reference, so
projection returns no edges and does not expose its producer or evidence
fields. Nested annex-declared `ValueRef` fields remain semantic edges.
