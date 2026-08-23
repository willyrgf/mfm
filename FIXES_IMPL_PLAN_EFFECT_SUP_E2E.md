# Corrective Effect support and EVM contract lifecycle e2e implementation plan

## Status

Corrective implementation is complete on `effect-w-e2e` as seven ordered lower-case commits. The
focused commit verifications and final composed gate are recorded in the implementation handoff.
This document remains the completed corrective design and audit checklist.

This plan targets the current unmerged candidate at 6d559d5a on branch effect-w-e2e, consisting of
the six commits from 8f3d215b through 6d559d5a. It supersedes only the conflicting corrective
decisions in IMPL_PLAN_EFFECT_SUP_E2E.md. The original plan remains useful for the already-built
Effect, signing, EVM, authority, and managed-fixture contracts, and its status now records that both
the original implementation and this corrective follow-up are complete.

The baseline review covered AGENTS.md, docs/code-quality.md, docs/design.md,
docs/architecture.md, docs/build-and-verification.md, the complete original implementation plan,
all six commits, and the affected Runtime, signing, keystore, EVM domain, live EVM, PostgreSQL, app
test, documentation, and Nixfied surfaces. A dedicated Effect architecture review resolved the
general corrective design. A second, signing-only architecture review then traced every signing
consumer and resolved whether crates/signing should remain, what its minimum contract is, and what
persisted identity can be deleted. The decisions below are the resulting single target design.

At corrective-planning handoff, the reviewed baseline changed 90 files with 14,140 additions and
920 deletions relative to origin/dev:

| Surface | Additions | Deletions |
| --- | ---: | ---: |
| Production and build code | 7,162 | 635 |
| Tests and fixtures | 6,583 | 123 |
| Documentation | 395 | 162 |

The corrective work is not a request to minimize the diff at the expense of durability,
cryptographic safety, exact persistence qualification, or independent on-chain proof. It removes
accidental concepts, misleading semantics, duplicated projections, mandatory development-only
coupling, and low-value test scaffolding while retaining the safety-critical core.

## Outcome

After this cut:

- one authored EVM transaction Effect still represents one semantic transaction;
- nonce reservation, signing, exact raw custody, submission, and settlement remain internal
  append-only phases behind that one Effect;
- successful submission without terminal evidence is normal non-error Pending progress and returns
  a Runnable RunView;
- crates/signing remains only as a small transient recoverable-secp256k1 capability port, while
  KeystoreSigner is its purpose-bound in-process implementation;
- EVM persists the Ethereum sender, not a signer implementation route, algorithm tag, public key,
  canonical signer reference, or wallet wrapper;
- EVM confirmation, revert, and settlement values use one current non-duplicated closed sum;
- the optional development EVM transaction authority has its own PostgreSQL handle, pool, gate, and
  provisioner instead of being mandatory for the production Store/config backend;
- managed-Reth settlement uses one fully validated receipt and one canonical-block equality check,
  without implying finality through duplicate immediate observations;
- the fixture Program begins directly at the deployment transaction context, carries no ABI
  calldata through persisted workflow context, and derives the report from Read evidence;
- progression is exercised by a real maximum-eight-invocation loop;
- high-value ambiguity, cancellation, concurrency, crypto, authority, wire, and independent Reth
  tests remain; and
- every authoritative document describes the one current design.

## Frozen architecture decisions

| Question | Corrective decision | Explicitly excluded |
| --- | --- | --- |
| EVM mutation topology | One ExecuteEvmTransaction Effect per semantic transaction | A reservation Effect plus a submission Effect |
| Nonce reservation | Adapter-internal append-only authority phase | Program-visible reservation State |
| Capability injection | CapabilityInjection and InjectionWriter remain Pure-only; transaction injection remains identity topology | Injected Effect, Read, Operation, Match, or IO |
| Effect progress | Runtime-owned EffectAdapterOutcome with Pending and Settled | Encoding normal progress as Unavailable |
| Journal/Program wire | Keep Program v3, Journal frame v2, and current EffectId derivation | Another generic wire version without a wire change |
| Signing capability | Secp256k1Signer is a purpose-bound transient live capability; Keystore is one concrete provider | Persisted signing values, Keystore as Program State/Effect, per-call purpose, or a generic algorithm framework |
| EVM account identity | Chain instance plus sender; a live public key is only a transient witness of control | Signer route, algorithm tag, public key, wallet wrapper, or signer content ref in Program/authority persistence |
| Transaction values | Confirmation and revert are reused directly by settlement and workflow projections | Parallel terminal-result and confirmation-result enums |
| PostgreSQL composition | Separate core and optional authority handles, pools, gates, and provisioning calls | Mandatory mfm_evm_tx in production core; another locator grammar or crate |
| Development receipt policy | One validated receipt plus one canonical block equality check | Duplicate immediate reads presented as confirmation or finality |
| Production finality | Transaction capability remains absent from production composition | Reusing the development capability for production |
| E2E admission | Direct EvmTransactionContext C0 | Fixture input wrapper and PrepareDeployment State |
| E2E calldata | Fixed fixture constants independently checked against solc | Persisting configure/value calldata in caller context |
| Test reduction | Remove duplicated fixtures and implementation-count assertions | Removing boundary, hostile-input, ambiguity, crypto, or on-chain proofs |

The original phrase "nonce reservation via State injection from capabilities" is superseded by this
decision. A deterministic injected Pure State cannot reserve a nonce because reservation performs
persistence IO. An injected Effect would create a second EffectId and restore the rejected
two-Effect design. Capability injection selects deterministic support topology; durable nonce
authority remains inside the adapter for the one transaction Effect.

## Primary implementation map

| Workstream | Primary current files |
| --- | --- |
| Generic Effect outcome and fold | crates/kernel/runtime/src/lib.rs, assembly.rs, engine.rs |
| Runtime regression fixtures | crates/kernel/runtime/tests/runtime_contract.rs |
| Transient signing port | crates/signing/src/lib.rs, Cargo.toml, tests, and README.md |
| Thread-affine custody | crates/keystore/src/lib.rs and crates/keystore/tests/api_contract.rs |
| EVM value cut | crates/domains/evm/src/transaction.rs, src/lib.rs, tests/transaction_contract.rs |
| Authority port | crates/domains/evm-transaction-authority/src/lib.rs and its tests/README |
| PostgreSQL authority | crates/storages/postgres/src/evm_tx.rs, lib.rs, provision.rs, tests.rs, migrations |
| Live transaction orchestration | crates/live/evm/src/transaction.rs and transaction_tests.rs |
| EIP-1559 hostile input | crates/live/evm/src/codec.rs and its consuming tests |
| Persisted-type derive cleanup | crates/kernel/program-derive/src/lib.rs and affected tests/README |
| JSON-RPC mapping | crates/live/evm/src/json_rpc.rs, json_rpc_tests.rs, lib.rs, README.md |
| Managed lifecycle proof | crates/app/tests/evm_contract_effect_e2e.rs and its Solidity fixture |
| Managed task graph | nixfied.nix and docs/build-and-verification.md |
| Authoritative contracts | docs/design.md, architecture.md, run-execution.md, persisted-public-surfaces.md, evm-rpc-routing.md, known-gaps.md |

The engineer should confirm the current path/symbol with rg before editing; this table names
ownership, not an exhaustive permission to change adjacent modules.

## Non-goals

This corrective cut does not add:

- a new Program or Journal wire version;
- another EffectId derivation or capacity rule;
- injected IO or injected Effects;
- Store semantics or Store API methods;
- production EVM transaction registration, finality, replacement, fee bumping, or background work;
- persistent/encrypted key recovery;
- authority rollback, nonce release, or writable restoration;
- another database server, locator grammar, Cargo feature, storage crate, or third-party dependency;
- a Reth-forwarding acknowledgement-loss proxy;
- a reusable public contract-lifecycle Operation; or
- compatibility with the superseded EVM values or authority v1 baseline.

## Preserved safety and ownership invariants

