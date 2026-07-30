# Recoverability v3 authority

`annex.json` and `corpus.json` are the sole current production recoverability
contract. Both files are exact, float-free canonical JSON bytes. No superseded
contract artifacts are retained or accepted.

This is a destructive lineage cutover. There is no compatibility parser,
database migration, checkpoint upgrade, dual reader, or fallback.

Portable run transfer uses `mfm.portable-run-export-stream.v2` with media type
`application/vnd.mfm.run-export-stream.v2+json-seq`. Its sole integrity value
is the external raw SHA-256 over every RS, canonical frame byte, and LF.

## Frozen artifact metadata

```text
annex_bytes: 227880
annex_sha256: 50cbcaa6185c36ebc652277a178108a8a3785285a5871f5d720e48d2c16dd0d7
annex_schema_count: 262
annex_invariant_clause_count: 248
corpus_bytes: 1741323
corpus_sha256: ec9220cca6dd0e7da1cfda485006d58353baa3470c3474aba20d782058efc942
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
stream_contract: mfm.portable-run-export-stream.v2
stream_schema_id: schema:mfm.portable-run-export-stream:2:sha256-jcs-v1:af2a60c5df624fe6e72ec212427ea243931073db00ef83e147383cb787c87723
frame_contract: mfm.portable-run-export-frame.v2
frame_schema_id: schema:mfm.portable-run-export-frame:2:sha256-jcs-v1:13c4988c86aa1b7bed600302ab29acf152b4a00074f980bd88f644ceac3f1fec
```
