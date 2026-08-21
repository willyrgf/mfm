# Persisted and public surfaces

Program v2 is one strict checked canonical document. It contains entry point, admitted-context
contract, exact root success/failure contracts, and an ordered State/Match declaration array. There
is no public wire DTO parallel to `Program`.

Journal persists only canonical `mfm.run.frame.v1` frames. Genesis records the exact Program and C0.
Later frames are either one fused Pure conclusion or one fused Read intent/evidence/outcome
conclusion. Every referenced object appears exactly once in the frame-local sorted object closure.
Recursive heads use `content:sha256-v1` over exact canonical frame bytes.

`EncodedRunFrame` is sealed and exposes Store's read-only run/sequence/predecessor/head/byte
projections. `StoredRunBytes` is opaque unqualified transfer. `JournalHistory` is the sole qualified
complete prefix and exposes only borrowed semantic records/objects required by Runtime. No raw frame
parser, open Journal DTO, portable codec, or independent semantic record hash is public.

Store persists immutable frame bytes and one current head only. PostgreSQL owns exactly
`mfm_store_schema`, `mfm_run_frames`, and `mfm_run_heads`; the static schema contract is
`mfm.run-history-postgres.v1`. Those three relations remain the complete run-history authority even
when the independent `mfm_catalog` schema is installed.

The separate catalog/index port defines a bounded named config custody record containing only a
checked name, a `sha256-jcs-v1` digest, and opaque canonical bytes. It also defines a mechanical run
summary containing only RunId, head sequence/digest, and cumulative bytes. Resource-specific
unpadded-base64url cursors freeze ascending bytewise keyset traversal; pages are not snapshots across
requests. Config interpretation belongs to Application, and run status remains a Runtime fold.
PostgreSQL catalog custody owns exactly `mfm_catalog_schema` and `config_entries` under
`mfm.config-catalog-postgres.v2`. The storage adapter checks schema, ownership, ACL, and durability
independently for run history and catalog; the fixed runtime role owns neither surface.

Application interprets catalog bytes as one strict, complete, versioned config document. Its public
JSON contains only the entry-point tag, stable route selectors, and checked secret-free domain
input. Config summaries expose the checked name, canonical-document digest, and entry point;
config reads embed the retained canonical document as a raw JSON value. Current and exact run-start
selection never add config provenance to Journal or the mechanical RunIndex: the exact Program and
C0 remain the durable execution admission.

Public `RunView` contains RunId, durable sequence/head, and `Runnable`, typed `Succeeded`, or typed
`Failed`. Terminal retained values expose contract ref, instance ref, and exact canonical bytes.
Client JSON preserves that sum and embeds the terminal canonical bytes as a raw JSON value rather
than a quoted string.
Ambiguous start/progress acknowledgement is the only public error carrying data; both client
transports use the exact recovery envelopes frozen under `docs/contracts/client-surface/`.
Public surfaces never contain credentials, private keys, raw provider material, or unreviewed error
details.