The engineer must treat these as non-regression requirements:

1. State implementations are deterministic and perform no ambient IO.
2. Runtime owns the sole Program-aware semantic fold.
3. Journal owns exact frame wire, closure, adjacency, and complete-history qualification.
4. Store remains a mechanical exact-head append and complete-load port with no Effect or EVM
   semantics.
5. Runtime appends the exact command and derived EffectId before any Effect adapter call.
6. An ambiguous prepared-frame append causes zero adapter calls in that invocation.
7. A retained prepare is cold-qualified by deterministic re-preparation and exact command/EffectId
   comparison.
8. Every retry uses the same command and reserved nonce. Once a Prepared authority fact exists,
   every retry uses its same signature, transaction hash, and exact raw bytes; before Prepared,
   no locally produced raw bytes may reach a provider.
9. Exact raw bytes are retained before provider submission.
10. No database transaction or lock spans signer or provider IO.
11. A nonce domain has at most one unsettled reservation.
12. Confirmed success and confirmed revert both consume the nonce; unresolved observations never
    release it.
13. Same-Effect concurrency converges through exact retained facts, not a Runtime mutex.
14. Adapter Pending, adapter failure, panic, or cancellation appends no Effect conclusion.
15. Runtime performs no background retry, sleep, polling, or timeout policy.
16. Secrets and raw signed transactions never enter Program, C0, Journal, RunView, public output,
    errors, or logs.
17. The development transaction capability remains absent from ComposedRuntime, configuration,
    CLI, and REST.
18. Current-only cutovers delete superseded APIs, schemas, tests, fixtures, and docs; no aliases,
    legacy decoder, migration, feature flag, or fallback remains.

## 1. Represent nonterminal Effect progress explicitly

### Public Runtime contract

Add one Runtime-owned public sum:

~~~rust
pub enum EffectAdapterOutcome<E> {
    Pending,
    Settled(E),
}
~~~

Change only Effect adapter callbacks:

~~~rust
Result<EffectAdapterOutcome<C::Evidence>, AdapterError>
~~~

Read adapter callbacks remain Result<Evidence, AdapterError>. Do not add a base capability trait or
make Read use Effect progress semantics.

The alternatives mean exactly:

- Pending: the adapter has safely made or observed nonterminal progress, but has no terminal typed
  evidence. Runtime retains the existing Effect prepare, appends nothing, stops this start/resume
  invocation, and returns Ok with RunViewState::Runnable at the current prepared head.
- Settled(evidence): Runtime qualifies and binds evidence, interprets it, appends the adjacent
  Effect conclusion, and continues its existing advancement loop.
- AdapterError::Unavailable: a real dependency, transport, provider-ingress, or ambiguous-authority
  failure. Runtime returns RuntimeError::Unavailable and appends no conclusion.
- AdapterError::Internal: a local trusted-contract failure. Runtime returns RuntimeError::Internal
  and appends no conclusion.

Pending is not persisted as another frame and does not alter RunViewState. The durable prepare is
already the complete public proof that the Effect is pending. Program v3, Journal frame v2,
EffectId, frame capacity arithmetic, and Store remain unchanged.

### Runtime execution changes

Add one private driver disposition, named Yield or equivalent, with this exact behavior:

1. start_pending_effect removes the pending input, EffectId, and command from the fold slot only for
   the adapter call.
2. On Pending, reconstruct the identical FoldState::EffectPending without re-preparing,
   canonicalizing, deriving another identity, loading, or appending.
3. Return the private Yield disposition.
4. advance_until_stable converts the current accumulator directly to a Runnable view and returns.
5. It must not immediately re-enter the same adapter, otherwise Pending becomes a busy loop.

Settled follows the existing conclusion path. Adapter errors preserve the prepared frame and return
their existing Runtime errors. A later caller resumes from the same qualified prepare.

Update erasure so the typed callback returns EffectAdapterOutcome<C::Evidence> and the erased
callback returns the same sum over the deferred evidence qualifier. Only Settled owns a qualifier;
Pending must not fabricate a value.

### Live EVM mapping

The live transaction adapter returns:

| Observation | Outcome |
| --- | --- |
| Retained Settled authority fact | Settled(retained evidence) |
| Newly validated and durably retained terminal receipt | Settled(evidence) |
| Null receipt followed by successful matching eth_sendRawTransaction response | Pending |
| Authority commit acknowledgement ambiguity | Unavailable |
| Transport failure or dropped submission acknowledgement | Unavailable |
| Malformed response or returned submission hash mismatch | Unavailable |
| Local binding, epoch, signer, purpose, command, or retained-fact mismatch | Internal |

Do not turn a dropped acknowledgement into Pending: without a matching response the caller does not
know whether submission was accepted. Duplicate-safe exact-byte retry remains the recovery rule.

### Runtime tests

Add or update tests proving:

- Pending returns Ok(Runnable), retains exactly the prepare head, and appends no conclusion;
- one start/resume invocation enters a Pending callback at most once;
- a later Settled result closes the same prepare;
- cold read after Pending re-runs only deterministic prepare qualification and performs zero
  adapter IO;
- Unavailable/Internal/panic behavior is unchanged and remains distinguishable from Pending;
- cancellation while the adapter is pending leaves one resumable prepare; and
- concurrent callers returning Settled still converge on one conclusion.

Do not assert internal loop iterations or helper counts except the safety-critical zero adapter call
before an inserted prepare and at-most-one adapter entry for one Pending invocation.

## 2. Simplify Runtime mode erasure

The current private RegisteredState trait requires every mode to implement invalid Read and Effect
methods. It also retains separate optional Read and Effect codec fields and stores driver objects in
FoldState even though the associated executable already owns them.

Replace it with mode-specific private data:

~~~text
RegisteredState {
    signature,
    mode: RegisteredMode::Pure | Read | Effect
}

ExecutableState {
    output_codec,
    failure_codec,
    mode: ExecutableMode::Pure | Read | Effect
}
~~~

Each variant owns only the functions and data it can use:

- RegisteredMode::Pure: monomorphized selected-start function.
- RegisteredMode::Read: selected-start function, retained evidence validator, capability
  signature, and association metadata.
- RegisteredMode::Effect: prepare-start function, pending-start function, retained prepare
  validator, retained evidence validator, capability signature, and association metadata.
- ExecutableMode::Pure: associated selected-start function.
- ExecutableMode::Read: associated selected-start function, exact Read adapter, and
  intent/evidence codecs.
- ExecutableMode::Effect: associated prepare/pending functions, exact Effect adapter, and
  command/evidence codecs.

The private function pointers may still use QualifiedValue internally. They must not create a public
erased-value workflow.

Delete:

- the RegisteredState trait object;
- PureDriver, ReadDriver, and EffectDriver;
- all invalid/no-op validate_retained_read, validate_retained_effect_prepare,
  validate_retained_effect_evidence, and start_pending implementations;
- the read_codecs: Option<_> plus effect_codecs: Option<_> invalid-state pair;
- driver objects duplicated in FoldState.

FoldState retains declaration index and exact qualified values only. Hot execution and cold fold
index into the already-associated ExecutableProgram and exhaustively match ExecutableMode. This is
not a registry lookup: association remains complete and immutable before Store IO.

Preserve separate Read and Effect registries and exact capability/binding association. Do not merge
their typed authority contracts merely to reduce code.

### Runtime hostile Store fixture

Replace SequenceIndeterminateStore, SequenceNotInsertedStore, RecordingStore, and overlapping
one-off wrappers with one test-only ScriptedStore representing the external Store boundary. It
must support:

- a queue of append actions selected by run sequence;
- Insert normally;
- retain candidate then return NotInserted;
- return Indeterminate before retention;
- retain candidate then return Indeterminate;
- record inserted canonical frames; and
- serve a supplied complete retained prefix.

Retained invalid-history fixtures may remain a distinct immutable Store when that makes hostile
bytes visible. The objective is to delete repeated object-safe forwarding boilerplate, not to hide
scenario setup.

