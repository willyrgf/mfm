# MFM

Experimental, WIP toolkit for recoverable on-chain operations built around a certified typed-state
runtime and one append-only committed journal.

> WARNING: Not production-ready. Do not use on mainnet.

## Architecture at a glance

```mermaid
flowchart TD
    B["bin/cli<br/>bin/rest-api<br/>(transport only)"] --> A["crates/app<br/>(typed assembly)"]
    A --> P["program + spec + certify<br/>(deterministic planning)"]
    A --> R["crates/kernel/runtime<br/>(one-action driver)"]
    R --> S["crates/kernel/store<br/>(verified journal authority)"]
    S --> J["crates/kernel/journal<br/>(frozen persisted values)"]
    A --> RP["crates/kernel/replay<br/>(verification and export)"]
    A --> AD["crates/live/*<br/>(audited capability adapters)"]
    AD --> TP["protocol transports<br/>(one bounded operation)"]
    S --> ST["crates/storages/postgres<br/>(qualified writer)"]
    R --> EX["crates/kernel/executor<br/>(durable keyed effects)"]

    classDef transport fill:#e8f0ff,stroke:#2f5aa8,color:#0f2d63,stroke-width:1px;
    classDef orchestration fill:#eefbe7,stroke:#3a7a2a,color:#1d4d12,stroke-width:1px;
    classDef statecore fill:#ffe8cf,stroke:#a84b00,color:#5a2b00,stroke-width:3px;
    classDef engine fill:#fff3df,stroke:#a66a00,color:#5a3a00,stroke-width:1px;
    classDef storage fill:#f3ebff,stroke:#6d3da8,color:#39136b,stroke-width:1px;
    classDef adapter fill:#e9f8f7,stroke:#0d7a77,color:#084645,stroke-width:1px;

    class B transport;
    class A,P orchestration;
    class R,J,RP,EX engine;
    class S,ST storage;
    class AD adapter;
    class TP transport;
```

Typed state programs are the semantic executable surface. Certification deterministically expands
them into the one runtime graph. States own reusable semantics; adapters bind their intent to
explicit capabilities; transports, executors, and signers remain reusable platform primitives.
Binaries only parse input, invoke the application facade, and render its reviewed outputs.

## Core capabilities

- Certified typed runs with five append-only journal record kinds.
- Stateless, crash-safe `drive_once` scheduling derived from verified history.
- Audited reads and durable keyed effects with authorization-before-access and
  observation-before-settlement.
- Content-addressed manifests, snapshots, facts, and outputs.
- Callback-free recorded-history verification and deterministic portable export.
- Purpose-bound tenant authorization at the application boundary.
- Security-hardened Ethereum keystore (tamper checks + signing utilities).
- Atomic in-memory conformance storage and a deployment-qualified PostgreSQL writer.

## Current workflow surface

The compiled application exposes exactly one public run objective:
`mfm.portfolio/snapshot@1`. It composes an audited, anchor-confirmed EVM read graph and a pure
portfolio aggregation. Bitcoin collection remains unregistered until its provider work and
concurrency behavior qualify. Commit 5 contains no production mutation registration; EVM writes
qualify separately through the durable keyed executor.

## Documentation

Start here:

- Design contract (source of truth): [`docs/design.md`](docs/design.md)
- Architecture taxonomy + placement rules: [`docs/architecture.md`](docs/architecture.md)
- Rust build and verification contract: [`docs/build-and-verification.md`](docs/build-and-verification.md)
- AI-agent contribution rules: [`AGENTS.md`](AGENTS.md)
- Code quality policy: [`docs/code-quality.md`](docs/code-quality.md)

User-facing docs:

- Run execution from admission through fan-out and public output:
  [`docs/run-execution.md`](docs/run-execution.md)
- CLI docs + output contract: [`bin/cli/README.md`](bin/cli/README.md)
- REST API docs: [`bin/rest-api/README.md`](bin/rest-api/README.md)
- Portfolio snapshot workflow: [`docs/portfolio-snapshot.md`](docs/portfolio-snapshot.md)
- EVM runtime routing runbook: [`docs/evm-rpc-routing.md`](docs/evm-rpc-routing.md)
- EVM transaction contract: [`docs/evm-transactions.md`](docs/evm-transactions.md)
- Persisted/public surface inventory: [`docs/persisted-public-surfaces.md`](docs/persisted-public-surfaces.md)

Crate docs:

- Keystore and crypto primitives: [`crates/keystore/README.md`](crates/keystore/README.md)
- Frozen journal values: [`crates/kernel/journal/README.md`](crates/kernel/journal/README.md)
- Typed runtime: [`crates/kernel/runtime/README.md`](crates/kernel/runtime/README.md)
- Typed store contract: [`crates/kernel/store/README.md`](crates/kernel/store/README.md)
- Typed replay: [`crates/kernel/replay/README.md`](crates/kernel/replay/README.md)
- Pure EVM domain: [`crates/domains/evm/README.md`](crates/domains/evm/README.md)
- Pure portfolio domain: [`crates/domains/portfolio/README.md`](crates/domains/portfolio/README.md)
- EVM audited transport bindings: [`crates/live/evm/README.md`](crates/live/evm/README.md)
- Qualified journal storage (PostgreSQL): [`crates/storages/postgres/README.md`](crates/storages/postgres/README.md)

Development and operations:

- Nixfied project model: [`nixfied.nix`](nixfied.nix)
- Nixfied integration and upgrade guide:
  [upstream adopter guide](https://github.com/willyrgf/nixfied/blob/HEAD/docs/GUIDE.md)

## Development

Use the pinned default Nix shell for Rust development. The
[build and verification contract](docs/build-and-verification.md) owns the focused commands, task
selection, gate composition, and artifact policy.

Discover runnable flake apps and run binaries locally:

```bash
nix run .#help
nix run .#mfm -- --help
nix run .#mfm -- ops list
nix develop -c cargo run -p mfm -- --help
nix develop -c cargo run -p mfm-rest-api
```

The [CLI reference](bin/cli/README.md) owns commands, output behavior, and the custom `.#mfm`
app's managed PostgreSQL lifecycle.

## License
MIT (see [`LICENSE`](LICENSE)).
