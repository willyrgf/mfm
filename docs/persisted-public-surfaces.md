# Persisted and Public Surfaces

Status: current authority and redaction inventory

## Material uncertainties

none

Persisted and public values are strict, bounded, float-free canonical JSON with denied unknown
fields unless the frozen schema explicitly says otherwise. A content reference identifies exact
bytes; it is never bearer authority.

## RunHistory

| Family | Persisted meaning |
| --- | --- |
| `RunAdmitted` | Store/tenant/run/invocation identity, the one certified-program reference, immutable admission material, initial lexical bindings, and genesis semantic digest. |
| `StateTransitionCommitted` | Exact occurrence/path/call, input binding, optional consumed observation, nominal outcome, facts, and before/after semantic digests. |
| `ExternalAccessAuthorized` | Exact occurrence, attempt ordinal, semantic head, capability/adapter contracts and implementations, request/digest, public binding, and optional stable resource lineage. |
| `ExternalAccessObserved` | Exact authorization/access-attempt linkage and one closed observation outcome. |
| `RunClosed` | Exact terminal nominal operation-outcome reference. |

Each record is assigned a run sequence, ordinal, and record hash inside a predecessor-linked atomic
batch. A batch also binds append request id, candidate digest, commit digest, store scope, writer
epoch, and the exact new object set. No control record exists for Match, FanOut, lanes, joins,
fragments, handlers, or result expressions.

## Observation outcomes

| Variant | Public/persisted fields | State-consumable |
| --- | --- | --- |
| `Returned` | Exact typed value reference | yes |
| `SafeFailure` | Exact reviewed typed failure reference | yes |
| `SupersededBeforeEntry` | Public lineage-head and evidence references | no; Effect refresh only |
| `EntryUnknown` | Fixed bounded fault code | no; parks Effect |
| `IntegrityFault` | Fixed bounded fault code | no; blocks |

Raw provider bodies, arbitrary diagnostic text, endpoints, credentials, signed bytes, private
sessions, fence keys, and mutation permits are never fields of these records or their object
closure.

## Persisted schema vocabulary

`mfm-values` owns the one shape-derived persisted-schema mechanism. A schema identity hashes its
schema kind, name, manual version, optional semantic type identity, fixed policies, and its
`PersistedEncoding`. Audit provenance stays outside the hash. There is no registry: a descriptor
names a closed grammar identifier and the checked Rust owner is the one implementation of that rule.

`SchemaKind::PersistedContract` marks a retained history/component contract that is not a state
value, planning config, state input, or operation output. Codec-only public DTOs have no schema
identity at all.

`PersistedEncoding` is closed:

| Encoding | Hashed elements |
| --- | --- |
| `CanonicalJson` | complete serialized shape |
| `CanonicalJsonLines` | record shape, minimum/maximum records, maximum framed record bytes, maximum stream bytes |

`CanonicalJsonLines` fixes LF delimiters and a required final LF; those are not configurable flags.
Each encoding validates its own form: `CanonicalJson` validates one canonical value against the
complete shape, and `CanonicalJsonLines` validates delimiters, record bounds, and every record
against the declared record shape. A stream owner therefore never restates its own framing rules.

The bounded-shape vocabulary is closed. Beyond the existing unit, boolean, string, bytes, integer,
decimal, option, sequence, tuple, struct, enum, string-map, inline-value, and generic forms, a
persisted shape may use:

| Form | Hashed bounds |
| --- | --- |
| `BoundedString` | inclusive UTF-8 byte bounds and one `StringGrammar` |
| `BoundedBytes` | inclusive decoded byte bounds |
| `UnsignedRange` / `SignedRange` | inclusive numeric range |
| `Literal` | one exact null/boolean/unsigned/signed/string value |
| `BoundedSequence` | element shape, inclusive cardinality, ordering, uniqueness |
| `BoundedStringMap` | key grammar and byte bounds, value shape, inclusive entry bounds |
| `CanonicalJsonTerminal` | one `CanonicalJsonProfile` |

