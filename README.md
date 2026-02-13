# MFM

Experimental, WIP toolkit for on-chain operations built around an event-sourced state machine runtime.

> WARNING: Not production-ready. Do not use on mainnet.

## Documentation

Start here:

- Design contract (source of truth): [`REDESIGN.md`](REDESIGN.md)
- One-page overview + invariants: [`ARCHITECTURE.md`](ARCHITECTURE.md)
- Contribution rules / CI parity: [`AGENTS.md`](AGENTS.md)

User-facing docs:

- CLI docs + output contract: [`bin/cli/README.md`](bin/cli/README.md)
- REST API docs: [`bin/rest-api/README.md`](bin/rest-api/README.md)

Crate docs:

- Core primitives (keystore + config models): [`crates/core/README.md`](crates/core/README.md)
- Runtime: [`crates/machine/README.md`](crates/machine/README.md)
- Proc macros: [`crates/machine-derive/README.md`](crates/machine-derive/README.md)
- SDK (orchestration helpers): [`crates/sdk/README.md`](crates/sdk/README.md)
- Ops (proof op): [`crates/ops/proof-op/README.md`](crates/ops/proof-op/README.md)
- Ops (keystore op): [`crates/ops/keystore-op/README.md`](crates/ops/keystore-op/README.md)
- Storage (EventStore, mem): [`crates/storages/event-store-mem/README.md`](crates/storages/event-store-mem/README.md)
- Storage (EventStore, Postgres): [`crates/storages/event-store-postgres/README.md`](crates/storages/event-store-postgres/README.md)
- Storage (ArtifactStore, fs): [`crates/storages/artifact-store-fs/README.md`](crates/storages/artifact-store-fs/README.md)
- Storage (ArtifactStore, S3/MinIO): [`crates/storages/artifact-store-s3/README.md`](crates/storages/artifact-store-s3/README.md)

Design notes / planning:

- Nixfied vendoring boundaries: [`nixfied/VENDORED.txt`](nixfied/VENDORED.txt)

## Development

Canonical entrypoints:

```bash
nix run .#help
nix run .#dev
nix run .#check
nix run .#test
nix run .#ci -- --basic --summary
nix run .#ci -- --audit --summary
nix run .#ci -- --parity --summary
```

### Contract notes:

- Service hooks (for example `run_hook MINIO_START`) and service apps (for example `nix run .#service::minio::start`) share the same launcher path and argument/slot-env enforcement.
- Local supervisor wrappers in `nixfied/local/default.nix` (`up`, `down`, `svc-*`) are intentional prod-only overrides.
- CI help/docs metadata comes from `nixfied/project/ci.nix` at `commands.ci.api`, and is mirrored into `apps.<system>.ci.meta.nixfied.api`.

### Process-first ops:

- Runtime visibility:
  - `nix run .#process::status`
  - `nix run .#process::status -- --all`
  - `nix run .#process::runs -- --all`
  - `nix run .#process::inspect -- <id>`
- Service diagnostics:
  - `nix run .#service::postgres::events -- --limit 100`
  - `nix run .#service::postgres::log -- --lines 200`
  - `nix run .#service::helios::events -- --limit 100`
- Policy controls (optional overrides):
  - `SERVICE_REUSE_POLICY=never|same-root|same-slot|cross-run`
  - `SERVICE_OWNER_SCOPE=ephemeral|persistent`
  - `SERVICE_DISCOVERY_SCOPE=local|global`
- Migration note:
  - `MFM_KEEP_SERVICES` has been removed from `mfm::portfolio::snapshot`.
  - Start reusable services explicitly via `service::*::start`, then inspect ownership with `process::status`.

### Run binaries:

```bash
nix run .#mfm_cli -- --help
nix run .#mfm_rest_api
```

## License
MIT (see [`LICENSE`](LICENSE)).