Retain all Runtime tests for admission collision, hot/cold equivalence, EffectId/command
qualification, append ambiguity, cancellation, adapter panic, evidence binding, concurrency, and
read-without-adapter-IO.

## 3. Reduce signing to one transient capability port

### Decision: keep the crate, delete most of its current model

Keep crates/signing. Its crate boundary is useful because both a secret-custody implementation and
a live consumer need one implementation-independent signing port. Its current persisted identity
model is not useful and must be deleted.

The current two surfaces contain 1,019 lines including manifests, READMEs, tests, and trybuild
expectations. crates/signing alone is 414 lines: 263 implementation, 118 schema/crypto tests, and 33
manifest/README lines. Repository-wide use tracing shows:

- signer_route is constructed by the in-process keystore and otherwise only compared or asserted;
  it never selects or routes an implementation;
- algorithm accepts exactly one constant and therefore represents no supported choice;
- mfm-evm depends on mfm-signing only to persist PublicSignerIdentity;
- the nonce authority stores a canonical signer reference in addition to the Ethereum sender; and
- live EVM is the only production consumer of the actual signing trait and transient crypto values.

Those facts justify deleting the persisted layers, not deleting the reusable port.

The final ownership is:

~~~text
mfm-signing  = checked transient recoverable-secp256k1 values plus the live signer port
mfm-keystore = bounded thread-affine secret custody and one implementation of that port
mfm-evm      = persisted chain, route, authority epoch, sender, command, and evidence semantics
mfm-evm-live = EVM codecs and orchestration that capture a process-local signer capability
~~~

This is intentionally not the same as making the complete Keystore object a Program capability.
The thread-affine Keystore owns all keys and cannot be Send or Sync. KeystoreOwner is the unique
administrative controller. Each cloneable KeystoreSigner is the least-authority, key-bound,
purpose-bound live capability that an adapter may capture.

Reject these alternatives:

| Alternative | Why it is not the target |
| --- | --- |
| Delete crates/signing and put its trait in crates/keystore | HSM, remote, hardware, or test signers would depend on the in-process custody implementation and its Tokio/zeroization/owner-thread concepts |
| Merge crates/keystore into crates/signing | Every consumer of the small port would compile and conceptually inherit concrete custody machinery; this saves mainly one manifest/README, not the implementation |
| Put signing in mfm-capabilities | ReadCapabilityContract and EffectCapabilityContract own Program-visible persisted command/evidence contracts; digest, key witness, and signature are deliberately transient |
| Put signing in mfm-ids or mfm-values | A foundation crate would acquire one cryptographic algorithm and k256 |
| Put the trait in mfm-evm-live | Keystore and every alternative signer would depend on EVM provider/codec orchestration and Alloy |
| Use an untyped async closure | Identity, purpose, error, and checked signature contracts would be reconstructed at every consumer and fake |
| Add a generic multi-algorithm framework | There is one current recoverable-secp256k1 consumer; associated schemes and erased result families add concepts without reuse |

The separation therefore remains, but crates/signing changes from a persisted metadata/domain crate
into a small transient port. A realistic acceptance target is a 400-700 net LOC reduction across
signing, EVM, authority, PostgreSQL, fixtures, vectors, and documentation. This is an estimate, not
a quota; completion is based on deleted concepts and boundaries below.

### Exact final crates/signing surface

Retain only this semantic API, with ordinary rustdoc and redaction-safe errors:

~~~rust
pub type Result<T> = std::result::Result<T, SigningError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    #[error("signing input is invalid")]
    Invalid,
    #[error("signing operation failed")]
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SigningDigest([u8; 32]);

impl SigningDigest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self;
    pub const fn as_bytes(&self) -> &[u8; 32];
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Secp256k1PublicKey([u8; 65]);

impl Secp256k1PublicKey {
    pub fn new(bytes: [u8; 65]) -> Result<Self>;
    pub const fn as_bytes(&self) -> &[u8; 65];
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CompactRecoverableSignature {
    bytes: [u8; 64],
    recovery_id: u8,
}

impl CompactRecoverableSignature {
    pub fn new(bytes: [u8; 64], recovery_id: u8) -> Result<Self>;
    pub const fn as_bytes(&self) -> &[u8; 64];
    pub const fn recovery_id(&self) -> u8;
}

pub fn recover_public_key(
    digest: SigningDigest,
    signature: CompactRecoverableSignature,
) -> Result<Secp256k1PublicKey>;

pub type SigningFuture = Pin<
    Box<
        dyn Future<Output = Result<CompactRecoverableSignature>>
            + Send
            + 'static,
    >,
>;

pub trait Secp256k1Signer: Send + Sync + 'static {
    fn public_key(&self) -> &Secp256k1PublicKey;
    fn purpose(&self) -> &StableId;
    fn sign(&self, digest: SigningDigest) -> SigningFuture;
}
~~~

The algorithm-specific trait name is deliberate: the port promises recoverable secp256k1 ECDSA,
not arbitrary signing. A future incompatible scheme gets a separate narrow contract only when it
has a consumer. The boxed future is retained because Runtime composition captures Arc<dyn
Secp256k1Signer>; an async trait method would not provide the required object-safe port without
equivalent erasure.

SigningDigest, Secp256k1PublicKey, and CompactRecoverableSignature are checked, exact, transient
values. Digest and signature must not implement Debug, Display, or serde. The public key may have a
safe diagnostic representation only if a concrete current consumer needs it; do not add one for
symmetry. CompactRecoverableSignature continues rejecting invalid scalars, high-S signatures, and
recovery IDs outside 0..=3. EVM separately rejects parity above 1 for its type-2 wire.

Do not make deterministic ECDSA a requirement of the reusable trait. The in-process keystore keeps
its deterministic RFC6979 behavior and known-answer test, but a conforming HSM may produce another
valid low-S signature. This is safe because no raw transaction is submitted before one Prepared
fact wins append-or-compare; after that fact exists, all retries load and submit its exact bytes
without signing again.

Delete from crates/signing:

- PublicSigningKey;
- PublicSignerIdentity;
- verify_recoverable_signature, because its only behavior is recovery followed by equality and no
  production consumer needs a second helper;
- SECP256K1_ECDSA_RECOVERABLE_ALGORITHM_ID;
- IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID;
- signer route and generic algorithm concepts;
- all MfmValue derives, schemas, serde implementations, canonicalization, base64 storage, and
  ContentRef/key_instance_ref code; and
- exact JSON/schema tests for the deleted persisted values.

Rename UncompressedSec1PublicKey to Secp256k1PublicKey and Signer to Secp256k1Signer in one cut.
Do not leave aliases or deprecated names. crates/signing then depends only on workspace k256,
mfm-ids for StableId purpose, and workspace thiserror. Remove mfm-canonical, mfm-program-derive,
mfm-values, serde, and the serde_json dev dependency.

### Purpose-bound keystore implementation

Change import to:

~~~rust
pub async fn import_secp256k1(
    &self,
    secret: SecretSecp256k1Scalar,
    purpose: StableId,
) -> Result<KeystoreSigner, KeystoreError>;
~~~

KeystoreSigner stores exactly:

~~~text
sender: mpsc::Sender<Command>
public_key: Secp256k1PublicKey
purpose: StableId
~~~

It implements Secp256k1Signer. Purpose is immutable on the returned handle, is observable through
purpose(), and is absent from sign(). Remove purpose from Command::Sign, Keystore::sign, owner-loop
matching, test recorders, and the current ignored let _purpose statement. Purpose never crosses the
owner channel because the consumer authorizes the handle before invoking it; forwarding it to code
that does not enforce it is security theater.

The owner map is keyed directly by the checked public key, not by canonical MFM bytes:

~~~rust
BTreeMap<[u8; 65], SigningKey>
~~~

Using the exact byte array internally avoids adding Ord solely to widen the public key API. Import
returns the checked public key to KeystoreOwner, which constructs the purpose-bound handle outside
the owner thread. KeyEntry no longer stores or clones a public identity, and KeystoreSigner no
longer needs Arc around identity metadata.

