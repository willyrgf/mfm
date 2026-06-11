# Upgrade Notes

This document tracks downstream expectations when the Nixfied v2 flake input changes.

## What To Recheck After A Framework Upgrade

- Update `flake.lock` so the exact Nixfied, nixpkgs, and Rust overlay revisions are recorded.
- Recheck exposed app behavior and the compiled model only when the change is specifically about
  Nixfied behavior.
- Run the repository's focused Cargo architecture checks after accepting the framework update.

## Current Local Conventions

- [`nixfied.nix`](../nixfied.nix) is the project-owned model. It declares `check`, `test`, and `ci`
  workflows plus managed Postgres and Reth services for full CI.
- `.#ci` is full by definition and does not accept v1 `--mode`, `--full`, or `--summary` flags.
- Reth parity tests use the managed local dev node declared in `nixfied.nix`.

## Notes For Future Upgrades

- If a new release changes state placement, re-audit secret exposure around `.env`, keystore files,
  and other local-only inputs before accepting the new default.
- If a new release changes service or task substitution semantics, re-run `nix run .#ci` before
  accepting the update.
