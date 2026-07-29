# Recoverability v2 authority

`annex.json` and `corpus.json` are the sole current production recoverability
contract. Both files are exact, float-free canonical JSON bytes. The complete
v1 directory is retained byte-identically only as an archival and hostile-input
reference; production consumers must not load it.

Portable run transfer uses the registered framed sequence
`mfm.portable-run-export-stream.v1` with media type
`application/vnd.mfm.run-export-stream.v1+json-seq`. Its sole integrity value
is the external raw SHA-256 `ContentRef` over every RS, canonical frame byte,
and LF. There is no bundle, manifest, member path, or internal stream digest.

## Frozen artifact metadata

```text
annex_bytes: 223651
annex_sha256: d6ef3644581094b1d08812f71a6a05fdaa935972a8b63ab0af818ef4179a0ba4
annex_schema_count: 258
annex_invariant_clause_count: 233
corpus_bytes: 1709958
corpus_sha256: b41900112b6bb90c350c25897cbc24ba81977da77eb892c32519042c1647fe32
corpus_positive_case_count: 433
corpus_negative_case_count: 59
corpus_relational_case_count: 84
corpus_total_case_count: 576
corpus_schema_acceptance_case_count: 379
corpus_codec_rejection_case_count: 27
corpus_relational_rejection_case_count: 32
```

All 576 vectors are mandatory for each of the nine consumers named by
`corpus.json`.

## Portable stream registry

```text
stream_contract: mfm.portable-run-export-stream.v1
stream_schema_id: schema:mfm.portable-run-export-stream:1:sha256-jcs-v1:4fa9b2e8e090997c0ee69c70812059d2f9ea019391c02c86d7576037dc668d35
frame_contract: mfm.portable-run-export-frame.v1
frame_schema_id: schema:mfm.portable-run-export-frame:1:sha256-jcs-v1:88aab685674894648f7dd56f81a6f5b439c2f6beb566a1a64412ea79f3095572
```