Duplicate imports retain one SigningKey. The same scalar imported for different purposes yields
handles with the same public key and different immutable purposes without consuming another
capacity slot. Preserve the non-Send/non-Sync Keystore, dedicated bounded owner thread, immediately
awaited blocking join, zeroizing scalar admission, 64-key capacity, deterministic low-S signing,
last-sender exit, and redacted errors.

### EVM persists an account, not a signer implementation

Ethereum account authority is the chain instance plus sender address. The live signer's public key
is a transient witness that the captured capability controls that account. Persisting
in-process-keystore as a route does not route a call; it only prevents recovery with an equivalent
HSM or remote implementation. Persisting a fixed algorithm tag alongside a schema that already
admits only that algorithm adds no choice. Persisting the full public key duplicates the EVM account
binding and leaks provider mechanics into Program values.

Delete EvmWalletIdentity. Change EvmTransactionBinding to:

~~~rust
pub struct EvmTransactionBinding {
    authority_epoch: EvmAuthorityEpoch,
    route: EvmTransactionRoute,
    sender: EvmAddress,
}

impl EvmTransactionBinding {
    pub fn new(
        route: EvmTransactionRoute,
        authority_epoch: EvmAuthorityEpoch,
        sender: EvmAddress,
    ) -> Self;

    pub const fn sender(&self) -> &EvmAddress;
}
~~~

Consequences:

- remove mfm-signing from crates/domains/evm/Cargo.toml;
- delete the public-key, public-signer-identity, and wallet-identity MFM schemas;
- update the transaction-binding schema/semantic IDs and all exact JSON vectors;
- remove the PublicSigningKey exception from reject_known_secret_type in program-derive while
  preserving the generally useful byte-bound derive support used by current EVM values; and
- ensure no key, algorithm, signer route, purpose, digest, or standalone signature enters Program,
  C0, Journal, RunView, public output, or errors.

The signing signature is still physically embedded in the opaque exact raw EVM transaction retained
by the append-only transaction authority. That is required for exact-byte duplicate-safe
resubmission and is the sole durable exception; it must not gain serde, Debug, public output, or
Journal exposure.

### Collapse the nonce authority to the real domain

With signer identity removed, NonceDomainKey plus NonceDomain is a one-field wrapper pair. Replace
both with one NonceDomain containing exactly:

~~~text
authority_epoch: EvmAuthorityEpoch
chain_instance: EvmChainInstance
sender: EvmAddress
~~~

Delete signer_identity_ref and the canonical signer-reference derivation/comparison. In PostgreSQL,
delete signer_schema_id and signer_content_digest from mfm_evm_tx.nonce_domains together with their
constraints, insert/load bindings, parsers, schema qualification, test setup, and assertions.
Reset the fresh current authority baseline directly; do not add a compatibility decoder, data
migration, nullable columns, or unused placeholders.

This does not weaken EVM nonce ownership. The protocol's account and nonce domain is chain plus
address. If two cryptographic public keys collided to one 160-bit Ethereum address, Ethereum itself
would treat them as authority over the same account. A custody-provider route is operational policy,
not a nonce dimension.

### Live adapter binding and proof

The Effect adapter captures Arc<dyn Secp256k1Signer> during registration; no second Runtime registry,
SigningCapabilityContract, signing Effect, persisted signer selector, or generic live-dependency
framework is added. Multiple exact EVM adapter registrations may share the same Arc capability.

Before transaction-authority load, provider IO, or signing, validate_local performs this exact
sequence:

1. compare the complete command binding with the registered binding;
2. compare the binding authority epoch with the captured authority;
3. require signer purpose mfm.evm.sign-eip1559@1;
4. derive the Ethereum address from the signer's checked public key; and
5. require that address to equal the persisted binding sender.

Any mismatch is AdapterError::Internal because trusted local composition is inconsistent. After
signing, recover the public key from the exact digest and compact signature, then require all of:

- recovered public key equals signer.public_key();
- recovered address equals binding.sender();
- the decoded signed transaction equals the exact command plus reserved nonce; and
- the raw transaction hash equals the retained/submitted hash.

Only after these checks may exact raw bytes be retained. Cold recovery may replace an in-process
KeystoreSigner with any same-key, same-purpose Secp256k1Signer implementation. A different key,
wrong purpose, wrong digest/signature, malformed raw transaction, or mismatched sender fails before
new provider submission or authority mutation.

### Signing-focused tests

Keep only boundary-value tests; delete wire tests for types that no longer have a wire.

For mfm-signing, prove:

- the generator public key is accepted and an invalid prefix/off-curve point is rejected;
- the frozen recoverable signature known-answer recovers the exact key;
- a wrong digest does not recover the expected key;
- zero/invalid/high-S signatures and invalid recovery IDs are rejected; and
- digest and signature do not implement serde or diagnostic rendering.

For mfm-keystore, prove:

- Keystore remains !Send and !Sync;
- secret scalar remains non-Debug, non-Display, and non-Serialize;
- zero, out-of-range, and valid scalar admission;
- frozen scalar-one public key, deterministic signature, low-S, recovery ID, and recovery result;
- duplicate same-key import consumes one capacity entry;
- the same key imported for two purposes returns equal public keys and distinct immutable purposes;
- exact bounded channel behavior and 64/65 distinct-key capacity; and
- clean shutdown, final-sender exit, owner panic, post-shutdown failure, and redacted errors.

For live EVM, prove:

- wrong purpose returns Internal with zero authority/provider calls;
- public-key-derived address mismatch returns Internal with zero authority/provider calls;
- a wrong returned signature is rejected before retaining prepared bytes or submitting;
- retained raw decoding recovers the captured public key and persisted sender;
- cold recovery accepts a different implementation with the same key and purpose; and
- cold recovery rejects a different key or purpose before IO.

## 4. Collapse redundant EVM transaction projections

### Current-only value model

Replace the current seven-type settlement/output stack with these five types:

~~~rust
pub enum EvmTransactionConfirmation {
    Created {
        block_anchor: EvmBlockAnchor,
        created_address: EvmAddress,
        transaction_hash: EvmHash,
    },
    Called {
        block_anchor: EvmBlockAnchor,
        transaction_hash: EvmHash,
    },
}

pub struct EvmTransactionRevert {
    block_anchor: EvmBlockAnchor,
    transaction_hash: EvmHash,
}

pub enum EvmTransactionSettlement {
    Confirmed {
        effect_id: EffectId,
        nonce: u64,
        confirmation: EvmTransactionConfirmation,
    },
    Reverted {
        effect_id: EffectId,
        nonce: u64,
        revert: EvmTransactionRevert,
    },
}

pub struct EvmTransactionCompletion<K> {
    caller_context: K,
    confirmed: EvmTransactionConfirmation,
}

pub struct EvmTransactionReversion<K> {
    caller_context: K,
    reverted: EvmTransactionRevert,
}
~~~

EvmTransactionConfirmation uses the existing semantic namespace/name/version
mfm.evm/transaction-confirmation@1 and schema name mfm.evm-transaction-confirmation.
EvmTransactionSettlement, EvmTransactionRevert, EvmTransactionCompletion, and
EvmTransactionReversion retain their current semantic names and version 1. Their schema descriptor
digests change to describe the one current shapes; no compatibility claim exists.

Use strict adjacent kind/value tagging:

~~~text
confirmation created:
{"kind":"created","value":{"block_anchor":...,"created_address":...,"transaction_hash":...}}

confirmation called:
{"kind":"called","value":{"block_anchor":...,"transaction_hash":...}}

settlement confirmed:
{"kind":"confirmed","value":{"confirmation":...,"effect_id":...,"nonce":...}}

settlement reverted:
{"kind":"reverted","value":{"effect_id":...,"nonce":...,"revert":...}}
~~~

Every variant has structured content and denies unknown fields. This removes the manual
unit-variant deserialization workaround.

Delete completely:

