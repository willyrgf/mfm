# mfm-app

Purpose-authorized application facade and qualified structured-runtime assembly.

## Public facade

```text
entry_points
check_ready
admit_run
drive_once
read_public_run
replay_run
read_transition_trace
read_access_audit
export_run
```

Every protected call consumes a fresh `SecretCredential` and derives both tenant scope and a
stable authenticated principal only from the injected policy. Each call requests its exact
purpose grant. Compound same-run grants must return the same tenant and principal.
Export first authorizes the full recursive prior-run source closure under the same target, tenant,
principal, and `Export` purpose, then serializes only after that succeeds; denied dependencies emit
zero bytes. The app verifies the loaded run's tenant before rendering. IDs, cursors, DTOs, and
exports are not bearer authority.

The current public entry points are `mfm.portfolio/snapshot@1` and
`mfm.evm/submit-transaction@1`. Admission authorizes the exact store, operation, configured target,
and invocation before resolving one append-only configured-value revision. EVM configuration
contains transaction semantics only: it cannot assert a tenant, principal, or caller token. After
authorization, the app combines that configuration with the policy-derived tenant and principal
and the selector's bounded caller submission token. The invocation identity remains a separate
per-run identity. Admission performs no provider or signer IO and does not drive the run.

`drive_once` performs at most one structured Runtime action. Reads, recorded replay, trace, audit,
and export use the callback-free store fold. Replay is verification-only and has no live or
caller-supplied comparison path.

## Production composition

`connect_production_application` requires:

- a deployment-supplied `RunAccessPolicy`;
- a deployment-issued opaque exact-target session bundle;
- authoritative PostgreSQL;
- one affine `EvmWalletDeployment` produced only from
  `EvmWalletDeploymentAssemblyInput`: the provider-qualified routing catalog, pending exact private
  RPC inventory, concrete provider-qualified PostgreSQL wallet authority, qualified keystore
  signer, exact submission configuration, portfolio routing and coverage, admission manifests, and
  complete public release material. Callers cannot supply preassembled submission, balance, or
  wallet bindings.

Deployment assembly derives the known EVM signer-interface, key-specific semantic signer,
RFC6979-low-s signing profile, exact broadcast, structured submission, bounded expansion, and
terminal-assurance contracts in code from the qualified live signer's full public identity.
Syntax-valid substitute references fail before the provider lease begins. The sealed deployment
semantics and public signing identity are retained; every later configuration revision is
recomputed against that sealed tuple before history or wallet mutation. The async provider bracket
consumes the catalog and authorities, proves the retained private RPC inventory, rechecks unchanged
provider/signer/store state, and privately constructs all three live binding sets.

Assembly declares and builds exactly the three current structured production program identities in
one qualified registry; registry finalization rejects a missing, extra, or duplicate identity. The
builder supplies the private kernel fact-scanner baseline; it is not a fourth program and is not
added to unrelated program support closures. Assembly then splits the certification verifier from
the Runtime process registry, opens the fenced store, gives the non-cloneable writer to Runtime,
and retains only readers plus the facade. It exposes no raw pool, registry administrator, fence
issuer, signer secret, target session, mutation permit, or generic invoker.

The application receives resolve-only configured-value history. Deployment/maintenance owns its
append writer. The selected revision becomes immutable run admission material.

Standalone CLI/REST bootstraps cannot self-issue target sessions and fail closed with
`AuthoritativeWriterFenceUnavailable`. An embedding deployment injects a fully composed
`Application` or REST `AppState` after issuing the opaque session bundle from deployment-private
login materials.

## Privacy

Credentials and private deployment handles are non-serializable and redacted. Public errors use a
bounded reviewed code/message pair and may carry only the frozen secret-free Runtime attribution:
phase, run, nullable verified head and occurrence, and semantic process contract or store
scope/epoch. They exclude private implementation identity and diagnostics. Programs, histories,
objects, outputs, traces, exports, and logs exclude passwords, mnemonics, private keys, endpoints,
database URLs, provider text, signatures, and signed transaction bytes.

The exact DTO contract is documented in
[docs/recoverability-app-surface-v1.md](../../docs/recoverability-app-surface-v1.md).
