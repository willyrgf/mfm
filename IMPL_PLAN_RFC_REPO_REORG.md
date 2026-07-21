# Implementation Plan: RFC Repository Reorganization

- Status: Accepted with the binding refinements below
- RFC: `RFC_REPO_REORG.md`
- Date: 2026-07-21
- Target: one buildable, reviewable commit per numbered step

## Decision

Accept the RFC's core solution. The current package topology turns internal roles into public Cargo
boundaries, duplicates ordinary execution and replay, and registers product-inaccessible behavior.
The proposed pure-domain/live-domain structure removes those costs while preserving the Cargo
firebreaks that matter: kernel, pure domain, live IO, generic signing, secret-bearing keystore,
storage implementation, app assembly, and binaries.

The projected 27 packages are an expected result, not an architecture invariant. Do not weaken a
real boundary to hit that number, and do not preserve a package merely to keep an inventory stable.

The RFC is accepted with these binding refinements:

1. `mfm-portfolio-live` does not own Postgres, in-memory, or retained-artifact provider facades.
   Provider implementations stay with the stores that own the read semantics.
2. Managed fact publication becomes a synchronous, runtime-owned `NoCaps` effect. Delete
   `FactRecordCapability`, the broad async managed-write trait, adapter fact-record identities, and
   redundant capability/adapter fact provenance.
3. Bitcoin aggregate collection uses one multi-descriptor `scantxoutset`, not one scan per address.
   This is the only design that gives the batch one actual UTXO snapshot while also reducing IO.
4. Production reachability covers certification, fact descriptors, runners, capabilities, adapter
   executables, side-effect verify bindings, and replay callbacks. Checking only state and operation
   registries is insufficient.
5. Runtime document parsing and value-source resolution have one private owner in `mfm-app`.
   Domain live crates own endpoint/session semantics and receive already selected, resolved inputs.
6. The Postgres store authority is deliberately cut when the fact event/projection shape changes.
   Old databases are rejected; there is no migration, fallback decoder, or compatibility reader.
7. The RFC's commit order is replaced by the sequence in this document. In particular, it must not
   add a managed runner against `mfm-fact-capabilities` and then immediately rewrite that runner.
8. Bitcoin address authority moves to `mfm-bitcoin` and uses the `bitcoin` (`rust-bitcoin`) crate for network checking,
   canonical address rendering, and script derivation. The hand-written `bech32`/`bs58` validation
   path is deleted.
9. Final production binaries have one internal dependency boundary: `mfm-app`. They do not depend
   directly on domain, kernel, signing, live, storage, or keystore packages.

These refinements narrow and complete the RFC; they do not introduce a competing architecture.

## Aspect-by-aspect review

| RFC aspect | Verdict | Implementation consequence |
|---|---|---|
| Decision principles | Agree | Prefer deletion and one final path. No compatibility crates, aliases, dual registration, or old decoders. |
| Problem statement | Agree | The current 44-package topology, capability ownership, runner duplication, broad production assembly, and inventory tests were confirmed in the repository. |
| Goals and non-goals | Agree | Preserve pure/live, signing/keystore, store/Postgres, and proc-macro firebreaks. Do not create a monolith. |
| Approximately 27 packages | Agree as an outcome | Never assert the number or an exact package list. |
| One pure crate per domain | Agree | Consolidate Bitcoin, EVM, and portfolio model/capability/state/operation code; make package-crossing-only types private or delete them. |
| One live crate per domain | Agree | Keep each transport as a private module of its domain live crate. Export narrow binding/registration APIs, not a second raw-transport facade. |
| `mfm-effects` disposition | Agree | Move the effect types and tests into `mfm-capabilities`, update imports, and delete the package. |
| `mfm-fact-capabilities` disposition | Agree with tightening | Move the real fact-index read port into `mfm-facts`; delete the fake fact-record authority instead of moving it. |
| `mfm-artifact-capabilities` disposition | Agree with exact ownership | `mfm-store` owns retained-artifact authority and the fact-response requirement constructor. Domains decode their own canonical response bytes. Delete the generic hydration facade. |
| Domain package dispositions | Agree | Merge without shell crates or old root re-exports. Use `bitcoin` for MFM-owned names; reserve `BTC` for denomination terminology. |
| `mfm-runtime-config` disposition | Agree, but RFC plan was incomplete | Add an explicit deletion commit. Keep one private app parser/resolver and selective entry parsing. |
| `mfm_core` and keystore provider | Agree | Merge into `mfm-keystore`; preserve the security contract and current persisted keystore format unless a separate security reason requires a format change. |
| Proof workflow deletion | Agree | Delete all three packages, production wiring, replay branches, docs, and tests. Replace only genuinely unique kernel coverage with minimal test-local fixtures. |
| Generic pure runner/replay | Agree | Add one kernel path and migrate every surviving ordinary pure state in the introducing commit. |
| Generic managed fact runner/replay | Agree with a narrower contract | Replace `ManagedWriteState`; the state deterministically returns output plus one non-empty homogeneous fact batch. Runtime stages the batch atomically. |
| Bitcoin two-state graph | Agree with a stronger executor | One external-read node performs one multi-address scan and canonicality check; one managed node writes the entire fact batch and receipt. |
| EVM foundations | Agree | Keep exact-anchor validation and transaction submission as reusable library surfaces with explicit local test registries, not production registrations. |
| Production registration | Agree with expanded scope | Production assembly equals the union of published portfolio graph variants across every executable/verification registry surface. |
| Dependency contract | Agree | Replace path/name/count snapshots with semantic layer and edge properties plus synthetic negative tests. |
| Preserved invariants | Agree | Add boundary tests for batch atomicity, replay order, shared fact-query snapshots, canonical bytes, and redaction. |
| No persisted compatibility | Agree, but deletion alone is insufficient | Change the Postgres store authority version and baseline schema. Old stores fail authority validation and must be recreated. |
| Original 15-commit plan | Replace | It omitted runtime-config deletion and the store authority cut, and ordered fact/runner work so new code would be rewritten. Use the 18 commits below. |
| Acceptance criteria | Agree as amended | Add the stronger registry equality, one-scan Bitcoin, provider ownership, and store-epoch criteria in this plan. |
| Rejected alternatives | Agree | Do not keep role packages, create a global live crate, retain compatibility shells, or preserve adapter runner copies. |

## Target architecture

```text
crates/
|-- kernel/
|   |-- ids
|   |-- canonical
|   |-- values
|   |-- capabilities
|   |-- facts
|   |-- program
|   |-- program-derive
|   |-- spec
|   |-- certify
|   |-- events
|   |-- store
|   |-- manual-auth
|   |-- runtime
|   `-- replay
|-- domains/
|   |-- bitcoin
|   |-- evm
|   `-- portfolio
|-- live/
|   |-- bitcoin
|   |-- evm
|   `-- portfolio
|-- signing
|-- keystore
|-- storages/
|   `-- postgres
`-- app