- EvmTransactionTerminalResult;
- EvmTransactionConfirmationResult;
- their exports, manual deserializers, schemas, accessors, golden vectors, fixtures, and docs.

Do not leave deprecated aliases.

### Producer and consumer cutover

Update in the same commit:

- EvmTransactionEffect::bind_evidence to require EffectId equality and Create/Call versus
  Created/Called/Reverted shape compatibility;
- ExecuteEvmTransaction::interpret to pass the exact confirmation or revert into its
  context-preserving output/failure;
- settlement accessors needed by authority validation without duplicating variant logic;
- SettledRecord::new validation of EffectId, nonce, and transaction hash;
- PostgreSQL settlement canonicalization/load qualification;
- live receipt-to-confirmation/revert construction;
- retained authority validation;
- fixture lifecycle context and report extraction;
- all exact JSON, semantic ID, SchemaId, content-ref, and rustdoc vectors.

### One PostgreSQL authority baseline cut

The database stores canonical EvmTransactionSettlement bytes, and section 3 removes its two signer
reference columns. Treat the account-binding cut and the outcome-shape cut as one current-only
authority baseline replacement. Do not version or rewrite the baseline twice across adjacent
commits.

- Change the marker to mfm.evm-transaction-postgres.v2.
- Replace evm_transaction_postgres_v1.sql with the one current v2 baseline containing the
  sender-only nonce domain and simplified settlement bytes.
- Delete the v1 marker, migration filename, expected catalog strings, fixtures, and docs.
- Reject v1 as incompatible.
- Require a fresh mfm_evm_tx schema in managed tests.
- Add no decoder, SQL migration, compatibility enum, or fallback.

Program v3 and Journal frame v2 do not change. Programs built with the superseded EVM value
contracts fail exact current association and are not reinterpreted.

## 5. Make PostgreSQL transaction authority optional

### Concrete handle ownership

Keep the mfm-evm-transaction-authority port crate. Split the PostgreSQL implementation into:

~~~rust
pub struct PostgresBackend {
    pool: PgPool,
}

pub struct PostgresEvmTransactionAuthority {
    pool: PgPool,
    authority_epoch: EvmAuthorityEpoch,
}
~~~

PostgresBackend implements only Store, RunIndex, and ConfigRepository.
PostgresEvmTransactionAuthority alone implements EvmTransactionAuthority.

Each handle has its own PgPool and after_connect gate:

- PostgresBackend::connect verifies durability, runtime-role posture, public run history, and
  mfm_config only.
- PostgresEvmTransactionAuthority::connect verifies durability, runtime-role posture, and
  mfm_evm_tx only, captures the exact epoch, and exposes no Store/config/run interface.

Separate pools are deliberate. The two handles verify different owned surfaces, have no cross-port
database transaction, and should not create conditionally admitted pooled connections. Reuse the
same locator grammar and internal SQLx connection helpers; do not add another database service,
locator type, Cargo feature, or crate.

Move the authority test commit-fault state to PostgresEvmTransactionAuthority. Move the authority
trait implementation and every authority operation to that handle. Delete authority_epoch and all
EVM authority behavior from PostgresBackend.

### Provisioning split

Keep:

~~~text
provision_postgres(admin, runtime)
~~~

but make it install and verify only:

- public run-history schema;
- mfm_config schema;
- their exact ownership/ACL;
- the fixed runtime role and durability posture.

Add:

~~~text
provision_evm_transaction_authority(admin, runtime)
~~~

which installs or verifies only:

- mfm_evm_tx schema;
- authority marker v2 and fresh epoch;
- its exact ownership and append-only runtime privileges;
- the same target/role/durability prerequisites.

Each provisioner ignores the other managed surface after checking the shared role and database
posture. Required behavior:

- production CLI provisioning calls only provision_postgres;
- production provisioning does not create mfm_evm_tx;
- PostgresBackend::connect succeeds when mfm_evm_tx is absent or independently incompatible;
- PostgresEvmTransactionAuthority::connect rejects absent, partial, extra, wrong-marker,
  wrong-owner, wrong-ACL, replica, or weak-durability authority installations;
- provisioning one surface never repairs, migrates, deletes, or re-owns the other;
- the managed Effect e2e explicitly provisions and opens both;
- ReservationAcknowledgementFault wraps Arc<PostgresEvmTransactionAuthority>;
- ordinary client e2e and production tests no longer reset or expect mfm_evm_tx.

The Effect task may continue to use the same PostgreSQL service and database. A second pool is a
process-local authority boundary, not another server.

### PostgreSQL tests

Retain exact catalog, constraint, index, ACL, advisory-lock, synchronous-commit, ambiguity,
concurrency, append-only, epoch, raw/hash, settlement, and nonce-fence tests.

Add or update tests proving:

- base provisioning leaves mfm_evm_tx absent;
- base connection is independent of absent or hostile optional authority schema;
- authority provisioning can follow an existing base install;
- authority provisioning alone is fresh-or-exact for its surface;
- authority connection cannot expose Store/config/run ports;
- core backend cannot be coerced to EvmTransactionAuthority;
- old v1 authority marker is rejected;
- the two pools use the same locator target but independent exact gates; and
- no authority database lock spans simulated signer/provider waits.

Do not weaken exact schema verification to reduce LOC. Its independent catalog expectations are a
persistence and privilege boundary.

## 6. Simplify development EVM reconciliation

### Receipt policy

For retained Prepared bytes, use this exact sequence:

~~~text
verify command, retained raw, signer key/purpose, sender-only authority domain, and bound chain instance
-> receipt(expected transaction hash)
-> absent:
     submit exact retained raw at most once
     -> matching returned hash: Pending
     -> transport, malformed response, or mismatch: Unavailable
-> present:
     validate hash, sender, action/status, target or derived CREATE address, and block anchor
     -> canonical_block(receipt block number)
     -> require exact equality with receipt block anchor
     -> retain settlement
     -> Settled(evidence)
~~~

Delete the second receipt observation and second canonical-block observation.

Two immediate observations do not prove confirmation depth, elapsed stability, multi-provider
agreement, or resistance to a later reorg. One validated receipt plus one canonical equality check
proves the exact development policy: the receipt belongs to the provider's canonical block at
observation time. Production finality remains a future capability identity.

### Live adapter test fixture

Replace broad counter-based fakes with a scripted transaction provider and a small in-memory
authority representing their public boundaries. The provider records semantic operations and exact
raw submission bytes. Assert:

- zero authority/provider entry for local preflight mismatch;
- receipt before submission;
- no pending-nonce observation when a reservation already exists;
- at most one submit in an invocation;
- every repeated submit uses byte-identical raw bytes;
- settled fast path performs no signer/provider call; and
- canonical block comparison happens only for a present validated receipt.

Do not assert three receipt calls, two canonical calls, or other helper counts that are not public
safety contracts.

### Missing focused matrix

Add table-driven cases rather than another bespoke fake for each row:

| Owner | Required cases |
| --- | --- |
| Codec | leading-zero integer, nonempty access list, trailing bytes, malformed/noncanonical list, invalid parity, high-S or zero scalar |
| Preflight | wrong binding, epoch, route, chain, genesis, signer purpose, public key, or public-key-derived sender |
| Authority phase | absent, preloaded Reserved, Prepared, Settled, and wrong nested domain |
| Receipt | Create success/revert, Call success/revert, wrong CREATE address, wrong Call target, wrong sender/hash/anchor |
| Submission | matching success becomes Pending; timeout, disconnect, malformed/error response, and mismatched hash remain Unavailable |
| Retry | repeated null receipts submit identical bytes; retained raw mismatch stops before provider |
| Fast path | settled evidence is returned byte-identically with zero signer/provider IO |

Keep the loopback HTTP test that reads the full bounded request then drops the connection. Describe
it as a loopback submission-acknowledgement-drop regression. It is not a proxy that forwards to Reth
and drops Reth's response.

Change the json_rpc.rs module comment from "six frozen-wire RPC calls" to a non-fragile description
of the supported Read/transaction method set, or enumerate all current methods exactly. Prefer the
non-numeric wording so another reviewed method addition cannot stale the comment.

