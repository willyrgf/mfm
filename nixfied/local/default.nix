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
  ...
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
  apps =
    let
      aaveTools = import ../project/aave-origin-tools.nix { inherit pkgs; };
    in
    {
      aave-v3-origin-fetch = {
        type = "app";
        program = "${aaveTools.aaveV3OriginFetchTool}/bin/mfm-aave-v3-origin-fetch";
      };
      aave-v3-origin-compile = {
        type = "app";
        program = "${aaveTools.aaveV3OriginCompileTool}/bin/mfm-aave-v3-origin-compile";
      };
      aave-v3-origin-deploy = {
        type = "app";
        program = "${aaveTools.aaveV3OriginDeployTool}/bin/mfm-aave-v3-origin-deploy";
      };
    };

  # Extra flake packages (merged into `packages` output).
  packages = { };

  # Optional: extend dev shells (merged into `devShells` output).
  devShells = { };
}
