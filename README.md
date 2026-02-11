# MFM

A WIP platform for on-chain operations.

### ALERT: Experimental project, not for production or mainnet use. Mostly AI-generated with barely any human review.

## Development (Nixfied)

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

Contract notes:

- Service hooks (for example `run_hook MINIO_START`) and service apps (for example `nix run .#service::minio::start`) share the same launcher path and argument/slot-env enforcement.
- Local supervisor wrappers in `nixfied/local/default.nix` (`up`, `down`, `svc-*`) are intentional prod-only overrides.
- CI help/docs metadata comes from `nixfied/project/ci.nix` at `commands.ci.api`, and is mirrored into `apps.<system>.ci.meta.nixfied.api`.
