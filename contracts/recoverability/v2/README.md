# Recoverability v2 authority

`annex.json` and `corpus.json` are the sole current production recoverability
contract. Both files are exact, float-free canonical JSON bytes. The complete
v1 directory is retained byte-identically only as an archival and hostile-input
reference; production consumers must not load it.

The initial local v2 freeze named the absent `emission_ordinal` field as the
ordering key for certified fact slots, which rejected every nonempty
`fact_slots` value. Because that freeze was unpublished and no v2 data had
been persisted, these corrected bytes replace it in place as the sole v2
authority. There is no compatibility identity or fallback, and v1 remains
byte-identical.

Portable run transfer uses the registered framed sequence
`mfm.portable-run-export-stream.v1` with media type
`application/vnd.mfm.run-export-stream.v1+json-seq`. Its sole integrity value
is the external raw SHA-256 `ContentRef` over every RS, canonical frame byte,
and LF. There is no bundle, manifest, member path, or internal stream digest.

## Frozen artifact metadata

```text
annex_bytes: 223652
annex_sha256: bf1065f32a8249f69b9a82f19be2a221d1db5b683fa439ec6f701c42666b3ff8
annex_schema_count: 258
annex_invariant_clause_count: 233
corpus_bytes: 1728876
corpus_sha256: a2b249e054e7c6e85edfd91a6f3c6c9fc9588f65a9f3dfbfe0eac08f9f0be7a9
corpus_positive_case_count: 434
corpus_negative_case_count: 60
corpus_relational_case_count: 84
corpus_total_case_count: 578
corpus_schema_acceptance_case_count: 380
corpus_codec_rejection_case_count: 28
corpus_relational_rejection_case_count: 32
```

All 578 vectors are mandatory for each of the nine consumers named by
`corpus.json`.

## Portable stream registry

```text
stream_contract: mfm.portable-run-export-stream.v1
stream_schema_id: schema:mfm.portable-run-export-stream:1:sha256-jcs-v1:4fa9b2e8e090997c0ee69c70812059d2f9ea019391c02c86d7576037dc668d35
frame_contract: mfm.portable-run-export-frame.v1
frame_schema_id: schema:mfm.portable-run-export-frame:1:sha256-jcs-v1:88aab685674894648f7dd56f81a6f5b439c2f6beb566a1a64412ea79f3095572
```
