# Portfolio Snapshot

Status: current production operation guide

## Material uncertainties

none

The published entry point remains `mfm.portfolio/snapshot@1`. Its input is a strict
`PortfolioSnapshotSelector` naming one configured portfolio target. App resolves the current exact
append-only configured-value revision for `(store, tenant, operation, target)` and validates the
`PortfolioConfig` before program authoring.

## Structured program

The portfolio operation is declaration ordered and deterministic:

```text
configured portfolio + admitted routing manifest
  -> compile ordered EVM collection roots
  -> portfolio FanOut
       -> EVM-owned balance Read FanOut per collection
  -> aggregate ordered child results
  -> OperationOutcome<PortfolioPublicOutputs, PortfolioSnapshotFailure>
```

This is the certified depth-two fan-out case. Both levels are non-empty, collect-all, and limited
to `Pure` and `Read`. Physical completion order cannot change lane or output ordering. The EVM
domain owns the nested collection authoring helper and the same standalone balance operation;
portfolio owns only the outer topology and final aggregation.

The operation contains no Bitcoin live path in this cutover. Portfolio validation retains Bitcoin
model support, but no unregistered source can enter the structured executable.

## EVM collection

Each collection checks the admitted route and chain identity, captures one anchor, reads native or
ERC-20 balances in declared order, confirms that the anchor remains acceptable, and aggregates an
`EvmBalanceCollectionResult`. Network routing is admission material and each live Read is bound to
its registered public physical certificate.

The portfolio aggregate validates exact positions and caller context, preserves admitted
configuration order, and emits one `PortfolioPublicOutputs`. Invalid or incomplete collection
evidence follows the typed `PortfolioSnapshotFailure` handler and closes as an ordinary failed
operation.

## Configuration and authority

Portfolio configuration history is outside RunHistory and append-only. Deployment owns append
authority; the application can only resolve the current exact contract. The selected revision is
content-addressed and admitted with the run, so later configuration changes cannot rewrite an
existing run.

The routing manifest and policy are immutable qualified deployment material. The portfolio domain
does not open transports, select live endpoints, read a database, or construct process authority.
`mfm-evm-live` supplies the registered bounded Read invokers during application assembly.

## Replay and output

Public read, trace, audit, replay verification, and portable export use the callback-free store
fold. They do not resolve current configuration or routing and do not call EVM. The terminal public
output is selected from the exact committed nominal root outcome and its content-addressed value.

## Verification

Certification coverage freezes the exact production portfolio template, depth-two fan-out bound,
child substitution, policy/implementation closure, and deterministic bytes. Runtime/store tests
exercise declaration-order cursor semantics; EVM balance process tests exercise both native and
token lanes; application and transport tests retain the published entry id and strict DTO surface.
