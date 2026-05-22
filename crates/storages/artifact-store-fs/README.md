# mfm-artifact-store-fs

Filesystem typed artifact store implementation (fast lane, service-free).

`FsTypedArtifactStore` is the certified local artifact store for typed runs. It stores immutable
artifact bytes by content digest and persists the evidence required by typed commit validation:
byte length, media type, schema id, semantic id, producer node or seed, and artifact role.

Seed material must be persisted with `SeedInput` evidence and the matching `producer_seed_id`.
Missing or mismatched seed artifacts are rejected before they can satisfy run-start evidence.

`FsArtifactStore` remains only as the legacy `mfm_machine::stores::ArtifactStore` implementation
for old runtime tests and compatibility callers. It is not a certified typed submit/resume surface.