bin/
|-- cli
`-- rest-api

tests/
`-- integration
```

Final package names are:

- pure domains: `mfm-bitcoin`, `mfm-evm`, `mfm-portfolio`;
- live integrations: `mfm-bitcoin-live`, `mfm-evm-live`, `mfm-portfolio-live`;
- generic signer contract: `mfm-signing`;
- secret-bearing implementation: `mfm-keystore`;
- storage implementation: `mfm-storage-postgres`;
- app, binaries, integration package, and the 14 kernel packages shown above.

The dependency direction is:

```text
kernel
  ^       ^
  |       `-- mfm-signing
  |
source domains (bitcoin, evm)
  ^                    ^
  |                    |
portfolio domain       same-domain live integration
  ^                    ^
  |                    |
  `-------- mfm-app ---+--- mfm-keystore
               |
               +---------- mfm-storage-postgres
               |
             binaries
```

Additional rules:

- `mfm-bitcoin` and `mfm-evm` never depend on `mfm-portfolio`.
- `mfm-portfolio` may depend on the two source domains and never on live crates.
- A source live crate depends on its corresponding pure domain and kernel/platform contracts. The
  aggregate portfolio live crate may additionally depend on the source pure domains whose fact
  responses it decodes. No live crate depends on another live domain, Postgres, the keystore
  implementation, app, or a binary.
- `mfm-keystore` may depend on `mfm-signing`; `mfm-signing` never depends on `mfm-keystore`.
- `mfm-store` never depends on `mfm-storage-postgres`.
- `mfm-portfolio-live` consumes `FactIndexReadProvider` and `RetainedArtifactReadProvider` trait
  objects. It does not implement either storage authority.
- Transport, capability, state, and operation remain code roles, normally represented by private
  modules. They are not automatic package boundaries.

## Cross-cutting implementation contracts

### Deletion and naming policy

- Every replacement commit removes the superseded implementation, package, tests, documentation,
  imports, feature flags, manifest entries, and resulting `Cargo.lock` entries in that same commit.
- Do not add deprecated aliases, old package shells, compatibility modules, feature-gated legacy
  decoders, old registry branches, or temporary public session abstractions.
- A type is public only when another final package must name it or it is an intentional public
  library contract. Moving two modules into one crate is a reason to reduce visibility.
- New MFM-owned Bitcoin types use `Bitcoin...`. Keep `BTC` only for units and protocol terms such as
  `amount_btc` or satoshis-per-BTC.
- Stable schema/descriptor identities that change shape receive a new identity. Do not silently
  reuse an old schema identity with different fields.

### Generic pure execution and replay

`mfm-runtime` owns the one ordinary pure-state runner. Its erased execution sequence is:

1. Load and schema-check the certified config.
2. Construct `S` from `ValidatedConfig<S::Config>`.
3. Load `S::Input` with `load_materialized_input`, including arbitrary certified input trees.
4. Load the certified context.
5. Invoke `PureState::run`.
6. Stage exactly one canonical state output with `RunnerOutputBuilder`.
7. Return the runtime-owned settlement.

Preserve optional `ContextOutputExtractor` support. Use one runtime-owned factory/executable
identity; pure execution is no longer attributed to a domain adapter. Expose the smallest typed
registration helper needed by live/app assembly and keep the runner implementation non-public if
external construction is unnecessary.

`mfm-replay` owns `verify_pure_state<S>`. It matches the exact state descriptor identity, rebuilds
config/input/context from retained artifacts, invokes the same state implementation, and compares
canonical output bytes. It never resolves runtime configuration or a live provider.

### Generic managed fact execution and replay

Replace, rather than supplement, the current broad trait with this semantic contract:

```rust
pub trait ManagedFactWriteState:
    StateSpec<Effect = ManagedFactWrite, Caps = NoCaps>
{
    type Fact: MfmFactType;

    fn write(
        &self,
        input: Self::Input,
        context: &CertifiedContext<Self::Context>,
    ) -> StateResult<(Self::Output, NonEmpty<Self::Fact>)>;

    fn fact_visibility() -> FactVisibility;
}
```

Use the names above and delete the old names rather than aliasing them. The semantics may not broaden:

- execution is synchronous and deterministic;
- there are no state-visible capabilities and no ambient IO;
- one state emits one homogeneous, non-empty fact type;
- the state returns facts in canonical order and the runtime does not sort them;
- visibility is state-owned and participates in the managed-write effect contract digest;
- the emitted fact descriptor is derived from `S::Fact`, not supplied through a second manual hook;
- managed fact states have no adapter binding.

Rename the effect class and runner kind from the unused broad `ManagedPlatformWrite` concept to
`ManagedFactWrite`. Delete the old effect/runner variants rather than retaining aliases; no
surviving state performs another kind of managed platform write.

Delete `StateSpec::emitted_fact_descriptors`. Add one sealed
`EffectRunner<S>::fact_descriptors() -> Result<Vec<FactDescriptor>>` source: the managed-fact
implementation returns `S::Fact::descriptor()`, and pure, external-read, and side-effect
implementations return an empty vector. Derive `StateDescriptorIdentity.emitted_fact_descriptors`
from those full descriptors, and make `CertificationRegistry::register_state<S>` install the same
full descriptor artifacts automatically. This is implementable without specialization and gives
descriptor refs and retained artifacts one source of truth. Delete the public manual
`register_fact_type`/`register_fact_descriptor` paths (retaining only private insertion used by
automatic state registration) and every macro/domain `after_registration` fact hook; do not leave a
second registration path.

The managed effect-contract digest is canonical JSON with exactly this semantic payload:

```text
run-private: {"abi":"mfm.managed-fact-write.v1","visibility":{"kind":"run_private"}}
control:     {"abi":"mfm.managed-fact-write.v1","visibility":{"audience":"control","kind":"indexed","scope":"default"}}
platform:    {"abi":"mfm.managed-fact-write.v1","visibility":{"audience":"platform","kind":"indexed","scope":"default"}}
```

Use the same canonical visibility representation as `FactClaim`. The ABI definition requires one
homogeneous, non-empty batch. Do not repeat the fact descriptor hash in this digest: the enclosing
content-addressed state descriptor already binds the one derived emitted-fact descriptor alongside
the digest.

Allow `NoCaps` for `ManagedFactWrite`. Delete `ManagedPlatformWriteRole`, its public capability-role
variant, and the old broad effect/runner variants; the current repository has no surviving
non-fact consumer. Delete `FactRecordCapability` unconditionally.

`ManagedFactWriteRunner<S>` follows the pure runner's config/input/context loading, calls `write`,
records every fact in returned order, and records the output through one `RunnerOutputBuilder`.
The output and all fact response artifacts/events are committed by the existing single append.
Make adapter-facing fact record input/binding constructors private or delete them once no caller
remains.

The runtime binding and `FactRecorded` node/attempt identity already provide producer authority.
Delete `FactProducerProvenance` and its capability/adapter fields from claims, refs, canonical
codecs, event schemas, Postgres projections, public facts, and tests. `FactClaim` has no standalone
schema identity: update its nested canonical shape and the resulting content-addressed
`FactRecorded` descriptor/schema hash, codecs, projections, and fixtures. Do not bump the global
`EVENT_SCHEMA_VERSION`, because the other event variants did not change.

Removing producer fields from `InternalFactRef` also changes `FactQueryReceipt` and
`FactQueryEvidence` canonical bytes. Bump `FACT_QUERY_EVIDENCE_CONTRACT_VERSION` from v2 to a new v3
identity and update `fact_query_evidence_schema_id`; delete the v2 parser and update replay,
retention, query-evidence fixtures, and persisted-surface documentation. The query language and
compiler are unchanged, so do not bump `FACT_QUERY_COMPILER_VERSION`. `FactClaimId` remains the
coordinate-derived run/sequence/ordinal identity.

Keep the existing optional generic request and observation fields as part of the fact query/evidence
contract, but the managed runner always sets both to `None` and exposes no state hook for them. A
managed fact's source semantics belong in its typed response and in preceding external-read
evidence, not in adapter-shaped metadata.

Update certification, runtime commit validation, history validation/input reconstruction, artifact
matching, recovery, and replay together. Enforce that only a certified `ManagedFactWrite` node may
emit `FactRecorded`; node and attempt match the settlement; every claim descriptor equals the one
runner-derived certified descriptor; visibility equals `S::fact_visibility()`; the batch is
non-empty; non-managed settlements contain no facts; managed claims contain no request/observation
metadata; and facts plus output occur in one terminal atomic append. Preserve `NoCaps` for both
`Pure` and `ManagedFactWrite`, and reject any declared managed-write capability in static and dynamic
descriptor validation.

Delete `RecordedFacts`, `RecordedFact`, same-attempt fact-prefix reconstruction, and their recovery
tests. They exist to resume after a fact-only committed prefix, which the atomic managed settlement
makes impossible; recovery before that append simply reruns the deterministic state.

`mfm-replay` owns `verify_managed_fact_write_state<S>`. It reconstructs and invokes `write`, then
requires exact equality for:

- output canonical bytes;
- fact count and event order;
- descriptor, subject, response, content identity, and visibility;
- response artifact bytes/evidence;
- node, attempt, sequence/commit binding, and store commit order;
- absence of missing, extra, duplicate, or reordered facts.

Strengthen the shared fact-batch replay helper; do not implement a domain-specific copy.

### Bitcoin aggregate collection

The final graph is exactly:

```text
BitcoinBalanceCollectionOperation
  -> CollectBitcoinBalancesState
  -> RecordBitcoinBalanceFactsState
