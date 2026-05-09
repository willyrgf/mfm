# mfm-artifact-store-fs

Filesystem `ArtifactStore` implementation (fast lane, service-free).

Artifacts are addressed by SHA-256 digest. Same-byte writes are idempotent, and attempts to
materialize different bytes at an existing content address are reported as corruption.
