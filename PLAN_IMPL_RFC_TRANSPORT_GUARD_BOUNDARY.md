# Implementation Plan: Capability Source Authority Boundary

Status: engineer handoff plan for `RFC_TRANSPORT_GUARD_BOUNDARY.md`.

## Mandatory Rules

Read these before editing:

- `docs/code-quality.md`
- `docs/architecture.md`
- `docs/design.md`
- `RFC_TRANSPORT_GUARD_BOUNDARY.md`

Optimize for fewer concepts, fewer code paths, fewer public types, fewer duplicated
responsibilities, and fewer places future changes must touch. LOC reduction is valuable, but do not
make code cryptic just to make it shorter.

This work is intentionally breaking:

- Do not maintain backward compatibility.
- Do not create fallbacks, compatibility constructors, feature flags, deprecated wrappers, or shims.
- Delete old guard-based code instead of hiding it.
- Delete tests that exist only to preserve the old split responsibility.
- If removed code is needed later, recover it from git history.

Commit progressively. Prefer small commits that compile and have focused tests, but do not preserve
old public paths just to create artificial compatibility. Suggested commit subjects must be lower
case.

If architecture or design decisions are unclear, spawn an architect-agent before coding further and
pass along all rules in this section.

## Target Architecture

The only public external-source authority boundary is the capability request/provider contract.

The steady-state call path is:

```text
certified semantic config/context/input
  -> adapter builds a capability request
  -> provider backend reads request and enforces source authority
  -> state normalizes successful provider response
```

Live and replay use the same provider trait:

- live provider backend: resolves runtime route and probes the external source
- recorded provider backend: checks recorded response/evidence against the request

After a successful provider response, callers do not re-validate provider source evidence.

States do not construct EVM/BTC provider requests, do not construct guard objects, and do not know
whether a provider response came from live IO or recorded replay evidence.

## Forbidden Patterns

Do not add or keep:

- public `EvmChainGuard` / `BtcChainGuard` usage outside provider-backend internals
- public request `guard` fields
- public `verify_guard` or `verify_request` methods
- adapter-side response source re-verification after provider success
- state-side provider request construction
- app replay guard reconstruction
- public BTC replay helper functions that validate recorded source evidence directly
- no-IO APIs named `validate_guard`
- compatibility aliases, deprecated wrappers, hidden fallbacks, feature flags, or old constructors
- `#[allow(dead_code)]` just to keep old APIs around

## Implementation Order

Use BTC first. It is the clearest current violation and is small enough to validate the model before
the broader EVM cleanup.

### Commit 1: btc capability requests own private source authority

Suggested subject: `btc capability requests own source authority`

Files:

- `crates/btc-capabilities/src/lib.rs`
- `crates/btc-capabilities/tests/capability_contract.rs`

Work:

- Delete public `BtcChainGuard`.
- Add private request source authority, for example `BtcRequestSource`.
- Make `BtcChainHeadRequest` fields private.
- Make `BtcBalanceReadRequest` fields private.
- Add constructors/accessors:
  - `BtcChainHeadRequest::new(network_id, source_identity, bitcoin_network, selection)`
  - `BtcBalanceReadRequest::new(network_id, source_identity, bitcoin_network, address, block_height, block_hash)`
  - `network_id()`
  - `source_identity()`
  - `bitcoin_network()`
  - request-specific accessors such as `selection()`, `address()`, `block_height()`, `block_hash()`
- Update `RedactedBtcSourceEvidence` constructors to build from request/source authority.
- Delete public `RedactedBtcSourceEvidence::verify_guard`.
- Delete public `BtcChainHeadResponse::verify_request`.
- Delete public `BtcBalanceReadResponse::verify_request`.
- Add provider trait docs stating successful responses already enforced request source authority and
  request-local invariants.
- Update capability tests to check constructors, accessors, diagnostics, and evidence construction.
  Do not keep tests proving public guard helpers work.

Focused verification:

```bash
cargo test -p mfm-btc-capabilities
```

### Commit 2: btc live provider enforces request authority internally

Suggested subject: `btc transport centralizes source enforcement`

Files:

- `crates/transports/btc-jsonrpc-http/src/lib.rs`
- `crates/transports/btc-jsonrpc-http/src/tests.rs`

Work:

- Update all request field access to use request accessors.
- Add internal `verified_source` or `verified_transport` helper.
- Route lookup should use `request.source_identity()`.
- `getblockchaininfo` source probing should happen before operation-specific RPC behavior.
- Evidence should be constructed only after the observed Bitcoin network tag matches the request.
- Delete response `verify_request` calls from the transport.
- Keep unsupported balance reads after source verification when source authority matters.
- Rename `validate_guard` to a no-IO route/source binding name such as `validate_source_binding`.
  Its input should be only what route lookup needs: `&BtcSourceIdentity`.
