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
`sha256-jcs-v1` digest, and opaque canonical bytes. Import atomically retains the revision and
makes it current for the name; importing a historical digest moves the marker without duplicating
content. Listing returns every retained revision in ascending name/digest order with exactly one
current marker per name. Documents remain bounded at 256 KiB, while the complete unpaginated
aggregate has no count bound. There is no delete operation. The sibling RunIndex port beside Store
defines a mechanical run summary containing only RunId, head sequence/digest, and cumulative bytes.
Run listing uses the last returned RunId directly as its exclusive ascending keyset continuation;
pages are not snapshots across requests. Config interpretation belongs to Application, and run
status remains a Runtime fold.

PostgreSQL configuration custody owns exactly `mfm_config_schema` and `config_revisions` under
`mfm.config-postgres.v1`. One backend, pool, and admission gate check run history and configuration
schema, ownership, ACL, and durability together; the fixed runtime role owns neither surface.

Application interprets retained bytes as one strict, complete, versioned config document. Its public
JSON contains only the entry-point tag, stable route selectors, and checked secret-free domain
input. Config revision summaries expose the checked name, canonical-document digest, entry point,
and current marker. Public config management exposes only import and complete listing. Current and
exact retained run-start selection never add config provenance to Journal or the mechanical
RunIndex: the exact Program and C0 remain the durable execution admission.

Public `RunView` contains RunId, durable sequence/head, and `Runnable`, typed `Succeeded`, or typed
`Failed`. Terminal retained values expose contract ref, instance ref, and exact canonical bytes.
Client JSON preserves that sum and embeds the terminal canonical bytes as a raw JSON value rather
than a quoted string.
Ambiguous start/progress acknowledgement is the only public error carrying data; both client
transports use the exact recovery envelopes frozen under `docs/contracts/client-surface/`.
Public surfaces never contain credentials, private keys, raw provider material, or unreviewed error
details.
