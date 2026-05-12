# mfm-publish-docs

Standalone release tool for the docs.rs publish wave. This is not an MFM op/state runtime family:
its plan files are release-tool artifacts under `.mfm/publish-docs/`, not MFM `ExecutionPlan`
artifacts.

## Modes

- `plan`: build the selected wave plan without uploading crates.
- `sync-umbrella`: regenerate `crates/docs/README.md` from `crates/docs/catalog.toml`; use
  `--check` to report staleness without writing.
- `apply`: execute publishable actions. This runs `cargo publish --locked -p <package>` for each
  package the refreshed plan still marks publishable.
- `resume <run_id>`: replay the previous selection and continue real publish actions when they
  still plan as publishable.
- `yank <package>`: yank an explicitly allowed published version.

Omitting the subcommand defaults to `apply`, so `nix run .#publish-docs` can publish crates.

## Release Runbook

1. Start from a deliberate release branch with a reviewed publish wave and desired catalog.
2. Run `nix run .#publish-docs -- plan` and inspect the generated artifacts under
   `.mfm/publish-docs/runs/<run_id>/`.
3. Run `nix run .#publish-docs -- sync-umbrella --check` and commit README changes first if the
   check reports drift.
4. Configure crates.io credentials only for the release shell.
5. Run `nix run .#publish-docs -- apply` when the publish plan is intentional.
6. Use `nix run .#publish-docs -- resume <run_id>` only to continue the same reviewed selection.

Do not treat `apply`, the default command, or `resume` as dry runs.