## 7. Simplify the managed Reth Effect proof

### Direct admitted context

Replace EffectFlowContext with:

~~~text
LifecycleContext =
    AwaitingDeployment {
        binding: EvmTransactionBinding
    }
  | AwaitingConfiguration {
        binding: EvmTransactionBinding,
        deployment: EvmTransactionConfirmation
    }
  | BothTransactionsComplete {
        deployment: EvmTransactionConfirmation,
        configuration: EvmTransactionConfirmation
    }
~~~

Change EffectFixtureOperation::Input to:

~~~rust
EvmTransactionContext<LifecycleContext>
~~~

Before Runtime::start:

1. compile the first-party fixture;
2. verify the emitted ABI and fixed selectors;
3. derive the funded sender transiently from signer.public_key();
4. construct the sender-only binding and checked deployment Create command;
5. construct LifecycleContext::AwaitingDeployment with that exact binding; and
6. admit EvmTransactionContext::new(context, deployment_command) directly as C0.

Delete:

- FixtureBytes;
- EffectFixtureInput;
- PrepareDeployment;
- its State identity and registration;
- the first Pure declaration and Journal conclusion;
- configure_calldata and value_calldata fields from persisted lifecycle context.

Do not replace them with another bytes wrapper or setup State.

### Fixture constants and Pure topology

Freeze fixture-local selectors for configure(uint256) and value() as Rust constants. The compiler
helper independently derives selectors from solc ABI/function signatures and requires exact
equality. Pure States construct:

- configure calldata from the fixed configure selector plus the ABI word for 42; and
- anchored value calldata from the fixed value selector.

The independently compiled calldata remains available only to RethDevObserver assertions. It is not
admitted or persisted as workflow metadata.

The exact graph becomes:

~~~text
C0 deployment EvmTransactionContext
-> deployment ExecuteEvmTransaction Effect
-> PrepareConfiguration Pure
-> configuration ExecuteEvmTransaction Effect
-> PrepareObservation Pure
-> ReadAnchoredContractCall Read
-> FinalizeReport Pure
~~~

PrepareConfiguration requires AwaitingDeployment plus a Created confirmation, builds the exact Call
command, and returns AwaitingConfiguration carrying the deployment confirmation.
PrepareObservation requires AwaitingConfiguration plus a Called confirmation, builds the anchored
value intent at the configuration confirmation anchor, and returns BothTransactionsComplete as
caller context.

Invalid lifecycle variants or impossible Create/Call confirmation shapes must return one closed
fixture failure, not panic or unreachable. Keep the transaction-revert and anchored-Read failure
mappers needed to converge their domain failures on EffectFixtureFailure.

PrepareConfiguration, PrepareObservation, and FinalizeReport use EffectFixtureFailure directly as
their failure contract. Their success edges continue through the graph; their missing failure
successors terminate at the Operation's exact root failure. Do not add another mapper layer for
fixture-local deterministic validation.

### Evidence-derived report

FinalizeReport must:

1. require BothTransactionsComplete;
2. require Created deployment and Called configuration confirmations;
3. decode the exact 32-byte ABI return word;
4. reject a malformed length or a value outside the fixture's supported u64 projection through a
   typed EffectFixtureFailure;
5. construct EvmU256 from the decoded value; and
6. build the report from the decoded value, confirmation hashes/address, and returned anchor.

It must not assert that returned bytes equal 42 and then write EvmU256::from_u64(42). The final
external assertion that the report equals configured value 42 remains valuable because it is
independent of report construction.

Use closed redaction-safe fixture failures such as TransactionReverted,
AnchoredObservationFailed, InvalidLifecycle, and InvalidReturnData. Do not put raw return bytes,
provider messages, or secrets in failures.

### Real bounded caller progression

Drive at most eight total start/resume invocations:

~~~text
for attempt in 0..8:
    rebuild core backend, authority handle/decorator, provider, assembly, and Runtime
    attempt 0: start with Program and C0
    later: resume with the same RunId

    injected first reservation acknowledgement loss:
        require Unavailable and one retained Reserved fact

    Ok Runnable:
        inspect public Journal/authority state
        if a newly Prepared transaction exists, independently await its Reth receipt
        drop all reconstructed handles and continue

    Ok terminal:
        retain terminal view and stop

    any other error:
        fail

after loop:
    require terminal success
~~~

The consumed-once reservation fault flag remains outside reconstructed handles so it cannot re-arm.
Do not add a scheduler, retry sleep, or test-only Runtime loop. Remove the manually incremented
invocations <= 8 assertion.

### Journal and independent assertions

Remove the substring check for "raw". A base64 initcode string may contain arbitrary text. Exact
strict decoding of each prepared Journal command as Eip1559TransactionCommand and comparison with
the independently authored command proves there is no raw-transaction field.

Retain all of these independent assertions:

- one generated/imported/funded wallet;
- exactly two outgoing wallet transactions;
- nonces exactly 0 and 1, with no third transaction;
- authority raw Keccak equals authority hash equals Reth transaction hash;
- decoded Reth type-2 fields equal the authored command, nonce, and sender;
- deployment creates the expected address and exact deployed code;
- configuration targets that address with exact calldata/value;
- exactly one expected Configured event/value;
- anchored value() returns the configured value at the configuration receipt block hash;
- final report contains both transaction hashes, address, anchor, and evidence-derived value;
- Journal has exactly two Effect prepares and two adjacent conclusions;
- public authority load returns two Settled facts whose one domain is exactly authority epoch,
  chain instance, and sender, with no signer content reference;
- cold read returns byte-identical report bytes;
- extra terminal resume/read leaves the head unchanged; and
- final pending nonce is 2.

Keep RethDevObserver and the bounded solc compiler helper. They are independent external boundaries,
not redundant mirrors of Runtime or the live adapter.

## Documentation cut

Documentation changes ship with the commit that changes their contract:

| Document | Required correction |
| --- | --- |
| docs/design.md | Pending outcome, transient purpose-bound signer, sender-only EVM binding/nonce domain, simplified settlement values, optional authority, one receipt/canonical policy |
| docs/architecture.md | Mode-specific Runtime erasure, thin signing port versus concrete custody, sender-only domain ownership, separate PostgreSQL handles, one-Effect adapter-internal nonce ownership |
| docs/run-execution.md | Effect prepare/conclusion, Pending/Runnable, ambiguity/cancellation, cold Effect re-prepare, no adapter IO in read |
| docs/persisted-public-surfaces.md | Delete persisted signing schemas; document sender-only EVM binding/domain, current outcomes, authority marker v2, and core versus optional PostgreSQL surfaces |
| docs/evm-rpc-routing.md | Pending submission result and one receipt/canonical sequence |
| docs/build-and-verification.md | Core provisioning versus Effect e2e authority provisioning |
| docs/known-gaps.md | Production finality, persistent signer recovery, and authority rollback remain unsupported |
| docs/evm-portfolio-contract-freeze.md | Lower-level transaction support remains outside Portfolio and production composition |
| affected crate READMEs/rustdoc | Runtime, signing, keystore, EVM, authority, live EVM, PostgreSQL, and app current APIs |
| IMPL_PLAN_EFFECT_SUP_E2E.md | Mark original six-commit implementation complete and this corrective plan as the current follow-up |

In docs/run-execution.md, replace "read never executes State or adapter code" with the precise
contract: read performs no adapter IO and appends nothing, but cold qualification of a retained
Effect prepare deterministically invokes EffectState::prepare to compare exact command bytes and
EffectId.

Do not create another normative Effect protocol document. docs/design.md remains authoritative.
Avoid numeric RPC method counts in prose when the exact trait/method list already owns that
contract.

## Test retention and reduction policy

### Must retain

- Program v3 and Journal v2 hostile-input and exact-wire vectors;
- prepare-before-adapter, ambiguity, cancellation, concurrency, hot/cold, and evidence-binding
  Runtime tests;