- Add or update tests:
  - missing route is redacted
  - observed network mismatch fails with `SourceMismatch`
  - operation-specific RPC does not run after source mismatch
  - successful response evidence matches request by construction
  - diagnostics do not leak URLs, auth, file paths, request/response bodies, or provider messages

Focused verification:

```bash
cargo test -p mfm-transports-btc-jsonrpc-http
```

### Commit 3: btc state stops constructing provider requests

Suggested subject: `btc state drops provider request construction`

Files:

- `crates/states/btc/src/lib.rs`
- `crates/states/btc/Cargo.toml` if dependencies become unused

Work:

- Remove imports of `BtcChainGuard` and `BtcChainHeadRequest`.
- Delete `ObserveBtcChainHeadConfig::request`.
- Delete `ObserveBtcChainHeadState::request`.
- Add `ObserveBtcChainHeadConfig::selection()` or a local equivalent for semantic head selection.
- Change checkpoint compatibility validation to use config plus selection, not a request.
- Change `normalize_chain_head_response` to accept config/response/input, not request/response/input.
- Change `ObserveBtcChainHeadState::materialize_response` accordingly.
- Keep state validation limited to semantic config, checkpoint compatibility, and deterministic
  observation ordering.
- Delete tests that construct BTC provider requests inside state tests.
- Rewrite state tests to use config/selection and provider responses directly.

Focused verification:

```bash
cargo test -p mfm-states-btc
```

### Commit 4: btc adapter builds requests and recorded provider replaces replay helpers

Suggested subject: `btc adapter uses provider backend for replay evidence`

Files:

- `crates/adapters/btc-jsonrpc/src/lib.rs`
- `crates/adapters/btc-jsonrpc/src/tests.rs`

Work:

- Add adapter-local `chain_head_request(&ObserveBtcChainHeadConfig)` builder.
- Build `BtcChainHeadRequest` immediately before calling `BtcChainHeadReadProvider`.
- Delete adapter-side `response.verify_request(&request)`.
- Delete public `verify_recorded_chain_head_evidence`.
- Delete public `replay_chain_head_fact_from_evidence`.
- Add `RecordedBtcChainHeadProvider` or similarly named backend implementing
  `BtcChainHeadReadProvider`.
- Recorded provider should accept recorded response/evidence supplied by replay assembly or tests.
- Recorded provider should validate recorded source evidence and request-local fields internally.
- Replay tests should call the same adapter/provider path, or directly call the recorded provider
  when testing provider contract behavior. They should not call free source-validation helpers.

Focused verification:

```bash
cargo test -p mfm-adapters-btc-jsonrpc
```

### Commit 5: update btc app and portfolio wiring

Suggested subject: `btc runtime binding drops guard validation`

Files:

- `crates/adapters/portfolio/src/lib.rs`
- `crates/adapters/portfolio/src/tests.rs`
- `crates/app/src/lib.rs`
- related app/portfolio tests

Work:

- Replace `validate_btc_guard` with a route/source binding method name.
- Pass only fields needed for no-IO route lookup.
- Update BTC request construction to use constructors and accessors.
- Delete BTC adapter/portfolio response `verify_request` calls after provider success.
- Keep workflow-level checks such as exact anchor fields inside provider backend or explicit
  workflow checks when they are not source authority.

Focused verification:

```bash
cargo test -p mfm-adapters-portfolio
cargo test -p mfm-app
```

### Commit 6: evm capability requests own private source authority

Suggested subject: `evm capability requests own source authority`

Files:

- `crates/evm-capabilities/src/lib.rs`
- `crates/evm-capabilities/tests/*`

Work:

- Delete public `EvmChainGuard`.
- Add private request source authority, for example `EvmRequestSource`.
- Make every EVM request's source authority private.
- Add constructors/accessors for all request types:
  - chain identity
  - block read
  - balance read
  - call read
  - code read
  - logs read
  - nonce read
  - fee read
  - gas estimate
  - transaction submit
  - receipt read
  - nonce occupancy read
- Delete public `RedactedEvmSourceEvidence::verify_guard`.
- Add provider trait docs stating successful responses enforce request source authority by
  construction.
- Delete guard-verification tests and replace them with request constructor/evidence diagnostics
  tests.

Focused verification:

```bash
cargo test -p mfm-evm-capabilities
```

### Commit 7: evm live provider uses private request source authority

Suggested subject: `evm transport uses request source authority`

Files:

- `crates/transports/evm/src/lib.rs`
- `crates/transports/evm/src/tests.rs`

Work:

- Update `verified_source` to read source authority through request/source accessors.
- Update every EVM provider implementation to use constructors/accessors.
- Delete any transport dependence on public `EvmChainGuard`.
- Rename `validate_guard` to `validate_route_binding`.
- Ensure no-IO route binding takes `&EvmNetworkId` rather than a full request source object.
- Keep existing source-mismatch and redaction behavior.

Focused verification:

```bash
cargo test -p mfm-transports-evm
```

### Commit 8: evm adapters stop source re-verification

