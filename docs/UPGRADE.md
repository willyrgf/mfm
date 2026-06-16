# Nixfied Upgrade Notes

This document tracks downstream expectations when the Nixfied v2 flake input changes.

## What To Recheck After A Framework Upgrade

- Update `flake.lock` so the exact Nixfied, nixpkgs, and Rust overlay revisions are recorded.
- Recheck exposed app behavior and the compiled model when the change is specifically about Nixfied
  behavior:
  ```bash
  system=$(nix eval --raw --impure --expr 'builtins.currentSystem')
  nix eval --json ".#apps.${system}" --apply builtins.attrNames
  nix run .#model-check
  ```
- Run the repository's focused Cargo architecture checks after accepting the framework update.

## Local State Compatibility Triage

Nixfied runtime upgrades can intentionally reject old local state instead of silently migrating it.
For example, a run may fail before any task starts with `REGISTRY_CORRUPT` such as
`expected registry user_version 5, got 3`, or with `STATE_UNOWNED` for an old runtime ABI marker.

Use this sequence before changing the project model:

1. Confirm the compiled model and generated app surface are valid:
   ```bash
   system=$(nix eval --raw --impure --expr 'builtins.currentSystem')
   nix eval --json ".#apps.${system}" --apply builtins.attrNames
   nix run .#model-check
   ```
2. Compare the default state root with a fresh temporary state root:
   ```bash
   nix run .#check
   tmpdir=$(mktemp -d)
   NIXFIED_STATE_DIR="$tmpdir" timeout 30s nix run .#check
   ```
   If the fresh state root starts running tasks while the default state root fails at admission or
   state setup, treat the problem as local runtime state compatibility rather than a malformed
   `nixfied.nix`. A timeout or canceled run after tasks start is enough evidence that the fresh
   state root passed the registry and marker gates.
3. Check for live owned processes before cleanup:
   ```bash
   nix run .#ps
   pgrep -af 'nixfied-runtime|postgres|reth|mfm_cli|cargo'
   ```
   `ps` may fail with the same registry schema error because control commands open the registry
   first. In that case, rely on the process scan and avoid deleting state while matching processes
   are still running.
4. If the registry itself is too old for control commands to open, back it up before recreating it.
   On Linux, MFM's default slot registry is outside the slot root at:
   `~/.local/state/nixfied/registry/mfm/dev/0/registry.sqlite3`.
   Adjust this path if `NIXFIED_STATE_DIR` or `XDG_STATE_HOME` is set.
   Move the per-slot registry directory aside, preserving it for inspection:
   ```bash
   backup_root="$HOME/.local/state/nixfied/registry-backups/mfm-dev-0-$(date +%Y%m%d%H%M%S)"
   mkdir -p "$backup_root"
   mv "$HOME/.local/state/nixfied/registry/mfm/dev/0" "$backup_root/0"
   ```
5. Rerun a small gate. If it now reports an old slot marker such as `STATE_UNOWNED`, use Nixfied's
   marker-gated cleanup:
   ```bash
   nix run .#check
   nix run .#clean
   nix run .#check
   ```
6. Finish with the original upgraded surface:
   ```bash
   nix run .#ci
   nix run .#ps
   ```
   A healthy `ps` should reconcile completed tasks and run-scoped services as non-live/stopped.

## Current Local Conventions

- [`nixfied.nix`](../nixfied.nix) is the project-owned model. It exports `check`, `test`, and `ci`
  composite tasks as project verbs and imports upstream Postgres and Reth adapters for full CI.
- `.#ci` is full by definition and does not accept v1 `--mode`, `--full`, or `--summary` flags.
- Reth parity tests use the managed local dev node imported through the upstream adapter.
- MFM's deterministic service windows start at port `28080`, below common OS ephemeral ranges. Keep
  this range dedicated; the upstream Reth adapter models every reserved listener endpoint explicitly.

## Notes For Future Upgrades

- If a new release changes state placement, re-audit secret exposure around `.env`, keystore files,
  and other local-only inputs before accepting the new default.
- If a new release changes service or task substitution semantics, re-run `nix run .#ci` before
  accepting the update.
