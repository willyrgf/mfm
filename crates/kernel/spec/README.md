# mfm-spec

Frozen recoverability-v1 value contracts for authored programs, planning profiles, expanded
graphs, implementation manifests, and certificates.

The crate owns canonical data and strict Annex-backed codecs only. It does not author programs,
plan graphs, execute callbacks, bind live capabilities, or grant admission authority.

Persisted authored/spec/manifest/certificate bytes remain untrusted data until `mfm-certify`
strictly decodes them and reproduces the exact expanded graph and certificate. A content reference
or hash by itself is not certification authority.

Certified retained slots use and re-export the single
`mfm_values::RetainedValueContract`; the spec does not define a parallel retained metadata type.

`docs/design.md` is the normative typed-core authority contract. This crate is framework-owned and
must remain domain-free.
