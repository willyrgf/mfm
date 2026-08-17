# mfm-ids

Checked stable, schema, content, run, and entry-point identities shared by kernel/domain crates.
Parsing enforces exact algorithm tags, grammar, lengths, and lowercase hexadecimal spelling.

`RunId` and `ArtifactId` are fixed to `sha256-jcs-v1`. `ContentDigest` accepts either supported
algorithm, while `ContentRef` combines a schema interpretation with an exact-byte
`sha256-v1` identity. Journal separately qualifies the retained frame-local bytes.

This crate owns no execution address, capability kind/version, Store scope/epoch/tenant, append
request, field path, or semantic-record digest family.
