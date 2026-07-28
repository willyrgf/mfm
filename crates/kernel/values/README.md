# mfm-values

Typed kernel crate for value, planning config, schema descriptor, and public-output contracts.

The crate owns the one producer-independent `RetainedValueContract` used directly by
certification, executor, journal, runtime, and replay. The contract is annex-validated and contains
only exact schema, semantic type, role, media type, and evidence-contract authority; journal
producer and byte identity remain outside it.

The crate also owns the one frozen canonical component-object-evidence contract and its
annex-derived `ContentRef`. Framework retained-contract factories use that common identity instead
of accepting caller-selected evidence metadata.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.
