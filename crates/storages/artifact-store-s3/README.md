# mfm-artifact-store-s3

S3/MinIO `ArtifactStore` implementation (parity lane).

`put` uses SHA-256 content addresses and conditional create semantics. Rewriting an existing
artifact with different bytes is reported as corruption; same-byte writes are idempotent.
