# MFM

Experimental, WIP toolkit for on-chain operations built around an event-sourced state machine runtime.

> WARNING: Not production-ready. Do not use on mainnet.

## Architecture at a glance

```mermaid
flowchart TD
    B["bin/cli<br/>bin/rest-api<br/>(transport only)"] --> A["crates/app<br/>(typed assembly)"]
    A --> O["crates/ops/*-op<br/>(typed program planning)"]
    O --> SH["crates/states/*<br/>(typed executable states)"]
    SH --> AD["crates/adapters/*<br/>(runner bindings)"]
    AD --> CAP["capability + signer contracts<br/>(typed authority)"]
    CAP --> TP["crates/transports/*<br/>(live protocol backends)"]
    CAP --> SG["crates/signers/*<br/>(signer providers)"]
    A --> R["crates/kernel/runtime<br/>(typed scheduler)"]
    R --> K["crates/kernel/*<br/>(ids, values, spec, events, store, replay)"]
    R --> ST["crates/storages/*<br/>(mfm-store implementations)"]

    classDef transport fill:#e8f0ff,stroke:#2f5aa8,color:#0f2d63,stroke-width:1px;
    classDef orchestration fill:#eefbe7,stroke:#3a7a2a,color:#1d4d12,stroke-width:1px;
    classDef statecore fill:#ffe8cf,stroke:#a84b00,color:#5a2b00,stroke-width:3px;
    classDef engine fill:#fff3df,stroke:#a66a00,color:#5a3a00,stroke-width:1px;
    classDef storage fill:#f3ebff,stroke:#6d3da8,color:#39136b,stroke-width:1px;
    classDef adapter fill:#e9f8f7,stroke:#0d7a77,color:#084645,stroke-width:1px;

    class B transport;
    class A,O orchestration;
    class SH statecore;
    class R,K engine;
    class ST storage;
    class AD adapter;
    class CAP,TP,SG transport;
```

Typed state programs are the semantic executable surface. Ops plan typed programs, the certified typed execution spec is the runtime contract, states declare reusable semantics, adapters bind state intent to capability contracts, transports/signers provide reusable platform implementations, and binaries stay transport-only.

## Core capabilities

- Event-sourced certified typed runs with append-only execution history.
- Crash-resume and replay-aware typed execution semantics.
- Certified saga remediation with signed manual authorization decisions.
- Content-addressed manifests, snapshots, facts, and outputs.
- Deterministic typed-state orchestration for ops/pipelines.
- Thin CLI and REST transport layers for stable automation surfaces.
- Security-hardened Ethereum keystore (tamper checks + signing utilities).
- Typed storage backends for run events and artifacts.

## Documentation

Start here:

- Design contract (source of truth): [`docs/design.md`](docs/design.md)
- Certified saga contract: [`docs/saga.md`](docs/saga.md)
- Architecture taxonomy + placement rules: [`docs/architecture.md`](docs/architecture.md)
- Contribution rules / CI parity: [`AGENTS.md`](AGENTS.md)
- Code quality policy: [`docs/code-quality.md`](docs/code-quality.md)

User-facing docs:

- CLI docs + output contract: [`bin/cli/README.md`](bin/cli/README.md)
- REST API docs: [`bin/rest-api/README.md`](bin/rest-api/README.md)
- EVM contract state contract: [`docs/evm-contract-states.md`](docs/evm-contract-states.md)
- EVM runtime routing runbook: [`docs/evm-rpc-routing.md`](docs/evm-rpc-routing.md)

Crate docs:

- Core primitives (keystore + crypto): [`crates/core/README.md`](crates/core/README.md)
- Typed runtime: [`crates/kernel/runtime/README.md`](crates/kernel/runtime/README.md)
- Typed store contract: [`crates/kernel/store/README.md`](crates/kernel/store/README.md)
- Typed replay: [`crates/kernel/replay/README.md`](crates/kernel/replay/README.md)
- Ops (proof op): [`crates/ops/proof-op/README.md`](crates/ops/proof-op/README.md)
- Storage (typed run events and catalog values, Postgres): [`crates/storages/postgres/README.md`](crates/storages/postgres/README.md)

Design notes / planning:

- Nixfied v2 project model: [`nixfied.nix`](nixfied.nix)
- Framework upgrade notes: [`docs/UPGRADE.md`](docs/UPGRADE.md)

## Development

Use focused Cargo verification by default. Start external services such as Postgres or Reth manually
when a parity test needs them, then run the targeted test against those live endpoints.

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo test -p mfm-integration-tests --test cargo_metadata_contract
```

Useful focused parity examples:

```bash
DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:5432/mfm_test cargo test -p mfm-integration-tests --features parity-tests --test parity_rest_api_postgres_smoke
```

Nixfied v2 gates are available when changing Nixfied behavior or running the repository CI
surface:

```bash
nix run .#model-check
nix run .#check
nix run .#test
nix run .#test-db
nix run .#ci
```

`nix run .#model-check` validates the compiled model without executing project tasks.

`nix run .#test` runs `cargo nextest run --workspace` followed by
`cargo test --workspace --doc`, without managed external services.
`nix run .#test-db` starts managed Postgres, checks crate-local SQLx metadata
against a migrated schema, and runs Postgres-backed parity tests.
`.#ci` is full by definition: it starts the managed services required by feature-gated parity
tests. There is no `--mode` or `--full` alias flag.

Run binaries locally:

```bash
nix run .#mfm -- --help
nix run .#mfm -- setup import --file examples/setup/organization.toml
nix run .#mfm -- ops list
cargo run -p mfm -- --help
cargo run -p mfm-rest-api
```

`.#mfm-start` is a local development wrapper for `mfm run start`. It starts a
Nixfied-managed PostgreSQL process in slot 9, keeps the dev database under the
Nixfied state root for `mfm/dev/9`, applies the typed run-store migrations
idempotently, sets `DATABASE_URL`, and then delegates to the existing CLI. The
raw packaged CLI remains `.#mfm`.

## License
MIT (see [`LICENSE`](LICENSE)).
