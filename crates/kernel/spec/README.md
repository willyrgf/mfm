# mfm-spec

Strict serialized algebra for authored, expanded, and certified structured programs.

It owns canonical `State`/`Match`/`FanOut`/fragment forms, lexical producers, failure plans,
execution/capability contracts, expansion profiles/proofs, component manifests, secret-free
implementation descriptors, and the certified root/document. Normalization derives exact
occurrence, fragment-boundary, failure-plan, and slot identities and rejects hostile references.

The crate defines data and validation helpers only. Certification authority belongs to
`mfm-certify`; cursor/history authority belongs to `mfm-store`.