- transient signing key/signature validation, deterministic known-answer, low-S, recovery,
  purpose-bound handle, secret-surface, and !Send/!Sync tests;
- authority append-only, advisory-lock, synchronous-commit, ambiguous-commit, epoch, and
  concurrency tests;
- one independent real-Reth deploy/configure/read/report scenario;
- loopback provider bounds and dropped-acknowledgement regression; and
- exact schema/ACL gate tests for each PostgreSQL surface.

### Must delete or consolidate

- invalid/no-op Runtime mode methods;
- duplicated test Store forwarding implementations;
- signer purpose forwarding recorders;
- canonical JSON, SchemaId, semantic-ID, and content-ref tests for deleted signing values;
- authority tests whose only subject is the deleted signer reference or SQL columns;
- EvmTransactionTerminalResult and EvmTransactionConfirmationResult vectors;
- exact positive receipt/canonical helper call counts;
- FixtureBytes, EffectFixtureInput, and PrepareDeployment;
- fixed manual invocation counting;
- substring-based Journal raw checks; and
- duplicated documentation claims.

Every test must fail on an observable public, persistence, cryptographic, adapter, or external-chain
regression. Do not add assertions for facts already made unrepresentable by the new enums, private
fields, or exact deserializers.

## Complete deletion ledger

The corrective series is incomplete while any superseded symbol/path remains in current code,
tests, normative docs, fixtures, or build tasks:

~~~text
FixtureBytes
EffectFixtureInput
PrepareDeployment
EffectFlowContext
EvmTransactionTerminalResult
EvmTransactionConfirmationResult
PublicSigningKey
PublicSignerIdentity
EvmWalletIdentity
NonceDomainKey
UncompressedSec1PublicKey
trait Signer
signer_identity
signer_identity_ref
signer_route
signer_schema_id
signer_content_digest
SECP256K1_ECDSA_RECOVERABLE_ALGORITHM_ID
IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID
verify_recoverable_signature
let _purpose
Signer::sign(digest, purpose)
impl EvmTransactionAuthority for PostgresBackend
PostgresBackend.authority_epoch
mfm.evm-transaction-postgres.v1
evm_transaction_postgres_v1.sql
RegisteredState trait object
PureDriver / ReadDriver / EffectDriver
read_codecs: Option<_> plus effect_codecs: Option<_>
SequenceIndeterminateStore
SequenceNotInsertedStore
positive assertions for three receipt calls or two canonical calls
manual invocations <= 8 assertion
String::contains("raw") Journal assertion
documentation claiming successful submit returns Unavailable
documentation claiming Runtime::read executes no State code
documentation claiming the provider owns six RPC methods
~~~

Use repository-wide rg searches for the concrete Rust identifiers, v1 marker, obsolete API calls,
and stale prose before final verification. Exclude this corrective document and the explicitly
historical body of IMPL_PLAN_EFFECT_SUP_E2E.md from absence searches; the original plan's status
must clearly mark those contracts as superseded. Do not satisfy this ledger with renamed aliases or
hidden compatibility modules.

## Ordered logical commits

Every commit must compile with all current consumers, include its focused tests and documentation,
and leave one coherent current design. Subjects are lower case.

### 1. "return pending effects as runnable"

- add EffectAdapterOutcome;
- change typed and erased Effect callback contracts;
- add the private Runtime yield behavior;
- update all Effect adapters and Runtime tests;
- make successful matching EVM submission return Pending;
- update Runtime/live/app consumers enough to compile; and
- update docs/design.md, docs/run-execution.md, Runtime/live READMEs, and rustdoc.

Do not include Runtime erasure cleanup here unless inseparable from the outcome cut.

### 2. "simplify runtime state erasure"

- replace RegisteredState drivers with mode-specific registered/executable enums;
- remove invalid optional mode combinations and duplicated FoldState drivers;
- consolidate scripted hostile Store fixtures;
- preserve the entire semantic Runtime test boundary; and
- update docs/architecture.md and Runtime internals documentation.

### 3. "bind signing capabilities to one purpose"

- add purpose to keystore import and the current signer handle contract;
- make KeystoreSigner the purpose-bound live capability and keep Keystore as its provider;
- store immutable purpose on each KeystoreSigner;
- remove per-call purpose forwarding;
- enforce the EVM purpose before authority/provider IO;
- update every implementation, fake, known-answer, compile-fail consumer, and app fixture; and
- update signing, keystore, live EVM, design, and architecture docs.

This commit changes only process-local authority and intentionally leaves current persisted shapes
unchanged until the single current-contract cut in commit 4.

### 4. "bind evm transactions by account"

- reduce crates/signing to the exact transient API and rename the trait Secp256k1Signer;
- delete PublicSigningKey, PublicSignerIdentity, route/algorithm IDs, their schemas, dependencies,
  vectors, and the program-derive name exception;
- delete EvmWalletIdentity and persist sender directly in EvmTransactionBinding;
- remove the mfm-evm to mfm-signing dependency;
- collapse NonceDomainKey and NonceDomain into the sender-only domain;
- delete signer identity references and both signer columns from the authority and PostgreSQL;
- replace the two result enums with direct confirmation/revert settlement variants;
- update all domain/live/authority/app producers and consumers;
- replace all exact EVM schema and wire vectors;
- replace the authority marker/migration once with the complete PostgreSQL v2 target;
- reject and delete v1 with no migration or decoder; and
- update signing, keystore, persisted-surface, EVM, authority, PostgreSQL, design, and architecture
  docs.

This commit is intentionally broad and inseparable: signing schemas, EVM binding, nonce domain,
SQL columns, settlement bytes, and the fresh authority marker are one current persisted-contract
cut. Splitting it would either leave an incoherent intermediate schema or version the same unshipped
baseline twice.

### 5. "decouple postgres evm transaction authority"

- create PostgresEvmTransactionAuthority with its own pool and exact gate;
- remove authority behavior from PostgresBackend;
- split base and authority provisioning;
- update CLI, app e2e, PostgreSQL tests, managed fixture resets, and docs;
- prove base provisioning/connection does not require or create mfm_evm_tx; and
- retain the same locator grammar and PostgreSQL service.

### 6. "simplify development evm reconciliation"

- use one receipt plus one canonical block check;
- delete duplicate immediate observations;
- replace implementation-count assertions with semantic sequence/raw-byte assertions;
- add the missing table-driven codec, phase, mismatch, success/revert, retry, and Pending cases;
- retain and correctly name the loopback drop regression; and
- update live EVM and RPC routing docs.

### 7. "simplify effect recovery against reth"

- admit the deployment EvmTransactionContext directly;
- replace EffectFlowContext with LifecycleContext;
- delete fixture bytes/input/setup State;
- construct calldata from checked constants;
- derive report value from return evidence with typed failure;
- drive a real bounded reconstruction/resume loop;
- remove brittle Journal assertions;
- preserve independent Reth/authority/history proofs;
- update Nixfied tasks only where provisioning/reset behavior changes;
- update app/build/known-gap docs and both implementation-plan statuses.

If the branch history may be rewritten before merge, split the existing unrelated CLI component
line-count correction into its own lower-case logical commit. If history rewrite is not authorized,
do not create a revert/re-add pair; report the existing exception and keep all new corrective
commits scoped.

## Verification workflow

All Cargo/Rust commands run inside the default Nix development shell. Use the narrowest focused
command during each commit and expand when a public or persistence boundary changes.

### Commit 1

~~~sh
nix develop -c cargo test -p mfm-runtime --all-targets
nix develop -c cargo test -p mfm-evm-live --all-targets
nix develop -c cargo check -p mfm-app --all-targets
~~~

### Commit 2

~~~sh
nix develop -c cargo test -p mfm-runtime --all-targets
~~~

### Commit 3

~~~sh
nix develop -c cargo test -p mfm-signing -p mfm-keystore --all-targets
nix develop -c cargo test -p mfm-evm-live --all-targets
nix develop -c cargo check -p mfm-app --all-targets
~~~

