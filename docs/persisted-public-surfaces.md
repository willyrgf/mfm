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
when the sibling `mfm_config` schema is installed.

The configuration repository port defines an immutable revision containing only a checked name, a
`sha256-jcs-v1` digest, and opaque canonical bytes. Import atomically creates or compares the exact
name/digest revision; different digests under one name remain independent. Load and idempotent
delete require that exact pair. Listing returns every revision in ascending name/digest order.
Documents remain bounded at 256 KiB, while the complete unpaginated aggregate has no count bound.
The sibling RunIndex port beside Store
defines a mechanical run summary containing only RunId, head sequence/digest, and cumulative bytes.
Run listing uses the last returned RunId directly as its exclusive ascending keyset continuation;
pages are not snapshots across requests. Config interpretation belongs to Application, and run
status remains a Runtime fold.

PostgreSQL configuration custody owns exactly `mfm_config_schema` and `config_revisions` under
`mfm.config-postgres.v2`. One backend, pool, and admission gate check run history and configuration
schema, ownership, ACL, and durability together; the fixed runtime role owns neither surface.

Application interprets retained bytes as one strict, complete, versioned config document. Its public
JSON contains only the entry-point tag, stable route selectors, and checked secret-free domain
input. Config revision summaries expose the checked name, canonical-document digest, entry point,
and no mutable status. Public config management exposes import, complete listing, and exact delete.
Exact retained run-start selection never adds config provenance to Journal or the mechanical
RunIndex: the exact Program and C0 remain the durable execution admission.

Public `RunView` contains RunId, durable sequence/head, and `Runnable`, typed `Succeeded`, or typed
`Failed`. Terminal retained values expose contract ref, instance ref, and exact canonical bytes.
Client JSON preserves that sum and embeds the terminal canonical bytes as a raw JSON value rather
than a quoted string.
Ambiguous start/progress acknowledgement is the only shared use-case error carrying data. Both
client transports use the same Application-owned error serializer and the exact recovery envelopes
frozen under `docs/contracts/client-surface/`; an identified REST start error may additionally carry
the already selected RunId.
Public surfaces never contain credentials, private keys, raw provider material, or unreviewed error
details.
