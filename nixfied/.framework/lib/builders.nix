# Nix builders - mkAppScript, mkApp, mkAppWithDeps, withTiming
{
  pkgs,
  project,
  shellContract,
  fixtureLib,
  loadEnv,
  loadEnvFile ? null,
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
    if [ $_TIMING_DURATION -lt 60 ]; then
      echo "TIMING: ${name} duration=''${_TIMING_DURATION}s"
    else
      _TIMING_MINS=$((_TIMING_DURATION / 60))
      _TIMING_SECS=$((_TIMING_DURATION % 60))
      echo "TIMING: ${name} duration=''${_TIMING_MINS}m''${_TIMING_SECS}s"
    fi

    exit $_TIMING_EXIT
  '';

  # Helper to create app scripts
  mkAppScript =
    {
      name,
      script,
      fixtures ? null,
      env ? { },
      useDeps ? false,
      fixtureProfile ? "default",
      appContract ? null,
    }:
    let
      envExports = concatMapStringsSep "\n" (key: "export ${key}=${toString env.${key}}") (
        builtins.attrNames env
      );
      depsScript = project.install.deps or "";
      depsBlock = if useDeps && depsScript != "" then depsScript else "";
      pathBlock = if runtimePath != "" then "export PATH=\"${runtimePath}:$PATH\"" else "";
      script0 = fixtureLib.wrapScript {
        contextName = name;
        inherit fixtures;
        defaultProfile = fixtureProfile;
        defaultLogs = true;
        script = script;
      };
      contractFile =
        if appContract == null then
          null
        else
          pkgs.writeText "${name}-app-contract.json" (builtins.toJSON appContract);
      contractPrelude =
        if appContract == null then
          ""
        else
          ''
            NIXFIED_APP_CONTRACT_FILE="${toString contractFile}"
            source ${toString shellContract.runtime}
            nixfied_contract_validate_env "$NIXFIED_APP_CONTRACT_FILE"
            nixfied_contract_validate_args "$NIXFIED_APP_CONTRACT_FILE" "$@"
          '';
      contractExitCheck =
        if appContract == null then
          ""
        else
          ''
            _NIXFIED_CONTRACT_RC=0
            nixfied_contract_validate_exit "$NIXFIED_APP_CONTRACT_FILE" "$_NIXFIED_APP_RC" || _NIXFIED_CONTRACT_RC=$?
            if [ "$_NIXFIED_CONTRACT_RC" -ne 0 ]; then
              exit "$_NIXFIED_CONTRACT_RC"
            fi
          '';
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
      ${contractPrelude}
      _NIXFIED_APP_RC=0
      (
        set -euo pipefail
        export NIXFIED_CLEANUP_OWNER_BASHPID="''${BASHPID:-}"
        ${script0}
      ) || _NIXFIED_APP_RC=$?
      ${contractExitCheck}
      exit "$_NIXFIED_APP_RC"
    '';

  mkApp =
    {
      name,
      script,
      fixtures ? null,
      env ? { },
      useDeps ? false,
      fixtureProfile ? "default",
      description ? null,
      meta ? { },
      api ? null,
    }:
    let
      scriptDrv = mkAppScript {
        inherit
          name
          script
          fixtures
          env
          useDeps
          fixtureProfile
          ;
        appContract = if api == null then null else (api.appContract or null);
      };
    in
    {
      type = "app";
      meta = pkgs.lib.recursiveUpdate (pkgs.lib.optionalAttrs (description != null) {
        inherit description;
      }) meta;
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
