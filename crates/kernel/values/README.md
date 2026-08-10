# mfm-values

Typed kernel crate for value, planning config, schema descriptor, and public-output contracts.

The crate owns the one producer-independent `RetainedValueContract` used directly by
certification, journal, Runtime, store, and replay. The contract is owner-validated and contains
only exact schema, semantic type, role, media type, and evidence-contract authority; journal
producer and byte identity remain outside it.

The crate is also the sole owner of complete `SchemaIdentity` and `SchemaShape` validation.
`SchemaIdentity::new` validates a newly assembled descriptor, `strict_decode` accepts only its
exact bounded canonical representation, `canonical_json` and `schema_id` derive its hash-defining
identity, and `validate_canonical_value` checks canonical value bytes against the complete closed
shape. Shape construction and decoding reject invalid field or variant structure, unsupported
integer forms, floats, and descriptors deeper than the framework bound; downstream classifiers,
stores, and replay reuse these entry points instead of defining another schema interpreter.

The crate also owns the one frozen canonical component-object-evidence contract and its
owner-derived `ContentRef`. Framework retained-contract factories use that common identity instead
of accepting caller-selected evidence metadata.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.
