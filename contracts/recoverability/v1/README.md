# Recoverability v1 authority

`annex.json` and `corpus.json` are the authoritative recoverability v1 machine
contracts. Both files are exact, float-free canonical JSON bytes. Their object
key order, integer spelling, string bytes, schema identifiers, semantic
identities, content digests, and vector bytes are normative.

Consumers must load these files directly. They must not replace an annex
contract or schema identifier with a local string constant, derive a persisted
wire shape from Serde defaults, copy selected corpus cases into a private
fixture set, or add compatibility readers and fallbacks. Runtime-only authority
types listed by the annex have no persisted encoding.

## Required consumption

1. Read `annex.json` as exact bytes, enforce the annex-wide byte bound, and
   validate that the bytes are already canonical.
2. Recompute every schema identifier from its descriptor after removing only
   `schema_id`, using the `mfm.schema.v1` domain.
3. Resolve every `reference` against the closed schema registry and validate
   values against the selected schema before hashing or use.
   Structural codec validation covers the machine `shape`. Every string in a
   shape's `invariants` array is separately mandatory owner/fold validation;
   it is never implied by structural acceptance and may require surrounding
   journal, observation, frontier, or store authority. Schema-acceptance
   vectors exercise the structural boundary, while semantic-domain positives
   are also owner-valid and relational vectors exercise representative
   cross-value invariants.
4. Compute semantic identities from the universal canonical envelope
   `{"domain":domain,"value":value}` and the registered domain/result kind.
5. Compute `ContentDigest` only as raw SHA-256 over exact retained bytes, using
   the `content:sha256-v1:` identity grammar. A semantic digest is never a
   content digest.
6. Read `corpus.json`, verify its `annex_content_digest`, and execute every
   vector whose `consumer_coverage` names the consuming component.

The corpus classifies failures explicitly. `error_codes.codec` is the exact
closed API vocabulary returned by the shared recoverability codec.
`error_codes.relational` is the separate closed vocabulary for cross-record,
store, authority, and fold validation. Consumers must not translate ad hoc
between the two classes.

## Change discipline

These artifacts change only through a reviewed recoverability contract cutover.
A schema descriptor change changes its schema identifier. A semantic preimage
or domain change changes its result identity. Expected digests must never be
edited merely to make a test pass; regenerate the complete artifact set, review
the semantic diff, and rerun exhaustive validation.

Portable run exports use
`application/vnd.mfm.run-export.v1+json`. Their manifests contain no digest of
themselves or of the complete bundle; an external raw content digest addresses
the exact retained export bytes.

No manifest, event, artifact, fact payload, context snapshot, vector, error
detail, or export may contain a password, mnemonic, private key, credential,
provider response body, URL, or raw diagnostic text.

## Frozen artifact metadata

The current recoverability-v1 cutover contains 259 schemas and 231 separately
owned invariant clauses. The corpus contains 427 positive, 59 negative, and 84
relational vectors (570 total), including 373 structural schema-acceptance
vectors.

```text
annex_bytes: 222127
annex_sha256: a3fb5cf2e0486a1a1e906c2fd93b10b3f0f52c5a785b163b6cc758ff39a4defe
corpus_bytes: 1706315
corpus_sha256: 8d10c1a05820a18781a4864fb2d47248d6de41689db5fe0ed4800dd8b0742f82
```
