# Upgrade Notes

This document tracks downstream expectations when the vendored Nixfied framework changes.

## What To Recheck After A Framework Upgrade

- `nix run .#help`
- `nix run .#services`
- `nix run .#features`
- `nix run .#validate-env`
- `nix run .#ci -- --mode basic --summary`

## Current Local Conventions

- Ephemeral copies are pinned to `git-files` with `includeUntracked = true` in [`nixfied/project/conf.nix`](../nixfied/project/conf.nix) so local `.env` files are not pulled into the framework's new `nix-source` materialization path.
- Service metadata in [`nixfied/project/conf.nix`](../nixfied/project/conf.nix) uses explicit `sources` entries so the compiled model can describe local Postgres, MinIO, Reth, and Helios backends.
- Helios declares `sourceKinds.local = "real"` and `readiness.profile = "strict"` so framework readiness checks reject shim-like sources for snapshot flows.

## Notes For Future Upgrades

- If a new release adds required discovery docs, prefer adding thin compatibility entrypoints here rather than moving the existing lowercase MFM design docs.
- If a new release changes ephemeral defaults again, re-audit secret exposure around `.env`, keystore files, and other local-only inputs before accepting the new default.
