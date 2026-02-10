# Nix builders - mkAppScript, mkApp, mkAppWithDeps, withTiming
{
  pkgs,
  project,
  loadEnv,
  helpersScript,
  hookExports,
}:

let
  inherit (pkgs.lib) concatMapStringsSep makeBinPath;

  runtimePackages = project.tooling.runtimePackages or [ ];
  runtimePath = if runtimePackages == [ ] then "" else makeBinPath runtimePackages;

  # Timing wrapper - records and displays execution time
  withTiming = name: script: ''
    _TIMING_START=$(date +%s)
    _TIMING_EXIT=0

    (
      set -euo pipefail
      ${script}
    ) || _TIMING_EXIT=$?

    _TIMING_END=$(date +%s)
    _TIMING_DURATION=$((_TIMING_END - _TIMING_START))

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    if [ $_TIMING_DURATION -lt 60 ]; then
      echo "⏱️  ${name} completed in ''${_TIMING_DURATION}s"
    else
      _TIMING_MINS=$((_TIMING_DURATION / 60))
      _TIMING_SECS=$((_TIMING_DURATION % 60))
      echo "⏱️  ${name} completed in ''${_TIMING_MINS}m ''${_TIMING_SECS}s"
    fi
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    exit $_TIMING_EXIT
  '';

  # Helper to create app scripts
  mkAppScript =
    {
      name,
      script,
      env ? { },
      useDeps ? false,
    }:
    let
      envExports = concatMapStringsSep "\n" (key: "export ${key}=${toString env.${key}}") (
        builtins.attrNames env
      );
      depsScript = project.install.deps or "";
      depsBlock = if useDeps && depsScript != "" then depsScript else "";
      pathBlock = if runtimePath != "" then "export PATH=\"${runtimePath}:$PATH\"" else "";
    in
    pkgs.writeShellScript name ''
      set -euo pipefail
      ${pathBlock}
      export COMMAND_NAME="${name}"
      cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
      source ${loadEnv}
      source ${helpersScript}
      ${hookExports}
      ${envExports}
      ${depsBlock}
      ${script}
    '';

  mkApp =
    {
      name,
      script,
      env ? { },
      useDeps ? false,
      description ? null,
    }:
    let
      scriptDrv = mkAppScript {
        inherit
          name
          script
          env
          useDeps
          ;
      };
    in
    {
      type = "app";
      meta = pkgs.lib.optionalAttrs (description != null) {
        inherit description;
      };
      program = toString scriptDrv;
    };

  mkAppWithDeps = args: mkApp (args // { useDeps = true; });
in
{
  inherit
    withTiming
    mkAppScript
    mkApp
    mkAppWithDeps
    ;
}
