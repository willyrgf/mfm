# Persisted and public surfaces

Program v5 is one strict checked canonical document requiring `domain: "mfm.program.v5"`. It
contains the entry point, admitted-context contract and exact initial value ref, root success/failure
contracts, and an ordered State sequence with selected recovery policies, maps, checkpoints, and
finite lifecycle bounds. There is no public wire DTO parallel to `Program`.

Journal persists only canonical `mfm.run.frame.v3` frames. Genesis records the exact Program and C0.
Later frames are a fused Pure conclusion, a fused Read intent/evidence/outcome conclusion, an Effect
prepare, or its immediately adjacent conclusion. Conclusions retain the execution position and visit,
original domain failure or typed operational error/context, and the atomic recovery decision. A Stop
also retains the mapped root failure when applicable. A prepare may be the final record of a complete
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
`mfm_evm_tx_schema`, `nonce_reservations`, and `prepared_transactions` under
`mfm.evm-transaction-postgres.v2`. `PostgresBackend` gates only run history and configuration;
optional `PostgresEvmTransactionAuthority` owns a separate pool gated for custody. The runtime role
owns none of them.

Public reservation and stage descriptors are checked Program values. A reservation retains the
reserve EffectId, original command ref, nonce, and domain of epoch, chain instance, and sender.
Opaque custody records contain reservation plus optional exact raw bytes and transaction hash;
raw bytes have no serde, text, or debug surface. Journal retains public stage evidence and final
settlement, never raw wire. The database has no settlement, command copy, action, endpoint, provider
response, timestamps, mutable status, or broadcast instruction.

Application interprets retained bytes as one strict, complete, versioned config document. Its public
JSON contains only the entry-point tag, stable route selectors, and checked secret-free domain
input. Config revision summaries expose the checked name, canonical-document digest, entry point,
and no mutable status. Public config management exposes import, complete listing, and exact delete.
C0 retains the selected source revision name, entry point, and exact canonical-document digest as
64 lowercase hex characters. Enriched snapshot config and C0 additionally retain the enrichment
RunId, terminal head, and exact output ref. Application verifies that linkage before new admission.
The exact Program and C0 remain the durable execution admission; RunIndex remains mechanical.

Public `RunView` contains RunId, durable sequence/head, and one of `Runnable`, `EffectPending`,
`Succeeded`, or `Failed`. Runnable retains position and Advance/Retry/Restart reason; a pending
Effect retains position and EffectId. Success exposes the output contract ref, instance ref, and exact
canonical bytes. Failure exposes the content ref and canonical FailureReport, including the original
failure or operational error/context, Stop reason, position, and recovery usage.
Client JSON preserves that sum and embeds retained canonical bytes as raw JSON values.
An invocation failure is separate from durable run failure: execution errors retain the last observed
view when available, while pending recovery errors retain the observed view and typed error/context.

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

`EvmTransactionSettlement` retains EffectId, nonce, one shared transaction receipt, and a closed
Created, Called, or Reverted outcome. The generic transaction completion retains caller context
and complete checked command, reservation, preparation, and settlement facts. Transaction failure
preserves the facts completed before failure, including the checked receipt on reversion. No raw
transaction bytes or unreviewed provider response enter these projections. Anchored contract-call intents retain target, bounded calldata,
exact anchor, chain ID, operation ID, and transaction-route ref; returned evidence retains only the
anchor and bounded return bytes. Transient signing digests/signatures and raw-transaction custody
never enter these values.

Ambiguous start/progress acknowledgement carries exact recovery identity and the last observed
view, or null if none was observed. Other invocation errors expose their reviewed execution or
recovery detail. Both client transports use the same Application-owned error serializer and recovery envelopes
frozen under `docs/contracts/client-surface/`; an identified REST start error may additionally carry
the already selected RunId.
Public surfaces never contain credentials, private keys, raw provider material, or unreviewed error
details.
