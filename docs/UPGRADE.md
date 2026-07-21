# Nixfied Upgrade Notes

Nixfied uses an exact runtime ABI and toolchain contract. MFM updates the
authored model and the exact flake pin as one coordinated cut; mixed contracts,
compatibility shims, and new-runtime state migrations are unsupported.

## Coordinated upgrade

1. Start from a clean worktree and record the current `flake.lock` Nixfied
   revision.
2. Before repinning, use that old exact runtime to stop live services:

   ```bash
   nix run .#down
   nix run .#ps
   ```

3. If disposable Nixfied runtime state should be removed, do it now with the
   old exact runtime:

   ```bash
   nix run .#clean
   ```

   Use an explicit purge only when protected or persistent state is also
   intentionally disposable. Cleanup remains marker-, lease-, process-, and
   path-safety gated.
4. Update `nixfied.nix` for the new contract and update the exact Nixfied pin in
   `flake.lock` in the same change.
5. Check the generated surface and model, then run the complete gate once on
   the final upgrade revision:

   ```bash
   system=$(nix eval --raw --impure --expr 'builtins.currentSystem')
   nix flake check --no-build
   nix eval --json ".#apps.${system}" --apply builtins.attrNames
   nix run .#model-check
   nix run .#ci
   nix run .#ps
   ```

   `.#ci` composes `.#check`, `.#test`, and `.#test-db`; invoking those three
   first on the same revision repeats their work in separate runs. Use an
   individual component gate only while iterating on or diagnosing that part
   of the upgrade.

If retained state is rejected after repinning, do not move registry files,
rewrite markers, or ask the new runtime to migrate them. Restore the old exact
pin for any required inspection or supported cleanup. This guide deliberately
defines no new-runtime adoption or migration procedure for abandoned state.

## Local state drift triage

If the compiled model is valid but the default state root fails before any
task starts, compare it with a fresh temporary state root before changing the
project model:

```bash
system=$(nix eval --raw --impure --expr 'builtins.currentSystem')
nix eval --json ".#apps.${system}" --apply builtins.attrNames
nix run .#model-check
tmpdir=$(mktemp -d)
NIXFIED_STATE_DIR="$tmpdir" nix run .#check
NIXFIED_STATE_DIR="$tmpdir" nix run .#clean
```

If the fresh root passes while the default root fails during registry or marker
admission, treat the failure as local runtime-state drift. Check for live owned
processes before cleanup:

```bash
nix run .#ps
pgrep -af 'nixfied-runtime|postgres|reth|mfm_cli|cargo'
```

Control commands may fail when the old registry itself cannot be opened. Do
not relocate registry files, rewrite markers, or delete state around matching
live processes. Restore the old exact flake pin for supported inspection and
cleanup; only abandon old state after deciding that all state under that root
is disposable.

## Cargo verification artifacts

An ABI upgrade does not migrate or require cleaning Cargo artifacts. Follow the
steady-state ownership and cleanup policy in
[`build-and-verification.md`](build-and-verification.md).

## Current local conventions

- [`nixfied.nix`](../nixfied.nix) is the project-owned model. It exports
  `check`, `test`, `test-db`, and `ci` and imports the upstream Postgres and
  Reth adapters.
- `.#ci` is full by definition and does not accept v1 `--mode`, `--full`, or
  `--summary` flags.
- MFM's deterministic service window starts at port `28080`, below common OS
  ephemeral ranges. The upstream endpoint contract coordinates starts across
  state roots and reports ownership-aware conflicts.
