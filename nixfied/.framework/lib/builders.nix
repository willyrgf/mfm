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
  logging = project.logging or { };
  defaultLogLevel = toString (logging.level or "info");
  defaultOutputMode = toString (logging.output or "stdout");

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
      log_timing "${name} duration=''${_TIMING_DURATION}s"
    else
      _TIMING_MINS=$((_TIMING_DURATION / 60))
      _TIMING_SECS=$((_TIMING_DURATION % 60))
      log_timing "${name} duration=''${_TIMING_MINS}m''${_TIMING_SECS}s"
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
      source ${toString shellContract.runtime}
      ${hookExports}
      ${envExports}
      nixfied_contract_resolve_runtime_primitives "${defaultLogLevel}" "${defaultOutputMode}"
      export NIXFIED_LOG_TRACE="''${NIXFIED_LOG_TRACE:-0}"
      export COMMAND_NAME="''${COMMAND_NAME:-${name}}"
      if [ "''${OUTPUT_MODE}" != "stdout" ]; then
        if [ -z "''${NIXFIED_LOG_FILE:-}" ]; then
          _nixfied_slot="''${NIX_ENV:-0}"
          _nixfied_env="''${PROJECT_ENV:-default}"
          _nixfied_cmd="''${COMMAND_NAME:-app}"
          _nixfied_ts="$(date +%Y%m%dT%H%M%S)"
          NIXFIED_LOG_FILE="''${LOG_DIR:-/tmp}/nixfied-''${_nixfied_cmd}-''${_nixfied_env}-slot''${_nixfied_slot}-''${_nixfied_ts}-$$.log"
        fi
        export NIXFIED_LOG_FILE
        mkdir -p "$(dirname "$NIXFIED_LOG_FILE")" 2>/dev/null || true
      fi
      ${depsBlock}
      ${contractPrelude}
      _NIXFIED_APP_RC=0
      (
        set -euo pipefail
        export COMMAND_NAME="''${COMMAND_NAME:-${name}}"
        export NIXFIED_CLEANUP_OWNER_BASHPID="''${BASHPID:-}"
        if [ "''${LOG_LEVEL:-}" = "trace" ] && [ "''${NIXFIED_LOG_TRACE:-0}" = "1" ]; then
          if [ -z "''${NIXFIED_XTRACE_FILE:-}" ]; then
            NIXFIED_XTRACE_FILE="''${NIXFIED_LOG_FILE:-''${LOG_DIR:-/tmp}/nixfied-trace-''${COMMAND_NAME:-app}-''${PROJECT_ENV:-default}-slot''${NIX_ENV:-0}-$$.log}"
            export NIXFIED_XTRACE_FILE
          fi
          mkdir -p "$(dirname "$NIXFIED_XTRACE_FILE")" 2>/dev/null || true
          exec 19>>"$NIXFIED_XTRACE_FILE"
          export BASH_XTRACEFD=19
          set -x
        fi
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
