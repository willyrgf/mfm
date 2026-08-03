# Post-RFC Follow-up Work

This file contains only work intentionally outside the structured Runtime history choke-point
cutover.

## Material uncertainties

- The pinned managed Reth topology's `finalized` tag and canonical-inclusion behavior has not been
  requalified against the current structured submission observation contract. If it cannot supply
  that contract, change the parity topology; do not weaken terminal assurance.
- The deployment host API for injecting both the run-history fence and sealed wallet target/session
  provider into standalone transport processes remains deployment-owned. The parity test must
  exercise the same public embedding seams without adding an in-repository self-attestation path.

## Real Reth parity

The current qualification uses a loopback JSON-RPC server and real structured Runtime, signer, and
PostgreSQL wallet authority. It does not prove behavior against a real Reth implementation.

Add a pinned managed Reth target that:

- provisions a qualified physical chain instance and route membership;
- submits one deterministic type-2 transaction through the registered exact-broadcast Effect;
- recovers through exact-hash transaction/receipt/finalized-head/canonical-inclusion Reads;
- exercises process restart without a second broadcast;
- completes through the real PostgreSQL wallet authority; and
- proves provider responses and diagnostics remain redacted.

The target must use the current structured program and wallet authority. It must not restore the
old EVM session, add hidden retries, or count a mock/loopback run as real-service parity.

## REST and PostgreSQL production parity

The REST router is fully testable with an injected `Application`, while the standalone bootstrap
correctly fails closed without deployment authority. Add a deployment-host integration target that:

- provisions the structured RunHistory/configuration schema and wallet schema with their distinct
  production roles;
- supplies a real run-history writer fence and sealed wallet target/session provider;
- constructs `mfm_app::Application` through `connect_production_application`;
- injects it into `mfm_rest_api::AppState` and runs the real HTTP server;
- admits and drives portfolio and submission runs across process restart;
- checks independent tenants/run ids plus read/replay/trace/audit/export routes; and
- proves unavailable databases, stale fences, or missing wallet qualification fail without an
  in-memory or self-attested fallback.

Do not add run list/watch, start/resume, fact, generic-object, status-stream, or manual-resolution
routes.