```

Use one direct config with only semantic demand:

```text
BitcoinBalanceCollectionConfig
- network_id
- bitcoin_network
- semantic_source_identity
- addresses
```

Validation is deterministic:

- `bitcoin_network` is a checked closed tag;
- addresses contain between 1 and 1,024 entries;
- entries are valid for the selected Bitcoin network, strictly sorted, and unique by both canonical
  address and derived script pubkey;
- use the `bitcoin` (`rust-bitcoin`) crate as the one parsing authority: parse
  `Address<NetworkUnchecked>`, require the
  configured mainnet/testnet/signet/regtest network, require the submitted spelling to equal its
  canonical rendering, and derive `script_pubkey()` from the checked address;
- put that checked address/network authority in the Bitcoin domain and delete the portfolio
  `bech32`/`bs58` implementation plus the weaker Bitcoin address-envelope validator and direct
  dependencies;
- there is no authored read count, retry count, coverage, or status field.

The live executor binds one selected route/session and performs exactly three protocol calls:

1. `getblockchaininfo` to require the selected `chain` and `initialblockdownload == false` (missing
   synchronization status fails closed);
2. one `scantxoutset "start"` containing every sorted `addr(<address>)` descriptor;
3. `getblockhash(scan.height)` to prove that the returned height/hash pair is still canonical.

The scan's `height` and `bestblock` are the shared UTXO snapshot anchor. Tip advancement after the
scan is allowed; a different hash at the recorded height is a reorg and fails the attempt. The
derived read budget is the constant three and is not persisted in config/output. Do not add a
redundant `getblockheader`, `scantxoutset "status"`, or `scantxoutset "abort"` call.

The Bitcoin live transport owns one process-shared asynchronous scan permit per canonical resolved
Bitcoin Core endpoint, excluding credentials from both the key material and diagnostics. Keying by
semantic source ID is insufficient because aliases can address the same node, where
`scantxoutset` is global. Repeated calls and different source aliases for one endpoint serialize;
distinct endpoints proceed independently; an RAII permit is released on success, failure, and task
cancellation. App constructs one live bindings/coordinator instance per `AppServices` and clones
that binding for all routes; do not use ambient global state or construct one coordinator per route.
An externally occupied scan still follows the redacted operational-failure path below.

Bitcoin endpoint validation accepts only HTTP(S) with a host and rejects URL userinfo, query, and
fragment components; authentication remains a separately resolved secret. Derive the private lock
key from the normalized scheme/host/effective-port/path and never persist or render that key.

Set a private `MAX_BITCOIN_JSON_RPC_BODY_BYTES` to 16 MiB. For success and error bodies alike,
reject a larger declared `Content-Length` before allocation and cap chunked accumulation at
`limit + 1` before deserialization. The address limit does not bound the number of returned UTXOs,
so an unbounded `Response::text()`/`bytes()` path is forbidden.

Decode every amount from its raw JSON numeric representation directly to satoshis; conversion
through `f32`/`f64` is forbidden.
Require `success == true` and all mandatory anchor/result fields before considering any balance.
Map each returned UTXO by exact `scriptPubKey` bytes derived from the checked requested addresses;
reject malformed, unknown, or ambiguous scripts. If the returned specialized `desc` is used as an
additional cross-check, validate its checksum and script meaning—never strip a checksum and trust
the remaining string. Parse `bestblock` and every transaction ID with `rust-bitcoin`; decode bounded
script hex; require `vout` to fit `u32`; reject duplicate `(txid, vout)` entries; and require every
matched UTXO height to be at most the scan height. Parse each amount and `total_amount` from the raw
base-10 token with checked decimal-to-satoshi conversion, at most eight fractional digits, and no
exponent or binary float. Checked-add UTXOs by address, preseed zero balances for requested addresses
with no UTXOs, emit results in requested-address order, and require the checked sum of all
per-address balances to equal `total_amount`. Do not compare `txouts` with the match count: Bitcoin
Core defines it as the number of UTXOs scanned. Preserve all collection bounds.

The official [Bitcoin Core `scantxoutset` contract](https://bitcoincore.org/en/doc/26.0.0/rpc/blockchain/scantxoutset/)
accepts multiple scan objects and returns the shared anchor, specialized descriptor, individual
amounts, and total amount. Keep a focused raw-envelope transport test for the exact supported
response form rather than relying on an undocumented mock shortcut.

The external-read evidence contains one redacted source identity, the scan anchor, the ordered
address/satoshi vector, and the final canonicality observation. The reducer reconstructs the plan
and rejects any missing, extra, reordered, duplicated, mismatched-anchor, or mismatched-source
evidence.

The managed state validates the complete observation batch and returns:

- one `BitcoinBalanceSnapshotFact` per configured address in canonical order; and
- one minimal receipt containing network/source binding, shared anchor, ordered address keys, and
  aligned `FactContentIdentityEvidence` values.

The fact response contains only anchor and satoshi balance. The receipt contains no fact payloads,
repeated per-entry anchors, counts, coverage, or status. Remove `CoverageStatus`,
`HoldingSourceStatus`, and the constant coverage output from portfolio observations after their
last surviving use disappears.

Any IO, decoding, arithmetic, anchor, or fact validation failure aborts before settlement. Retry is
for the entire read or write node; a partial snapshot or partial fact batch is never visible.
Treat an already-running node scan as a redacted operational failure under the existing retry
policy. Never issue `scantxoutset "abort"` against a scan this process does not own.

### Production reachability

`mfm.portfolio/snapshot@1` remains the only published run entry point. Build representative valid
Bitcoin-only, EVM-only, and mixed drafts and compute the union of their domain requirements.
Do not add an empty variant unless the portfolio config contract actually permits it.

Excluding explicitly identified framework primitives, require set equality for:

- operation and state descriptors in certification authority;
- emitted fact descriptor artifacts;
- ordinary state runner descriptor IDs;
- declared capability descriptors and concrete implementation bindings;
- adapter executable bindings referenced by reachable state descriptors;
- side-effect verify bindings; and
- replay verifier state/intent coverage.

Every reachable state has exactly one runner; every reachable capability has exactly one concrete
implementation; there are no extra domain registrations. Keep any inspection API test-only or
crate-private. Do not introduce a new public registry inventory abstraction just for this test.

EVM balance collection has its own narrow production registration function. Exact-anchor
validation and transaction submission have separate explicit registration helpers used only by
their library/integration tests. Production start/resume does not construct a signer or transaction
session for a balance-only graph.

### Fact and artifact provider ownership

Move `FactIndexReadCapability`, request/future/provider types, and typed errors into `mfm-facts`.
Implement `FactIndexReadProvider` directly for:

- `PostgresStore`, using its existing single repeatable-read transaction for a batch; and
- `AsyncInMemoryRunStore` under test support.

Delete `PostgresFactIndexReadProvider`, `ProjectionFactIndexProvider`, their public factories, and
app-owned implementation-ID constants. Keep the existing stable implementation-ID values where the
provider identity is unchanged, but move their ownership to `mfm-storage-postgres` and test-support
`mfm-store`, respectively. Provider errors remain typed and redacted; Postgres diagnostics use the
storage-owned implementation identity and never expose SQL, connection data, or query inputs. Test
instrumentation wrappers should be local test types.

Delete `mfm-artifact-capabilities`. Retain `mfm-store::RetainedArtifactReadProvider`,
`EventArtifactRequirement`, and `VerifiedRunArtifactBytes` as the sole authority. Move
`fact_response_artifact_requirement(InternalFactRef)` to `mfm-store`, because `mfm-store` already
depends on facts and the reverse dependency would be invalid. Each domain decodes and validates its
own canonical response bytes; do not retain a generic JSON hydration shim. Portfolio hydration
passes verified bytes to narrow canonical decoders exported by `mfm-bitcoin` and `mfm-evm`.
`mfm-portfolio-live` must not duplicate either response decoder.

### Runtime configuration ownership

Delete `mfm-runtime-config`. `mfm-app` privately owns:

- reading the selected runtime document on `spawn_blocking`, with a 1 MiB hard byte limit;
- the closed top-level JSON/TOML envelope (`bitcoin`, `evm`, `signers`, `keystores`);
- one shared direct/env/file/file-env value-source resolver;
- a 64 KiB hard limit for every direct, environment, or indirection-file value;
- prohibited secret-field checks and redacted public-error mapping; and
- selection of one requested route, signer binding, or keystore profile.

Read files with a bounded `limit + 1` strategy rather than trusting metadata alone. Runtime-document
and indirection-file reads both run on blocking workers; errors redact paths, environment names, and
resolved values. Parse top-level sections and entry maps as raw values, then deserialize exactly the
selected family and selected route/profile. A malformed unrelated family, route, signer, or
keystore must not block the selected path. Selecting a signer parses only that signer and its
referenced keystore. Unknown top-level fields still fail closed. Exactly one supported value source
is required where applicable. The whole JSON/TOML document must still be syntactically valid;
isolation applies to schema-invalid or semantically invalid unselected raw entries.

`mfm-bitcoin-live` and `mfm-evm-live` accept resolved endpoint/auth inputs and own endpoint/session
validation. They do not read files or environment variables. EVM live receives only generic signer
contracts when an explicit transaction test/library binding requests them; it never depends on
`mfm-keystore`.

Move CLI keystore selection, CRUD, and transaction-signing setup behind app services. Production
binaries depend on `mfm-app` only; they must not reach domain, kernel, signing, runtime-document,
live-integration, storage-implementation, or keystore-implementation packages directly. App-owned
request/response APIs are the boundary—do not re-export lower crates as an escape hatch.
Evidence-only commands never load the runtime document.

### Persisted-data cut

When adapter fact provenance is removed:

- change the Postgres authority from `mfm.postgres.store.v3` to `mfm.postgres.store.v4`;
- update the baseline schema and checksums in place rather than adding a compatibility migration;
- remove obsolete fact projection columns, codecs, and decoders;
- reject a store whose metadata still declares v3 with the existing redacted store-authority error
  class;
- document that the database must be recreated; and
- do not keep a v3 fixture, reader, migrator, or automatic repair path.

Prove v3 rejection by corrupting only the authority metadata of a freshly created v4 test store, or
with a validator-unit input. Do not retain a v3 schema/database fixture to perform that test.

Removed/unknown state and operation descriptors continue to fail normal certification before
execution. Do not add special legacy-ID detection; the generic fail-closed errors are the contract.

### Architecture metadata

Replace `[package.metadata.mfm].category` topology assertions with semantic metadata. The final
shape is:

```toml
[package.metadata.mfm]
layer = "domain"                 # example
domain = "bitcoin"              # required for domain/live
domain-role = "source"          # source or aggregate; domain only in the final graph
```

Recognized layers are `kernel`, `domain`, `live`, `signing`, `secret-provider`, `storage`,
`assembly`, `binary`, and `test`. During consolidation, multiple packages may share one layer and
domain; the final graph naturally collapses them. Kernel packages additionally require
`domain-facing = true|false`. A pure domain may directly import only domain-facing kernel APIs;
`mfm-runtime`, `mfm-replay`, and `mfm-store` are platform-only. Certification/program-authoring
helpers may be domain-facing, but execution, replay, and storage authority may not.

Validate metadata shape as well as edges. `layer` is always required; `domain` is required and
allowed only on domain/live packages; `domain-role` is allowed only on domain packages and becomes
required in phase two; and `domain-facing` is required and allowed only on kernel packages. Reject
unknown layers, domains, roles, keys under `package.metadata.mfm`, or invalid value types.

Apply the metadata contract in three monotonic phases, without a named legacy exception:

1. Commit 1 enforces layer direction, pure/live separation, same-domain live isolation, signing,
   keystore, storage, app, and proc-macro properties. Pure-domain-to-pure-domain edges remain
   allowed temporarily because the current Bitcoin state package imports portfolio-owned constant
   coverage/status types. Same-layer edges are temporarily allowed within domain, live, signing,
   secret-provider, and assembly layers while those packages are still split; this also makes the
   current `mfm-app -> mfm-runtime-config` assembly edge valid without inventing a config layer.
2. Commit 11 deletes those constant types and the reverse dependency. In that same commit, require
   `domain-role` and enforce source/aggregate direction. This tightens the generic rule instead of
   allowlisting an old package edge.
3. Commit 18 deletes the last split assembly package and enforces the final no-self-layer matrix.

The final normal/build internal dependency matrix is:

- kernel -> kernel;
- signing -> domain-facing kernel only;
- source domain -> domain-facing kernel/signing only;
- aggregate domain -> domain-facing kernel/signing/source domains only;
- source live -> kernel/signing/its source domain only;
- aggregate live -> kernel/signing/its aggregate domain/source domains only;
- secret-provider -> domain-facing kernel/signing only;
- storage -> kernel;
- assembly -> lower layers, never assembly/binary/test;
- binary -> assembly only;
- test -> unrestricted.

Reject source-domain dependencies on portfolio, cross-domain live edges, live dependencies on
Postgres/keystore/app, signing dependencies on the keystore, and store dependencies on Postgres.
The aggregate-live source-domain allowance exists so portfolio live can call the canonical Bitcoin
and EVM fact-response decoders; it never permits a dependency on another live crate. During phase
one, thin-binary ownership remains enforced by the existing source/API contract tests while the CLI
still has direct lower-layer edges. After commits 17 and 18, enforce binary -> assembly only.

Derive the proc-macro rule from Cargo target kinds: a proc-macro package is separate and does not
also expose a runtime library target. Test the graph validator with synthetic forbidden edges. Do
not assert counts, exact names, directory suffixes, or named exception tables.

Synthetic fixtures must cover missing/extra/wrong-typed metadata; domain -> platform-only kernel;
source -> aggregate; cross-domain live and live -> concrete storage; signing -> secret-provider;
store -> Postgres; final assembly -> assembly; final binary -> any non-assembly layer; and a package
that combines proc-macro and runtime library targets. Include positive aggregate-domain and
aggregate-live -> source-domain cases so the validator does not over-restrict composition.

### Invariant guardrails

This reorganization changes names and persisted identities, not the following semantics:

- Run streams stay append-only. No commit updates or reinterprets an earlier event; store tests
  compare the original prefix before and after every new append.
- One store append stays the atomic unit. Managed facts, response artifacts, state output, and
  terminal events either all appear in one commit order or none appear under injected failures.
- Manifests, specs, descriptors, contexts, facts, artifacts, and outputs remain content addressed.
  Changed shapes receive new hashes/IDs and tampered bytes fail verification.
- Every hashed structure uses the existing canonical JSON authority and contains no floating-point
  value. Bitcoin amounts become checked integer satoshis before entering plan, evidence, fact, or
  output types.
- Operation expansion remains deterministic and performs no IO. Repeated expansion of each
  Bitcoin-only, EVM-only, and mixed portfolio input must produce byte-identical graph material.
- Pure and managed state logic has `NoCaps`; all network, filesystem, environment, database, and
  keystore access remains in live/storage/app/provider code and is prohibited by dependency and
  source-contract tests.
- Replay is evidence-only. Tests install panicking config/live/signer providers and prove ordinary
  pure, managed-fact, external-read, and EVM side-effect replay never touches them.
- A non-empty batched fact query uses one store snapshot/frontier for every result; the empty batch
  performs no database transaction.
- Secrets and bearer mutation material never enter manifests, events, artifacts, context snapshots,
  logs, errors, CLI/REST output, debug rendering, or fixtures. Existing redaction scans and targeted
  negative fixtures remain mandatory after every owner move.
- Keystore zeroization, constant-time comparisons, AAD swap resistance, fail-closed parsing, and
  file/entry/audit bounds retain their existing security tests under the new package name.

## Commit plan

Every commit below must leave the workspace buildable, update affected docs/tests in the same
commit, and use the exact lower-case subject shown. Do not create preparatory compatibility commits.

### 1. `replace topology snapshots with boundary contracts`

- Replace the exact 44-package, six-EVM-package, path-category, and named-exception assertions in
  `tests/integration/tests/cargo_metadata_contract.rs`.
- Replace the old `category` keys with semantic `layer`, `domain`, and kernel `domain-facing`
  metadata in the current packages; do not retain both taxonomies.
- Implement metadata-shape validation plus the phase-one generic edge validator and synthetic
  negative cases described above, including platform-only kernel rejection and transitional
  assembly -> assembly acceptance.
- Preserve useful target-ownership and thin-binary tests, but separate them from package inventory.
- Do not add production reachability assertions yet; dormant surfaces still exist at this point.

Focused verification:

- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo metadata --no-deps --format-version 1`