`StringGrammar` is a closed enum whose variants each delegate to one checked owner:
`UnicodeScalarText`, `ContentDigest`, `SemanticDigest`, `RunId`, `OccurrenceId`, `SchemaId`,
`SemanticTypeId`, `EntryPointId`, `StableId`, `StoreScopeId`, `TenantScopeId`, `UuidV4`,
`CanonicalUnsignedText`, `LowerPathToken`, and `MediaType`. `mfm_values::MediaType` is the one owner
of the lowercase registered media-type grammar without parameters.

`CanonicalJsonProfile` has two profiles: `GeneralFloatFree` for framework surfaces that deliberately
admit signed and unsigned integers, and `UnsignedNative` for the fact subject/predicate domain. Both
enforce the global canonical byte, depth, string, key, array, and object limits, and both scan every
string value under the persisted-surface secret-marker policy. Object keys are judged as structural
names, exactly as declared struct field names are, so a name such as `authorization_ref` is admitted
while a `"Bearer …"` value is not.

Changing any field, tag, literal, bound, grammar, number profile, referenced shape, framing rule, or
manual version changes the derived `SchemaId`; changing `SchemaAudit` provenance alone does not.

Retained owners declare their shape with `#[derive(PersistedSchema)]` rather than restating it, so a
contract cannot drift from the bytes it serializes. Checked identity fields map to their closed
grammar, a nested persisted owner embeds its own declared shape, and each derived identity and
schema id is a cached one-time constant. Retained documents whose semantic legality is established
by certification or expansion — the authored and expanded programs, policy recipes, lane and join
contracts, and configured values — declare the bounded canonical-JSON terminal instead of a flat
field list; that terminal still enforces the float-free profile and the global canonical bounds.

## Semantic-hash owners

Every domain-separated identity is minted by exactly one owner from one exact preimage. The
framework envelope is `SHA-256(JCS({"domain": D, "value": V}))`; the two byte-concatenation forms
are noted where they differ. There is no generic public "hash anything under a caller-supplied
domain" helper.

| Domain | Owner | Exact preimage |
| --- | --- | --- |
| `mfm.run-id.v1` | `mfm_journal::structured::derive_run_id` | store scope, tenant scope, entry-point operation id, invocation identity |
| `mfm.fact-content-identity.v1` | `derive_fact_content_identity` | fact descriptor ref, subject ref, response ref |
| `mfm.fact-logical-identity.v1` | `derive_fact_logical_identity` | producer transition ref, emission ordinal, fact content identity |
| `mfm.fact-query.v1` | `mfm_facts::derive_fact_query_digest` | exact canonical request bytes, byte-concatenated after the domain |
| `mfm.structured-candidate.v1` | `derive_candidate_digest` | exact canonical candidate bytes, byte-concatenated after the domain |
| `mfm.structured-access-attempt.v1` | `derive_access_attempt_id` | exact canonical attempt preimage, byte-concatenated after the domain |
| `mfm.evm.qualified-chain-instance.v1` | `derive_qualified_chain_instance_id` | chain instance declaration |
| `mfm.evm.chain-lineage.v1` | `derive_evm_chain_lineage_id` | qualified chain instance |
| `mfm.evm.wallet-nonce-domain.v1` | `derive_wallet_nonce_domain` | chain lineage id and lowercase hex sender address |
| `mfm.evm.intent-issuer.v1` | `derive_authenticated_intent_issuer_id` | authenticated issuer material |
| `mfm.evm.submission-intent.v3` | `derive_submission_intent_id` | issuer, nonce domain, submission semantics |
| `mfm.evm.submission-semantics.v1` | `derive_submission_semantics_digest` | exact submission semantics |
| `mfm.evm.nonce-reservation.v1` | `derive_evm_nonce_reservation_key` | nonce domain and submission intent |
| `mfm.evm.nonce-candidate.v1` | `derive_evm_candidate_operation_key` | reservation key and candidate ordinal |
| `mfm.evm.nonce-completion.v1` | `derive_evm_nonce_completion_key` | reservation key |
| `mfm.evm.transaction-intent.v1` | `EvmTransactionIntent::digest` | exact transaction intent |
| `mfm.evm.candidate-family.v1` | `EvmCandidateFamily::digest` | exact candidate family |

