# Persisted and public surfaces

Program v3 is one strict checked canonical document requiring `domain: "mfm.program.v3"`. It
contains entry point, admitted-context contract, exact root success/failure contracts, and an
ordered State/Match declaration array. There is no public wire DTO parallel to `Program`.

Journal persists only canonical `mfm.run.frame.v2` frames. Genesis records the exact Program and C0.
Later frames are a fused Pure conclusion, a fused Read intent/evidence/outcome conclusion, an Effect
prepare, or its immediately adjacent conclusion. A prepare may be the final record of a complete
valid prefix. Every referenced object appears exactly once in the frame-local sorted object closure.
Recursive heads use `content:sha256-v1` over exact canonical frame bytes.

`EncodedRunFrame` is sealed and exposes Store's read-only run/sequence/predecessor/head/byte
projections. `StoredRunBytes` is opaque unqualified transfer. `JournalHistory` is the sole qualified
complete prefix and exposes only borrowed semantic records/objects required by Runtime. No raw frame
parser, open Journal DTO, portable codec, or independent semantic record hash is public.

Store persists immutable frame bytes and one current head only. PostgreSQL owns exactly
`mfm_store_schema`, `mfm_run_frames`, and `mfm_run_heads`; the static schema contract is
`mfm.run-history-postgres.v2`. Those three relations remain the complete run-history authority even
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
`mfm.config-postgres.v2`. PostgreSQL EVM transaction authority owns exactly
`mfm_evm_tx_schema`, `nonce_domains`, `nonce_reservations`, `prepared_transactions`, and
`transaction_settlements` under `mfm.evm-transaction-postgres.v2`. `PostgresBackend` independently
gates only run history and configuration; optional `PostgresEvmTransactionAuthority` owns a second
pool and gates only the authority schema. The fixed runtime role owns none of them.

Authority records are non-Program, non-serde port values. A reservation retains EffectId, command
ref, nonce, and a domain consisting exactly of epoch, chain instance, and sender. A
prepared record adds only exact raw transaction bytes and transaction hash. A settled record nests
that predecessor and the qualified typed `EvmTransactionSettlement`. Raw bytes have no text/debug
surface. The database stores canonical settlement bytes but no command copy, action, endpoint,
provider response, timestamps, mutable status, or broadcast instruction.

Application interprets retained bytes as one strict, complete, versioned config document. Its public
JSON contains only the entry-point tag, stable route selectors, and checked secret-free domain
input. Config revision summaries expose the checked name, canonical-document digest, entry point,
and no mutable status. Public config management exposes import, complete listing, and exact delete.
Exact retained run-start selection never adds config provenance to Journal or the mechanical
RunIndex: the exact Program and C0 remain the durable execution admission.

Public `RunView` contains RunId, durable sequence/head, and `Runnable`, typed `Succeeded`, or typed
`Failed`. Terminal retained values expose contract ref, instance ref, and exact canonical bytes.
A selected State and a prepared pending Effect both render as `Runnable`; Effect command, evidence,
and identity are not added to the public view.
Client JSON preserves that sum and embeds the terminal canonical bytes as a raw JSON value rather
than a quoted string.

Signing public keys, digests, compact signatures, recovery IDs, private scalars, owner channels,
and signer handles are transient and have no persisted serde surface. EVM binds durable authority
to the sender address; a captured signer's checked public key is only a live witness for that
account.

EVM Program-visible transaction values are strict content-addressed values. The public chain
instance is `(nonzero chain_id, expected_genesis_hash)`; a transaction route adds one endpoint ref;
the Effect binding adds one checked sender and one exact 32-byte authority epoch.
`Eip1559TransactionCommand` contains only that binding, a bounded Create or
Call action, value, nonzero gas limit, and ordered fee pair. It contains no nonce, provider locator,
settlement policy, access list, timeout, or arbitrary metadata.

`EvmTransactionSettlement` retains EffectId, nonce, and a structured confirmation or revert that
contains the transaction hash and receipt block anchor. Workflow projections omit EffectId, nonce, command, raw
receipt, logs, and provider response. Anchored contract-call intents retain target, bounded calldata,
exact anchor, chain ID, operation ID, and transaction-route ref; returned evidence retains only the
anchor and bounded return bytes. Transient signing digests/signatures and raw-transaction
custody never enter these values.

Ambiguous start/progress acknowledgement is the only shared use-case error carrying data. Both
client transports use the same Application-owned error serializer and the exact recovery envelopes
frozen under `docs/contracts/client-surface/`; an identified REST start error may additionally carry
the already selected RunId.
Public surfaces never contain credentials, private keys, raw provider material, or unreviewed error
details.