### 2. `delete unused proof workflow`

- Delete `crates/collectors/proof` (`mfm-collectors-proof`), `crates/ops/proof-op`
  (`mfm-op-proof`), and `crates/transports/proof` (`mfm-transports-proof`).
- Remove workspace members, dependencies, lockfile entries, production runner/certification/replay
  registration, transport-boundary exceptions, examples, and README links.
- Inventory the kernel invariants exercised by proof tests. Add a minimal test-local kernel fixture
  only for an invariant not already covered; do not create another production workflow.
- Verify no `mfm-*-proof` package/import or proof descriptor remains outside the RFC/plan and Git
  history.

Focused verification:

- app tests covering certification, execution, recovery, and replay
- metadata contract test and negative `rg`/`cargo tree` checks

### 3. `delete bitcoin checkpoint collector surface`

- Delete checkpoint query/load/record states and `CollectorCheckpointFact`.
- Delete standalone observe/record chain-head states and `BitcoinChainHeadFact`.
- Delete `BitcoinChainHeadCollectorCycleOperation`, its config/root output, draft/launch helpers,
  app launch helpers, production registrations, replay branches, and dedicated workflow tests.
- For CLI status tests, admit the published portfolio draft without driving it when execution is not
  the subject. Exercise interrupted execution through the surviving portfolio recovery tests. Do
  not create replacement test-local states or operations merely to preserve this deleted workflow.
