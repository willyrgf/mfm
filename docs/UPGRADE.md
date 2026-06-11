# Upgrade Notes

This document tracks downstream expectations when the Nixfied v2 flake input changes.

## What To Recheck After A Framework Upgrade

- Update `flake.lock` so the exact Nixfied, nixpkgs, and Rust overlay revisions are recorded.
- Recheck exposed app behavior and the compiled model only when the change is specifically about
  Nixfied behavior.
- Run the repository's focused Cargo architecture checks after accepting the framework update.

## Current Local Conventions

- [`nixfied.nix`](../nixfied.nix) is the project-owned model. It declares `check`, `test`, and `ci`
  workflows and imports upstream Postgres and Reth adapters for full CI.
- `.#ci` is full by definition and does not accept v1 `--mode`, `--full`, or `--summary` flags.
- Reth parity tests use the managed local dev node imported through the upstream adapter.
- MFM's deterministic service windows start at port `28080`, below common OS ephemeral ranges. Keep
  this range dedicated because the Reth adapter currently derives `ws`, `authrpc`, and `p2p` as
  `http+1`, `http+2`, and `http+3` until Nixfied has model-level multi-endpoint reservation.

## Notes For Future Upgrades

- If a new release changes state placement, re-audit secret exposure around `.env`, keystore files,
  and other local-only inputs before accepting the new default.
- If a new release changes service or task substitution semantics, re-run `nix run .#ci` before
  accepting the update.
