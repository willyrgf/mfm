# mfm-canonical

Typed kernel crate for canonical bytes, canonical JSON, and digest contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.

The crate exposes canonical JSON exactness, the global syntax bounds in
`limits`, and unbranded byte hashing. `raw_content_digest` and
`RawContentDigestHasher` accept bytes and never a semantic domain string, so
neither can mint a domain-separated semantic identity. Exact retained-byte
content addressing remains distinct from semantic envelope hashing, which each
owner performs under its own domain.

There is no schema registry here. Persisted schema identity, shape validation,
and reference projection belong to `mfm-values` and to each retained owner.
