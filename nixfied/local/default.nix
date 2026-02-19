{
  pkgs,
  project,
  lib,
  slots ? null,
  hooks ? null,
  postgres ? null,
  nginx ? null,
  minio ? null,
  reth ? null,
  helios ? null,
  supervisor ? null,
  ephemeral ? null,
  serviceApis ? { },
}:

{
  # User-owned extension point.
  #
  # This directory is intended for customization that should survive framework
  # upgrades. Prefer nixfied/project/ for normal command/config wiring, and use
  # nixfied/local/ for extra apps/packages that shouldn't live in the framework.
  #
  # Notes:
  # - Apps must satisfy the Nixfied app API contract (meta.nixfied.api).
  # - Use `lib.appApi.mkNixfiedApp { ... }` to build compliant apps.
  apps = { };

  # Extra flake packages (merged into `packages` output).
  packages = { };

  # Optional: extend dev shells (merged into `devShells` output).
  devShells = { };
}