- Retain the joint-tip read types/state and active network receipt assembler only until commit 11;
  they are still part of the published portfolio graph at this point.
- Retain transport chain-head primitives needed by that active graph until commit 11.

Focused verification:

- Bitcoin state/operation, app run-control, resume, and replay tests
- negative checks for checkpoint and standalone cycle identifiers

### 4. `delete evm standalone collector wrapper`

- Delete only the standalone EVM balance cycle constants, root draft/launch helpers, wrapper output,
  wrapper-specific tests/examples, and app helpers.
- Keep the EVM balance operation and its two active states because portfolio composes them.
- Rewrite ingress/route behavior tests through the published portfolio entry point or move them to
  the EVM live boundary; do not lose malformed/missing-route coverage.
- Keep reusable transaction and exact-anchor validation code, but do not treat library availability
  as production reachability.

Focused verification:

- EVM operation tests, app portfolio behavior tests, and route-ingress failure tests
- negative checks for the standalone cycle helper names

### 5. `limit production assembly to published portfolio graphs`

- Split broad Bitcoin/EVM/portfolio registration helpers so each registers exactly one published
  graph responsibility.
- Remove EVM transaction and exact-anchor validation from production certification, runner,
  capability, transport/session, side-effect verify, and replay assembly.
- Move their tests to explicit local certification and runner registries.
- Remove any signer construction reachable only through the former production transaction binding.
- Add the full reachability set-equality test for Bitcoin-only, EVM-only, and mixed portfolio drafts.
- Keep `entry_point_ids()` equal to only `mfm.portfolio/snapshot@1`.
- Prove selected missing live config fails before `RunAdmitted` and evidence-only construction loads
  no config, transport, or signer.

