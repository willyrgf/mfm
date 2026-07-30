# Recoverability v1 authority

`annex.json` and `corpus.json` are the sole current production recoverability
contract. Both files are exact, float-free canonical JSON bytes. No superseded
contract artifacts are retained or accepted.

There is no compatibility parser, database migration, checkpoint upgrade,
dual reader, or fallback.

Portable run transfer uses `mfm.portable-run-export-stream.v1` with media type
`application/vnd.mfm.run-export-stream.v1+json-seq`. Its sole integrity value
is the external raw SHA-256 over every RS, canonical frame byte, and LF.

## Frozen artifact metadata

```text
annex_bytes: 227880
annex_sha256: 13d4b2c721daa20599c342d3bebd7bffcac9c44be4a09f998dce5f9081d50e6d
annex_schema_count: 262
annex_invariant_clause_count: 248
corpus_bytes: 1741323
corpus_sha256: 0af20cbb3d4217e1579f84c3fede302b0ceb2525bb6bed1c57beac02e75501ff
corpus_positive_case_count: 441
corpus_negative_case_count: 62
corpus_relational_case_count: 84
corpus_total_case_count: 587
corpus_schema_acceptance_case_count: 387
corpus_codec_rejection_case_count: 28
corpus_relational_rejection_case_count: 33
```

All 587 vectors are mandatory for each of the nine consumers named by
`corpus.json`.

## Portable stream registry

```text
stream_contract: mfm.portable-run-export-stream.v1
stream_schema_id: schema:mfm.portable-run-export-stream:1:sha256-jcs-v1:4fa9b2e8e090997c0ee69c70812059d2f9ea019391c02c86d7576037dc668d35
frame_contract: mfm.portable-run-export-frame.v1
frame_schema_id: schema:mfm.portable-run-export-frame:1:sha256-jcs-v1:88aab685674894648f7dd56f81a6f5b439c2f6beb566a1a64412ea79f3095572
```