### Commit 4

~~~sh
nix develop -c cargo test \
  -p mfm-signing -p mfm-keystore -p mfm-program-derive --all-targets
nix develop -c cargo test \
  -p mfm-evm -p mfm-evm-transaction-authority -p mfm-evm-live --all-targets
nix develop -c cargo test -p mfm-storage-postgres --lib
nix develop -c cargo check -p mfm-app --all-targets
~~~

Run the managed PostgreSQL task when the v2 authority baseline and rejection cases require a real
server:

~~~sh
nix run .#run -- --task postgres-test
~~~

### Commit 5

~~~sh
nix develop -c cargo test -p mfm-storage-postgres --lib
nix develop -c cargo check -p mfm -p mfm-app -p mfm-rest-api --all-targets
nix run .#run -- --task postgres-test
~~~

If nixfied.nix changes, run the model admission early:

~~~sh
nix run .#model-check
~~~

### Commit 6

~~~sh
nix develop -c cargo test -p mfm-evm-live --all-targets
~~~

### Commit 7

~~~sh
nix develop -c cargo test -p mfm-app --test evm_contract_effect_e2e --no-run
nix run .#run -- --task effect-e2e
~~~

Use focused formatting and strict Clippy for the touched packages while iterating. Because this
series changes cross-crate APIs, persistence contracts, concurrency behavior, and Nixfied fixture
composition, run one final composed gate on the exact candidate:

~~~sh
nix run .#ci
~~~

Do not run .#check, .#test, and .#test-db immediately before .#ci; CI composes the broad gates.
Run nix flake check --no-build only if the flake/output surface changes beyond nixfied.nix.

## Completion checklist

The corrective implementation is complete only when:

- all seven commits are ordered, coherent, and lower-case;
- the deletion ledger has no remaining current-code or stale-doc matches;
- successful matching submission returns Ok Runnable, not RuntimeError::Unavailable;
- true dropped acknowledgement and transport failure remain Unavailable;
- Pending appends no conclusion and causes no internal retry loop;
- Runtime mode erasure has no invalid method surface or double optional codecs;
- Secp256k1Signer is the thin transient live port, Keystore is one provider, purpose is immutable on
  the handle, and EVM rejects a wrong purpose/key-derived sender before mutation IO;
- crates/signing exposes no MfmValue, serde, canonicalization, ContentRef, route, algorithm tag, or
  persisted identity and has only k256, mfm-ids, and thiserror as normal dependencies;
- EvmTransactionBinding persists sender directly, EvmWalletIdentity is gone, and mfm-evm has no
  dependency on mfm-signing;
- NonceDomain directly contains epoch, chain instance, and sender, with no wrapper key, signer
  content ref, or signer SQL columns;
- EVM settlement and workflow output use only the five target types;
- authority PostgreSQL v2 is the sole current baseline;
- production core provisioning creates no mfm_evm_tx schema;
- core PostgresBackend cannot act as EvmTransactionAuthority;
- optional authority connection has its own exact pool/gate and captured epoch;
- receipt settlement uses one validated receipt plus one canonical block equality check;
- repeated submissions use byte-identical retained raw bytes;
- the fixture Program begins at deployment Effect with no PrepareDeployment frame;
- calldata does not persist in LifecycleContext;
- the report value is decoded from anchored Read evidence;
- the e2e uses a real maximum-eight-invocation loop;
- independent Reth proves exactly two transactions with nonces 0/1 and no third mutation;
- all high-value durability, ambiguity, cancellation, concurrency, crypto, persistence, and
  external-boundary tests remain;
- authoritative design, architecture, execution, persistence, routing, build, known-gap, README,
  rustdoc, and plan status text match the current design;
- no secret or raw signed transaction appears in Program, C0, Journal, RunView, output, logs, or
  failures;
- git diff --check passes;
- focused verification is reported per commit; and
- one final nix run .#ci passes on the exact candidate.

Do not use a raw LOC quota as acceptance. The engineer's final report must include:

- concepts, public types, fixture types, and duplicate test harnesses deleted;
- any new public type and why it removes more invalid states or branches than it adds;
- final origin/dev diff stat compared with the 90-file, +14,140/-920 baseline;
- exact focused and final verification commands/results; and
- remaining unverified risks or blockers.

## Material uncertainties

### Candidate has no supported external consumers

- Assumption: the six commits are still an unmerged clean-slate candidate with no supported
  retained Program, authority schema, or signer API consumers.
- Why uncertain: repository inspection cannot prove external deployment or data retention.
- Consequence if wrong: current-only EVM schema replacement and authority v2 invalidate external
  retained data.
- Validation: confirm the branch has not shipped and reset every managed development baseline. If
  it has shipped, stop; repository policy does not permit inventing a compatibility migration in
  this corrective series.

### One immutable purpose per signer handle

- Assumption: one purpose per signer handle is the intended custody authority model.
- Why uncertain: EVM is the only current Secp256k1Signer consumer, while an unseen future consumer
  might request a multi-purpose handle.
- Consequence if wrong: import and handle representation would need one explicit immutable checked
  purpose set rather than the single StableId.
- Validation: confirm current composition needs only mfm.evm.sign-eip1559@1. Do not add a purpose
  set without a concrete current consumer.

### Exact raw transaction is the only durable signature container

- Assumption: "signatures are transient" prohibits standalone digest/signature signing values in
  Program, Journal, metadata, output, and errors, while still allowing the authority to retain the
  opaque exact signed EVM transaction.
- Why uncertain: yParity, r, and s are necessarily encoded inside that raw transaction even though
  no standalone signature object is persisted.
- Consequence if wrong: a literal ban on retaining any signature bytes makes durable exact-byte
  resubmission impossible and invalidates the Effect recovery design.
- Validation: confirm exact raw transaction custody is the intended sole exception before commit 4;
  it remains non-serde, non-Debug, non-Journal, bounded, and accessible only through the authority
  port.

### Future composition selects an account, not a custody route

- Assumption: an EVM transaction semantically authorizes a chain account; an equivalent same-key,
  same-purpose HSM, remote signer, or in-process keystore may resume it.
- Why uncertain: no production EVM transaction composition exists, so a future operational policy
  might require choosing a specific custody provider.
- Consequence if wrong: a future Effect binding may require a secret-free capability selector in
  addition to sender.
- Validation: do not retain the speculative current in-process route. If a concrete production
  policy needs provider selection, introduce a separately reviewed binding-level selector and
  capability identity then; do not overload account or nonce identity now.

### Separate authority pool fits the connection budget

- Assumption: one additional PostgreSQL pool in development Effect composition is operationally
  acceptable.
- Why uncertain: external database connection budgets are not recorded in the repository.
- Consequence if wrong: the design would need an explicitly surface-gated shared-pool abstraction,
  which adds conditional gate complexity.
- Validation: confirm managed and intended development deployment connection limits before commit
  5. There is no cross-port transaction or atomicity requirement that otherwise requires one pool.

### Settlement remains development-only

- Assumption: EvmTransactionEffect version 1 remains limited to pinned non-reorging managed Reth.
- Why uncertain: the lower-level adapter is reusable and could be mistaken for production support.
- Consequence if wrong: both the new one-observation policy and the existing duplicate immediate
  observations are insufficient for finality or reorg safety.
- Validation: retain production composition tests proving the capability is absent and preserve the
  explicit known-gap statement. A production requirement stops this series and requires a new
  capability identity and policy design.

### Adapter-internal nonce authority supersedes the original wording

- Assumption: the owner accepts that adapter-internal append-only authority supersedes "nonce
  reservation via State injection."
- Why uncertain: the original request used the latter phrase, while the implemented and reviewed
  design intentionally chose one Effect.
- Consequence if wrong: the requested system is a different two-Effect Program architecture, not a
  corrective simplification of this branch.
- Validation: treat approval of this handoff plan as the explicit decision. If the owner objects,
  stop before implementation rather than smuggling IO or an Effect through InjectionWriter.
