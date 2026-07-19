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
5. Check the generated surface and model, then run every repository gate:

   ```bash
   system=$(nix eval --raw --impure --expr 'builtins.currentSystem')
   nix eval --json ".#apps.${system}" --apply builtins.attrNames
   nix run .#model-check
   nix run .#check
   nix run .#test
   nix run .#test-db
   nix run .#ci
   ```

If retained state is rejected after repinning, do not move registry files,
rewrite markers, or ask the new runtime to migrate them. Restore the old exact
pin for any required inspection or supported cleanup. This guide deliberately
defines no new-runtime adoption or migration procedure for abandoned state.

## Cargo verification artifacts

The steady-state ownership and lifecycle contract is documented in
[`build-and-verification.md`](build-and-verification.md).

Broad gates use the project-owned `target/verification` directory. It is not
Nixfied state: `NIXFIED_STATE_DIR` neither relocates nor cleans it, and a
Nixfied ABI upgrade does not migrate it.

No cleanup is normally required. For a deliberate cold rebuild or suspected
Cargo artifact corruption, use the project-owned operation:

```bash
cargo clean --target-dir target/verification
```

Direct development remains in the ordinary worktree target because
`nix develop` and `.#quick` unset `CARGO_TARGET_DIR`.

## Current local conventions

- [`nixfied.nix`](../nixfied.nix) is the project-owned model. It exports
  `check`, `test`, `test-db`, and `ci` and imports the upstream Postgres
  adapter.
- `.#ci` is full by definition and does not accept v1 `--mode`, `--full`, or
  `--summary` flags.
- MFM's deterministic service window starts at port `28080`, below common OS
  ephemeral ranges. The upstream endpoint contract coordinates starts across
  state roots and reports ownership-aware conflicts.