Focused verification:

- app production assembly/reachability tests
- portfolio start, resume, and evidence-only replay for all family combinations
- explicit local EVM transaction/validation boundary tests

### 6. `fold artifact reads into store`

- Move the fact response artifact requirement constructor into `mfm-store`.
- Change all consumers to use `RetainedArtifactReadProvider` and verified store bytes directly.
- Move mismatch, missing-artifact, and tamper coverage to store/domain tests.
- Delete `mfm-artifact-capabilities`, app `RetainedArtifactReadAdapter`, adapter factory/error mapping,
  generic hydrate helper, facade tests, manifest entries, and docs.
- Ensure no duplicate artifact evidence/request/verified-byte type remains.

Focused verification:

- `cargo test -p mfm-store`
- affected portfolio/Bitcoin/EVM live and replay tests
- negative package/import checks

### 7. `fold effects into capabilities`

- Move effect classes, descriptors, sealed role contracts, tests, rustdoc, and README material from
  `mfm-effects` into `mfm-capabilities`.
- Update all imports and manifests in one mechanical pass.
- Delete `crates/kernel/effects` and its workspace/lockfile entries; add no re-export facade.
- Preserve effect descriptor identity/canonicalization until commit 10 intentionally changes the
  managed-fact effect contract.

Focused verification:

- `cargo test -p mfm-capabilities`
- `cargo test -p mfm-program`
- metadata and negative import checks

### 8. `centralize pure state execution and replay`

- Add the generic runtime pure runner and typed registration helper.
- Add `mfm_replay::verify_pure_state<S>` using exact descriptor identity.
- Migrate portfolio snapshot assembly/report projection and the then-surviving Bitcoin receipt
  assembler.
- Delete every adapter-owned ordinary pure runner, factory identity, loading path, and bespoke pure
  replay body in the same commit.
- Keep specialized test runners only when they inject a failure the generic runner cannot represent.
- Add kernel tests for nested input trees, schema mismatch, invalid config, context/no-context,
  canonical output, output tamper, and zero live/config access.
- Update and rerun the production reachability equality test for the new runner and replay identities.

Focused verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- affected Bitcoin and portfolio adapter/app replay tests

### 9. `fold fact index reads into facts`

- Move `FactIndexReadCapability`, provider/request/future types, and typed errors into `mfm-facts`.
- Implement the provider directly on `PostgresStore` and test-support `AsyncInMemoryRunStore`.
- Preserve one shared snapshot for a non-empty query batch and the empty-batch fast path.
- Delete app Postgres/in-memory provider facades and app-owned implementation-ID constants; move
  the unchanged concrete ID values to the owning Postgres and in-memory store implementations.
- Move instrumentation providers into tests and update portfolio live wiring to accept the trait.
- At the end of this commit, `mfm-fact-capabilities` may contain only the still-used
  `FactRecordCapability`; do not move that marker into `mfm-facts`.
- Update and rerun production reachability for the moved concrete fact-index binding identities.

Focused verification:

- `cargo test -p mfm-facts`
- `cargo test -p mfm-storage-postgres` with focused fact-query parity
- portfolio selection shared-frontier tests

### 10. `centralize managed fact execution and replay`

This is intentionally one authority-cut commit. Splitting it would require a fake generic
capability/adapter provenance or parallel old/new managed-write paths.

- Replace `ManagedWriteState` with the final synchronous `ManagedFactWriteState` contract.
- Add the `ManagedFactWrite`/`NoCaps` effect and runner identity plus the
  visibility-only effect-contract digest described above; delete `ManagedPlatformWrite`.
- Make sealed `EffectRunner<S>` return full descriptors, derive state descriptor refs from them, and
  delete the manual `StateSpec::emitted_fact_descriptors` hook.
- Make certification state registration install the associated fact descriptor artifact; delete
  public manual fact-registration methods, all calls, and `after_registration` fact hooks.
- Add the generic managed fact runner and replay verifier, including order- and visibility-exact
  batch verification.
- Migrate EVM balance fact publication and the then-surviving Bitcoin fact writer.
- Delete `FactRecordCapability`, adapter record bindings/provenance, custom managed runners, custom
  replay bodies, the remaining `mfm-fact-capabilities` package, and unused managed capability roles.
- Delete `FactProducerProvenance`; update `FactClaim`, `InternalFactRef`, event/public/projection
  shapes and runtime/history validation; make the new claim/ref decoders reject obsolete producer
  fields and every unknown field; and delete `RecordedFacts` fact-prefix recovery plumbing.
- Update the content-addressed `FactRecorded` schema hash without changing global
  `EVENT_SCHEMA_VERSION`. Bump fact-query evidence v2 to v3, update its schema ID, delete the v2
  parser, add an explicit v2-rejection test, and leave the unchanged query compiler version alone.
- Update the Postgres baseline/projection and cut store authority to
  `mfm.postgres.store.v4`; no migration or v3 reader.
- Add `rejects_v3_store_authority` by corrupting fresh v4 authority metadata or by using a validator
  unit input; retain no v3 schema fixture. Name the query test
  `rejects_v2_fact_query_evidence` and retain only its malformed v2 literal.
- Update and rerun full production reachability for the runner, effect, fact descriptor, and replay
  changes.
- Update `docs/design.md` and `docs/persisted-public-surfaces.md` for runtime-owned fact publication.

Focused verification:

- program/certify/runtime/replay/facts/events/store tests
- compile-fail coverage proving managed fact writes require `NoCaps`, while pure `NoCaps` remains valid
- EVM/Bitcoin fact publication and app replay tests
- Postgres schema-drift, v3-rejection, atomic append, projection, and fresh-store parity tests
- failure injection proving one bad fact/artifact leaves zero batch events, facts, or output

### 11. `collapse bitcoin collection and live integration`

This commit creates the final live boundary while changing the protocol path. Updating the old
adapter and transport separately would require a temporary public bridge and then delete it one
commit later.

- Create `crates/live/bitcoin` / `mfm-bitcoin-live` by merging
  `mfm-adapters-btc-jsonrpc` and `mfm-transports-btc-jsonrpc-http`; delete both old packages and
  directories in this commit.
- Keep transport/client/session/coordinator modules private. Expose only resolved-endpoint binding
  and the aggregate external-read registration needed by app assembly.
- Introduce the final direct config, aggregate plan/evidence/batch/fact/receipt, two states, and one
  operation described above in the still-split pure Bitcoin packages; commit 12 moves them without
  changing the contract.
- Replace separate chain-head/balance capabilities and providers with the single aggregate
  collection capability and live implementation.
- Add the `bitcoin` (`rust-bitcoin`) workspace dependency to the Bitcoin authority package for
  checked network/address/script/hash/txid types. Delete the portfolio `bech32`/`bs58` validator and
  direct dependencies.
- Implement exactly three calls: `getblockchaininfo`, one multi-descriptor `scantxoutset`, and
  `getblockhash(scan.height)`. Enforce exact decimal-to-satoshi conversion, outpoint uniqueness,
  checked scripts/heights, total consistency, and the shared scan anchor.