Suggested subject: `evm adapters trust provider source enforcement`

Files:

- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/adapters/evm-contracts/src/tests.rs`
- `crates/adapters/portfolio/src/lib.rs`
- `crates/adapters/portfolio/src/tests.rs`
- related app wiring

Work:

- Replace all `EvmChainGuard` construction with request constructors from certified semantic fields.
- Delete every adapter-side `response.evidence.verify_guard(...)` after successful provider calls.
- Keep checks that are not provider source enforcement:
  - signed payload hash versus prepared expected transaction hash
  - returned submit transaction hash
  - receipt transaction hash and receipt status
  - lifecycle context/artifact/schema/phase checks
  - validation assertion result checks
- Update tests that used mismatched provider evidence to expect provider-backend failures instead, or
  delete them if they only tested adapter defensive re-verification.

Focused verification:

```bash
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-adapters-portfolio
```

### Commit 9: app replay diagnostics stop reconstructing guards

Suggested subject: `app replay stops reconstructing source guards`

Files:

- `crates/app/src/lib.rs`
- `crates/app/tests/*`

Work:

- Delete app replay code that reconstructs `EvmChainGuard` values to validate source diagnostics.
- Keep validation of diagnostic artifact digest, schema, shape, and redaction policy.
- Do not move source authority validation into app replay. Recorded provider backends own it.
- Delete tests whose only purpose is proving app replay validates source authority from reconstructed
  guards.

Focused verification:

```bash
cargo test -p mfm-app
```

### Commit 10: docs and final guard cleanup

Suggested subject: `docs align source authority provider boundary`

Files:

- `docs/design.md`
- `docs/architecture.md`
- `docs/evm-rpc-routing.md`
- `docs/btc-rpc-routing.md`
- `docs/persisted-public-surfaces.md`
- `bin/cli/README.md` if it still says adapters derive guards
- crate README files that mention guards
- `RFC_TRANSPORT_GUARD_BOUNDARY.md` if implementation reveals needed wording tweaks

Work:

- Replace "guard" language with capability request/provider source authority.
- Remove "replay verifies against request guard" language.
- State that live and recorded provider backends enforce source authority behind the same provider
  trait.
- Document no-IO route binding as route binding, not guard validation.

Focused verification:

```bash
cargo fmt --all -- --check
cargo check --workspace
```

## Final Sweep

Run these searches and remove old paths until only intentional references remain:

```bash
rg -n "BtcChainGuard|EvmChainGuard" crates bin tests docs
rg -n "verify_guard|verify_request|validate_guard" crates bin tests docs
rg -n "request guard|transport guard|derive.*guard|guarded request" docs crates bin
rg -n "\\.guard\\b" crates/adapters crates/states crates/transports crates/*-capabilities crates/app
```

Expected remaining references:

- RFC/planning docs may mention removed symbols as historical examples.
- Non-source concepts such as test mutex guards or mutation guards are unrelated; do not rename them
  just because they contain "guard".

Final focused verification before opening/merging:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm-btc-capabilities
cargo test -p mfm-states-btc
cargo test -p mfm-adapters-btc-jsonrpc
cargo test -p mfm-transports-btc-jsonrpc-http
cargo test -p mfm-evm-capabilities
cargo test -p mfm-transports-evm
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-adapters-portfolio
cargo test -p mfm-app
```

Before each final commit or PR handoff, follow repository policy:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

Run `nix run .#ci` after major work or for final merge-readiness validation.

## Definition Of Done

Done means all of these are true:

- No state or op constructs EVM/BTC provider requests or source guard objects.
- Public `EvmChainGuard` and `BtcChainGuard` are gone from state/adapter-facing APIs.
- Capability request source fields are private and exposed through constructors/accessors.
- Live provider backends enforce source authority before successful responses.
- Recorded provider backends enforce recorded source evidence before successful responses.
- Adapters do not call `verify_guard` or `verify_request` after provider success.
- BTC replay source validation is no longer exposed as public helper functions.
- App replay does not reconstruct source guards.
- No no-IO API is named `validate_guard`.
- Tests prove provider contracts, not duplicated adapter/state source verification.
- Docs describe one boundary: capability request/provider source authority.
- No compatibility shim or old callable path remains.

## Architect-Agent Escalation Prompt

If a design decision is unclear, stop and spawn an architect-agent with this prompt:

```text
We are implementing RFC_TRANSPORT_GUARD_BOUNDARY.md in MFM.
Rules: optimize for fewer concepts, fewer code paths, fewer public types, fewer duplicated
responsibilities, and fewer places future changes must touch. LOC reduction is valuable, but do not
make code cryptic just to make it shorter. No backward compatibility. No fallbacks, shims,
deprecations, feature flags, or hidden old paths. Old code must be deleted; git history is the
recovery path. Read docs/code-quality.md. If the current design choice conflicts with these rules,
recommend the simpler boundary-preserving change.
Question: <specific decision here>
```
