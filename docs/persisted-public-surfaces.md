# Persisted and public surfaces

Program v2 is one strict checked canonical document. It contains entry point, admitted-context
contract, exact root success/failure contracts, and an ordered State/Match declaration array. There
is no public wire DTO parallel to `Program`.

Journal persists only canonical `mfm-run-frame@1` frames. Genesis records the exact Program and C0.
Later frames are either one fused Pure conclusion or one fused Read intent/evidence/outcome
conclusion. Every referenced object appears exactly once in the frame-local sorted object closure.
Recursive heads use `content:sha256-v1` over exact canonical frame bytes.

`EncodedRunFrame` is sealed and exposes Store's read-only run/sequence/predecessor/head/byte
projections. `StoredRunBytes` is opaque unqualified transfer. `JournalHistory` is the sole qualified
complete prefix and exposes only borrowed semantic records/objects required by Runtime. No raw frame
parser, open Journal DTO, portable codec, or independent semantic record hash is public.

Store persists immutable frame bytes and one current head only. PostgreSQL owns exactly
`mfm_store_schema`, `mfm_run_frames`, and `mfm_run_heads`; the static schema contract is
`mfm.run-history-postgres.v1`.

Public `RunView` contains RunId, durable sequence/head, and `Runnable`, typed `Succeeded`, or typed
`Failed`. Terminal retained values expose contract ref, instance ref, and exact canonical bytes.
Public surfaces never contain credentials, private keys, raw provider material, or unreviewed error
details.
