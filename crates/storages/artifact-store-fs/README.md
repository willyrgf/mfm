# mfm-artifact-store-fs

Filesystem typed artifact store implementation (fast lane, service-free).

`FsTypedArtifactStore` is the certified local artifact store for typed runs. It stores immutable
artifact bytes by content digest and persists the evidence required by typed commit validation:
byte length, media type, schema id, semantic id, producer node or seed, and artifact role.

Seed material must be persisted with `SeedInput` evidence and the matching `producer_seed_id`.
Missing or mismatched seed artifacts are rejected before they can satisfy run-start evidence.

No legacy dynamic artifact-store trait is exposed from this crate.