Semantic digests use `sha256-jcs-v1`; exact retained-byte content addressing uses `sha256-v1`. The
two are distinct identity kinds and neither substitutes for the other.

## Bound limits

Every bound is owned by the layer that enforces it; there is no central cross-domain budget module.

| Owner | Bounds |
| --- | --- |
| `mfm_canonical::limits` | canonical JSON bytes, depth, string bytes, object key bytes, object entries, array items, base64url characters |
| `mfm_store::structured` | stored frame bytes, batch objects, batch records, configuration revision bytes, export source runs, export fact routes |
| `mfm_replay::portable` | portable stream bytes, framed record bytes, frames, batches, objects |
| `mfm_journal::structured` | prior-run source rules, programs and descriptors per rule, total references, manifest bytes |
| `mfm_evm::wallet_authority` | completion recovery bytes, provider message bytes, provider proof bytes, provider deployment routes, provider finish-authorization bytes |
| `mfm_facts` | eight fact-scan bounds, selection queries, selection limit, fact emissions |

## Content-addressed objects

Every `HistoryObject` contains an object-type tag, schema id plus raw canonical-byte digest, and
the exact canonical JSON. A retained owner is built and read through
`HistoryObject::from_persisted`/`decode_persisted`, which bind one `HistoryObjectPayload` to both
its declared schema identity and its one checked object type, so typed bytes cannot be paired with
a foreign object kind. The store validates bytes and identity on admission and load. Each append
must introduce exactly the newly reachable closure required by its records—no missing member and no
unreferenced extra object.

The closure includes certified program components, immutable admission roots, lexical values,
state outcomes, facts, access request/response values, and purpose-limited public physical
evidence. Secret-free implementation descriptors identify semantic contract, implementation id,
executable identity, and qualification artifact; they never identify credentials or endpoints.

## Facts

A committed fact binds dense emission ordinal, certified slot ordinal, descriptor, exact typed
subject/response, and claim reference. Facts are transition members, not an independent mutable
table of semantic truth. Backend indexes are rebuildable projections only.

## Configuration history

Configured values use a separate append-only stream keyed by store, tenant, entry operation, and
target. A revision contains sequence, predecessor, append request id, exact configured-value
contract, content reference, canonical bytes, and writer lineage. `ConfigurationRevision` has one
owner-derived `mfm.structured-configuration-revision` schema identity; admission material and the
configuration store share it, so the same revision bytes cannot carry two derivations. Application paths can resolve but
cannot append. The shared append boundary bounds the canonical serialized revision before any
backend receives it, so memory and PostgreSQL accept the same content-addressed revision bytes.
Configuration is not a RunHistory record family.

## Certified program

The certified root content-addresses the exact authored program, expanded program, expansion
profile/proof, policy proof, qualified entry-point admission policy,
state/capability/adapter/signer/resource manifest closure, and secret-free implementation manifest.

`CertifiedProgramRoot` has one owner-derived `mfm.certified-program-root` canonical-JSON schema, and
its reference is that schema identity plus the raw SHA-256 digest of its exact JCS bytes. That single
`RunAdmitted.certified_program_ref` is simultaneously the program identity, the retained root-object
key, the prior-run authorization identity, and the export identity. There is no second bespoke
digest and no repeated audit projection on `RunAdmitted`: everything behind the reference is exactly
`root.components`. Serialized authored or component bytes have no authority independently.

## Wallet authority

Wallet PostgreSQL persists only current-schema public semantic state:

- permanent chain/domain activation and store-incarnation lineage;
- stable intent/reservation identity and canonical intent closure;
- nonce high water and one incomplete intent;
- bounded candidate family and contiguous activated prefix;
- permanent operation-key results; and
- canonical terminal completion closure.

Target-held private keys, live sessions, transaction permits, database passwords, signer secrets,
signatures, and signed transaction bytes are process/deployment authority and are not persisted in
semantic tables. SQL role and session metadata is infrastructure, not portable semantic evidence.