- Add the 16 MiB bounded body reader for both success and error responses; delete all unbounded
  response buffering.
- Key the cancellation-safe scan coordinator by canonical resolved endpoint, not source ID. Add no
  status or abort RPC.
- Delete the active joint-tip state, per-address observe/write states, nested at-anchor operation,
  pure receipt assembler, old provider paths, fixed read-count fields, counts, coverage/status,
  weak address validators, schemas, and replay branches.
- Remove constant coverage/status from Bitcoin facts/receipts, portfolio selection material, and
  final portfolio observations; update app DTOs, `bin/cli/README.md`, `bin/rest-api/README.md`, and
  public JSON/text contract tests in the same commit.
- Add `domain-role` metadata, enforce source/aggregate dependency direction, and add synthetic
  negative cases now that Bitcoin no longer depends on portfolio-owned status types.
- Keep runtime-document parsing in `mfm-runtime-config` until commit 18; do not duplicate it in the
  live crate.
- Update and rerun full production reachability for the new graph, capability, executable, and replay
  identities.

Focused verification:

- config: empty, over-limit, unsorted, duplicate, noncanonical, checksum-invalid, and
  network-mismatched addresses plus duplicate script pubkeys
- transport: one session, exactly three RPC calls, one multi-descriptor request, zero-balance
  inclusion, malformed/unknown/ambiguous scripts, invalid txid/vout/height, duplicate outpoints,
  descriptor cross-checks, exact amount/total checks, and no `txouts == matches` assumption
- body bounds: over-limit declared length, chunked overflow, and bounded redacted error bodies
- coordination: repeated same-source calls serialize, two aliases for one endpoint serialize,
  distinct endpoints overlap, cancellation releases the permit, and external busy-node errors redact
- consistency: tip advancement accepted, returned anchor reorg rejected, unsynced/source mismatch
- atomicity/replay: missing/extra/reordered evidence, mid-batch failure, zero partial commit, retry
- package/import negatives for `mfm-adapters-btc-jsonrpc` and
  `mfm-transports-btc-jsonrpc-http`
- portfolio Bitcoin-only and mixed collection/selection/report/replay

### 12. `consolidate bitcoin domain`

- Create `crates/domains/bitcoin` / `mfm-bitcoin` with private `model`, `capability`, `state`, and
  `operation` modules.
- Move the surviving contents of `mfm-btc-capabilities`, `mfm-states-btc`, and
  `mfm-op-btc-collectors` into it.
- Flatten duplicate Bitcoin network/address/source identities and expose only types needed by
  portfolio, Bitcoin live, app inputs, or intentional library consumers.
- Export one narrow canonical balance-fact response decoder for portfolio hydration; it accepts
  canonical bytes and does not depend on `mfm-store`.
- Update imports, manifests, examples, rustdoc, tests, schemas/names, and metadata.
- Delete all three old packages and paths in the same commit; no old crate-name aliases.
- Rerun production reachability and require stable semantic registry sets across this source move.

Focused verification:

- `cargo test -p mfm-bitcoin`
- dependent portfolio, live Bitcoin, app, and integration tests
- negative old-package and old-public-name checks

### 13. `consolidate evm domain`

- Create `crates/domains/evm` / `mfm-evm` with private model/signing/capability/state/operation
  modules.
- Merge `mfm-evm-capabilities`, `mfm-evm-signing`, `mfm-states-evm`, and
  `mfm-op-evm-collectors`.
- Keep balance collection plus reusable transaction and exact-anchor validation foundations.
- Export one narrow canonical balance-fact response decoder for portfolio hydration; it accepts
  canonical bytes and does not depend on `mfm-store`.
- Reduce visibility of package-crossing DTOs and remove duplicate wrapper APIs.
- Preserve generic signer abstraction dependency, canonical quantities, transient signed material,
  lane claims, recovery, and redaction guarantees.
- Delete all four old packages/directories and update every dependent import/test/doc.
- Rerun production reachability and require stable semantic registry sets across this source move.

Focused verification:

- `cargo test -p mfm-evm`
- schema/descriptor tests and explicit transaction/validation composition tests
- dependent portfolio/live/app tests and negative old-package checks

### 14. `consolidate evm live integration`

- Create `crates/live/evm` / `mfm-evm-live`.
- Merge `mfm-adapters-evm` and `mfm-transports-evm`; keep HTTP/session implementation modules
  private and delete both old packages/directories.
- Expose separate narrow registration APIs for production balance collection, test/library exact
  validation, and test/library transaction submission.
- Preserve transaction-specific submit/verify/recovery replay; use generic external-read and
  managed-fact replay for ordinary states.
- Delete broad all-EVM registration helpers.
- Keep runtime-document parsing outside this crate until commit 18.
- Update and rerun production reachability for executable bindings and prove transaction/validation
  remain absent.

Focused verification:

- `cargo test -p mfm-evm-live`
- Reth EIP-1559 parity, transaction ambiguity/recovery/redaction, exact-anchor, and balance tests
- negative package/import checks for `mfm-adapters-evm` and `mfm-transports-evm`

### 15. `consolidate portfolio domain`

- Create `crates/domains/portfolio` / `mfm-portfolio` with private model/state/operation modules.
- Merge `mfm-portfolio-model`, `mfm-state-portfolio`, and `mfm-op-portfolio-snapshot`.
- Reuse checked Bitcoin/EVM domain identities instead of retaining duplicated protocol validators.
- Remove public types that existed only to cross model/state/op package boundaries.
- Preserve deterministic graph construction, exact receipt-pinned fact selection, and report
  projection semantics after coverage/status deletion.
- Delete all three old packages/directories and update imports/docs/tests.
- Rerun production reachability and require stable semantic registry sets across this source move.

Focused verification:

- `cargo test -p mfm-portfolio`
- Bitcoin-only, EVM-only, mixed draft/certification/selection/report tests
- dependent app/live/integration tests and negative old-package checks

### 16. `consolidate portfolio live integration`

- Create `crates/live/portfolio` / `mfm-portfolio-live` from `mfm-adapters-portfolio` and delete the
  old package/directory.
- Keep only fact-selection external-read binding and any final live registration required by
  portfolio; pure states use the kernel runner.
- Accept `mfm-facts::FactIndexReadProvider` and `mfm-store::RetainedArtifactReadProvider` trait
  objects. Do not depend on or wrap Postgres.
- Hydrate verified bytes only through the canonical decoders exported by `mfm-bitcoin` and
  `mfm-evm`; delete duplicate response parsing.
- Delete any remaining app provider facade.
- Update and rerun full production reachability after all domain package consolidation.

Focused verification:

- `cargo test -p mfm-portfolio-live`
- shared-snapshot fact selection, hydration tamper, decoder mismatch, app portfolio, and replay tests
- metadata proving portfolio live has no concrete storage dependency or cross-live edge

### 17. `merge keystore implementation and provider`

- Create `crates/keystore` / `mfm-keystore` with private `crypto`, `store`, and `provider` modules
  and a narrow root API.
- Move `crates/core` (`mfm_core`) keystore/crypto and `crates/signers/keystore`
  (`mfm-signers-keystore`) provider code; delete both old packages/directories.
- Delete every `mfm_core` import and the `mfm-core` Cargo dependency alias. Do not add compatibility
  re-exports or aliases.
- Preserve the current keystore persisted format unless an independently justified security change
  is required; this reorganization is not a format redesign.
- Preserve `Keystore: !Send + !Sync`, zeroized secrets/passwords, constant-time integrity checks,
  AAD swap protection, file/entry/audit bounds, and redacted errors/debug.
- Put CLI keystore CRUD/selection and signing setup behind app services. Every keystore file
  operation uses `spawn_blocking`; construct, use, and drop the `!Send` `Keystore` entirely inside
  that closure. Pass passwords as zeroized secret arguments, never as serializable or `Debug` app
  DTO fields, and return only redacted, non-secret DTOs with non-secret `Debug` implementations.
- Remove production binary dependencies on the implementation crate.
- Keep `dangerous-secret-export` explicit and test both feature configurations.

Focused verification:

- `cargo test -p mfm-keystore`
- the same suite with `--features dangerous-secret-export`
- corruption, truncation, swap, tamper, size/shape DoS, zeroization, redaction, and blocking-provider tests
- CLI/app keystore CRUD, selection, and transaction-signing boundary tests
- negative `mfm_core` import and `mfm-core` dependency-alias checks

### 18. `move runtime configuration into app assembly`

- Implement the one private raw runtime document/value-source resolver in `mfm-app`, using the
  blocking-worker and 1 MiB/64 KiB bounds defined above.
- Move selected endpoint/binder validation to the final Bitcoin/EVM live crates and selected
  keystore/signer binding to app plus `mfm-keystore`/`mfm-signing` contracts.
- Resolve exactly the selected Bitcoin/EVM route or signer/keystore profile; do not deserialize an
  entire selected family first.
- Rename the MFM-owned top-level config family to `bitcoin`; reject `btc` as an unknown key and do
  not accept it as an alias. Keep the removed key only in a named
  `rejects_removed_btc_top_level_key` test.
- Preserve lazy selected-entry parsing and unrelated-malformed-entry isolation.
- Delete public `RuntimeConfig`, `RuntimeConfigRequirement`, source/path wrapper DTOs, full-document
  eager parsing, `mfm-runtime-config`, its docs, and every dependency/import.
- Remove every remaining production binary dependency except `mfm-app`. Tighten metadata to the
  final no-self-layer matrix and binary -> assembly-only rule; do not re-export lower crates from
  app. Binaries pass parsed presentation primitives into app request constructors, and app owns
  conversion to kernel/domain identities.
- Rename `docs/btc-rpc-routing.md` to `docs/bitcoin-rpc-routing.md`; update it together with
  `docs/evm-rpc-routing.md`, app/CLI/REST READMEs, examples, error contracts, and redaction tests.

Focused verification:

- JSON and TOML documents; unknown fields; missing selected entry; exactly-one source selection
- direct/env/file/file-env resolution, declared/growing file bounds, invalid UTF-8, blocking-worker
  execution, path/environment/value redaction, and URL/auth validation
- selected Bitcoin/EVM/signer/keystore entry succeeds despite malformed unrelated sections/entries
- evidence-only status/list/watch/output/replay proves zero runtime-document access
- app, CLI, REST, Bitcoin live, EVM live, and keystore tests
- metadata shape/edge synthetic cases and negative package/import/config-key checks

## Per-commit completion checklist

Before each commit:

1. Run focused `cargo test`/`cargo check` for every touched package and boundary.
2. Run `cargo fmt --all -- --check`.
3. Run `cargo metadata --no-deps --format-version 1` and the metadata contract test.
4. For a deletion/rename, use negative `rg` and `cargo tree` checks. Exclude only
   `RFC_REPO_REORG.md`, this plan, `.git`, and target/build artifacts. An old authority/schema/config
   literal may additionally appear only as malformed input in its specifically named rejection test;
   old imports, decoders, aliases, production fixtures, and positive-path uses must be absent.
5. Run all mandatory managed gates:
   - `nix run .#check`
   - `nix run .#test`
   - `nix run .#test-db`
6. Inspect `git diff --check` and the full staged diff. Commit only the named logical change.

Every new or changed public item, field, method, error contract, and crate entry point must have
current rustdoc in its owning commit; changed crate usage includes at least one compiling rustdoc
example. Preserve `#![warn(missing_docs)]`.

Do not claim a gate passed unless it was run against the exact staged tree.

## Final validation

After commit 18:

- Run `nix run .#ci` and retain its full `closing-source-revision` SHA.
- Require empty `git status --short` output at that SHA.
- Confirm the workspace has the target semantic layers without asserting its package count.
- Report the before/after workspace-package and Rust source-LOC deltas as review information. Any
  unexplained net increase in concepts or LOC blocks closure even though neither delta is a test.
- Confirm removed package/type/config names and old schema/authority values are absent outside the
  RFC, plan, and the exact malformed literals in named rejection tests.
- Confirm production publishes only `mfm.portfolio/snapshot@1` and its registry surfaces equal the
  union of valid Bitcoin-only, EVM-only, and mixed graphs.
- Start, resume, inspect, and evidence-only replay Bitcoin-only, EVM-only, and mixed portfolio runs.
- Confirm ordinary pure and managed fact states have exactly one kernel execution/replay path each.
- Confirm each production binary's only internal normal/build dependency is `mfm-app`.
- Confirm one Bitcoin collection attempt performs exactly three RPCs, one of them a
  multi-descriptor scan, and publishes one atomic fact batch sharing one anchor and commit.
- Confirm EVM transaction/validation local tests still cover signing, redaction, recovery,
  exact-anchor validation, and protocol replay while production assembly excludes them.
- Confirm Postgres v3 authority is rejected, a fresh v4 store passes schema drift/parity, and no
  migration/fallback reader exists.
- Confirm v2 fact-query evidence and an old producer-bearing `FactRecorded` claim are rejected, with
  no old parser or positive fixture retained.
- Confirm default and `dangerous-secret-export` keystore suites pass and no secret-bearing debug,
  error, event, artifact, output, or fixture was introduced.

## Definition of done

The reorganization is done only when all of the following are true:

- the final code has one pure and one live package per product domain;
- the old role packages and directories are deleted, not hidden;
- proof, Bitcoin checkpoint/standalone chain-head, and EVM standalone-cycle surfaces are absent;
- ordinary pure and managed fact execution/replay are runtime/kernel-owned and unique;
- `FactRecordCapability`, adapter fact provenance, and manual emitted-fact registration are absent;
- Bitcoin balance collection is exactly three RPC calls including one multi-descriptor scan, plus
  one atomic managed write;
- fact index and artifact provider ownership follows store boundaries;
- production registrations are exactly graph-reachable across every authority surface;
- runtime config has one private selective parser and no standalone package;
- production binaries have `mfm-app` as their only internal normal/build dependency;
- generic signing remains independent from the secret-bearing keystore implementation;
- old Postgres stores fail closed under the new authority and no compatibility path exists;
- old fact-query evidence and old `FactRecorded` claim shapes fail strict decoding with no
  compatibility parser;
- architecture tests enforce metadata shape, kernel exposure, and dependency properties rather than
  topology inventory;
- all changed public contracts and rustdoc are current; and
- every per-commit gate plus final `nix run .#ci` passes on a clean worktree.