Wallet authority retains no derivative evidence reference. The former
`reservation_evidence_ref`, `activation_evidence_ref`, `completion_evidence_ref`,
`winning_activation_evidence_ref`, `predecessor_activation_ref`,
`observed_floor_ref`, and `original_terminal_witnesses_ref` digests restated
material the closure already retains exactly, so they are gone and there is no
replacement identity. The authoritative bindings are the permanent operation
keys — `EvmNonceReservationKey`, `EvmCandidateOperationKey` (now carried on
`ActiveWalletCandidate` and on the replacement permit), and
`EvmNonceCompletionKey` — over the exact retained request, state input, and
result, plus the signed `ProviderMutation`. The winner is selected by ordinal and
transaction hash against the exact retained prefix. The provider protocol is v4.

## Public application DTOs

The reviewed application surfaces are:

- published entry points (`mfm.published-entry-point.v1`), which are output metadata with no
  retained-value contract, content reference, or decoder;
- admission response with logical run identity and admission disposition;
- one-action drive response;
- public run view with verified status/head and terminal output when closed;
- fixed-head transition-trace pages;
- fixed-head access-audit pages;
- callback-free replay results; and
- streamed current structured portable exports.

All JSON rendering uses a strict one-current schema. Cursor and content-reference strings are
opaque identity, not authorization. Every protected method independently authenticates and
authorizes its exact purpose and tenant/run target.

## Portable export

The current media type is:

```text
application/vnd.mfm.structured-run-export-stream.v3
```

The complete stream is the one portable identity: `mfm-replay` owns it as a `CanonicalJsonLines`
persisted schema, and `encode()` returns the exact bytes and their `ContentRef` together so the two
can never be paired across streams. A frame is an internal typed record of that one codec, carries
no identity of its own, and binds only a kind and a payload.

The terminal seal binds version, requested semantic/audit kind, every run fixation, source closure,
per-run authenticated principal, fixed `export` grant, content-addressed policy-decision references,
and assigned prefixes. It deliberately restates no frame ordinal, predecessor digest, chain digest,
frame count, or byte count: physical line order, the caller's expected complete-stream `ContentRef`,
and the canonical batch predecessor chains already own those facts.

`verify_offline(bytes, expected_content_ref, trust)` compares the caller's expected complete-stream
reference against the bytes before decoding anything, then checks the closed batch/seal union,
framing, bounds, terminal placement, source closure, fixation, and seal. Old bytes are rejected; no
v2 decoder exists.

Semantic exports authorize only the exact selected cutoff and recursively required producer heads;
later audit-only suffixes are not semantic dependencies. The retained decision references are
opaque content-addressed policy evidence, and offline verification accepts them only when its
explicit trust snapshot binds the exact closure digest.

## Error and logging contract

CLI/REST/application errors use a non-empty code bounded to 128 UTF-8 bytes and a non-empty reviewed
message bounded to 4096 UTF-8 bytes. Transport class is carried by HTTP status or process exit
status, not JSON. A Runtime-classified error may additionally carry the frozen, secret-free
`runtime_fault` attribution: phase, run, nullable verified pre-fault head, nullable occurrence, and
either the semantic process contract or store scope and epoch. This attribution is public context,
not persisted history or bearer authority; it excludes private implementation identity and
diagnostics. JSON uses `mfm.public-error.v1` inside `mfm.error-response.v1`.

Typed library errors may preserve internal sources, but the public boundary classifies and redacts
them. Logs contain route, method, status, bounded latency, and reviewed operational identifiers
only. They must not contain:

- credentials, passwords, mnemonics, private keys, API tokens, or headers;
- endpoints, filesystem paths, database URLs, raw SQL parameters, or provider response text;
- signature material or signed transaction bytes; or
- arbitrary `Debug` output from secret-bearing/private authority values.

## Review checklist

For every new persisted or public field:

1. identify its semantic owner and exact schema;
2. prove it is canonical, bounded, float-free, strict, and content-addressed where required;
3. prove it is public evidence rather than a disguised credential or bearer;
4. add hostile unknown-field, contract/provenance substitution, and redaction tests; and
5. reject old bytes rather than adding a compatibility path.
