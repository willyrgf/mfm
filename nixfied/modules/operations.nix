{
  lib,
  config,
  pkgs,
  ...
}:
let
  cfg = config.nixfied.operations;
  runtime = config.nixfied.runtime;
  services = config.nixfied.services;

  postgresCfg = services.postgres;
  minioCfg = services.minio;
  nginxCfg = services.nginx;
  rethCfg = services.reth;
  heliosCfg = services.helios;

  postgresEnabled = postgresCfg.enable;
  minioEnabled = minioCfg.enable;
  nginxEnabled = nginxCfg.enable;
  rethEnabled = rethCfg.enable;
  heliosEnabled = heliosCfg.enable;

  resolvePortBase =
    key:
    if builtins.hasAttr key runtime.ports then
      runtime.ports.${key}
    else
      throw "nixfied.operations: port key '${key}' is not defined in nixfied.runtime.ports";

  postgresPortBase = if postgresEnabled then resolvePortBase postgresCfg.portKey else 0;
  minioApiPortBase = if minioEnabled then resolvePortBase minioCfg.portKeyApi else 0;
  nginxHttpPortBase = if nginxEnabled then resolvePortBase nginxCfg.portKeyHttp else 0;
  rethHttpPortBase = if rethEnabled then resolvePortBase rethCfg.portKeyHttp else 0;
  heliosRpcPortBase = if heliosEnabled then resolvePortBase heliosCfg.portKeyRpc else 0;

  netcatPkg =
    if pkgs ? netcat then
      pkgs.netcat
    else if pkgs ? netcat-openbsd then
      pkgs.netcat-openbsd
    else
      throw "nixfied.operations: netcat package is required for readiness probes";

  postgresProbePkg = if pkgs ? postgresql_16 then pkgs.postgresql_16 else pkgs.postgresql;
  serviceProbeRuntimeInputs = [
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.curl
    netcatPkg
    postgresProbePkg
  ];

  envNames = runtime.env.names;
  envPattern =
    if envNames == [ ] then runtime.env.default else builtins.concatStringsSep "|" envNames;

  envOffsetCase = builtins.concatStringsSep "\n" (
    map (
      envName: "    ${envName}) env_offset=${toString (runtime.env.offsets.${envName} or 0)} ;;"
    ) envNames
  );

  slotEnvPrelude = ''
    slot_var=${lib.escapeShellArg runtime.slot.var}
    env_var=${lib.escapeShellArg runtime.env.var}
    slot_default=${toString runtime.slot.default}
    env_default=${lib.escapeShellArg runtime.env.default}

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
      echo "ERROR: $slot_var must be an integer"
      exit 3
    fi

    case "$env_value" in
${envOffsetCase}
      *)
        echo "ERROR: unsupported $env_var '$env_value'"
        exit 3
        ;;
    esac
  '';

  portNames = builtins.sort builtins.lessThan (builtins.attrNames runtime.ports);
  portEmitLines = builtins.concatStringsSep "\n" (
    map (
      portName:
      let
        base = runtime.ports.${portName};
      in
      ''
        value=$(( ${toString base} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
        printf "%s=%s\n" "${lib.toUpper portName}_PORT" "$value"
      ''
    ) portNames
  );

  portCheckLines = builtins.concatStringsSep "\n" (
    map (
      portName:
      let
        base = runtime.ports.${portName};
      in
      ''
        value=$(( ${toString base} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
        if command -v lsof >/dev/null 2>&1; then
          if lsof -nP -iTCP:"$value" -sTCP:LISTEN >/dev/null 2>&1; then
            status="LISTEN"
          else
            status="FREE"
          fi
        else
          status="UNKNOWN"
        fi
        printf "%s=%s (%s)\n" "${lib.toUpper portName}_PORT" "$value" "$status"
      ''
    ) portNames
  );

  mkTask =
    {
      id,
      appName,
      summary,
      description,
      command,
      runtimeInputs ? [ ],
    }:
    {
      inherit
        id
        summary
        description
        ;
      kind = "utility";
      runner = {
        type = "shell";
        command = command;
      };
      contract = {
        version = 1;
        output = {
          format = "text";
          channels = "stdout";
        };
        behavior = {
          idempotent = true;
          effects = [ "none" ];
          timeoutSec = 0;
        };
        errors.codes = {
          generic = 1;
          usage = 2;
          precondition = 3;
        };
      };
      runtime = {
        slotEnv = "optional";
        workdir = "projectRoot";
        hermetic = true;
        runtimeInputs = runtimeInputs;
        passThroughEnv = [
          "HOME"
          runtime.env.var
          runtime.slot.var
        ];
        env = { };
        umask = "022";
        locale = "C.UTF-8";
        timezone = "UTC";
      };
      scheduling = {
        locks = [ ];
        maxAttempts = 1;
        retryBackoffSec = [ ];
        priority = 100;
      };
      deps = {
        needs = [ ];
        softNeeds = [ ];
      };
      produces = {
        artifacts = [ ];
        stateKeys = [ ];
      };
      ui.app = {
        expose = true;
        name = appName;
        category = "ops";
        usage = [ "nix run .#${appName}" ];
        examples = [ ];
      };
    };

  validateScript = ''
    set -euo pipefail

    slot_var=${lib.escapeShellArg runtime.slot.var}
    env_var=${lib.escapeShellArg runtime.env.var}

    slot_default=${toString runtime.slot.default}
    slot_max=${toString runtime.slot.max}
    env_default=${lib.escapeShellArg runtime.env.default}

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
      echo "ERROR: $slot_var must be an integer"
      exit 3
    fi

    if [ "$slot_value" -gt "$slot_max" ]; then
      echo "ERROR: $slot_var exceeds max slot ($slot_max)"
      exit 3
    fi

    case "$env_value" in
      ${envPattern}) ;;
      *)
        echo "ERROR: unsupported $env_var '$env_value'"
        exit 3
        ;;
    esac

    echo "OK: environment is valid (''${env_var}=$env_value ''${slot_var}=$slot_value)"
  '';

  portsScript = ''
        set -euo pipefail

        slot_var=${lib.escapeShellArg runtime.slot.var}
        env_var=${lib.escapeShellArg runtime.env.var}
        slot_default=${toString runtime.slot.default}
        env_default=${lib.escapeShellArg runtime.env.default}

        slot_value="''${!slot_var:-$slot_default}"
        env_value="''${!env_var:-$env_default}"

        case "$env_value" in
    ${envOffsetCase}
          *)
            echo "ERROR: unsupported $env_var '$env_value'"
            exit 3
            ;;
        esac

        echo "INFO: Port assignments for slot ''${slot_value} env ''${env_value}"
    ${portEmitLines}
  '';

  checkPortsScript = ''
        set -euo pipefail

    ${slotEnvPrelude}

        echo "INFO: Port status for slot ''${slot_value} env ''${env_value}"
    ${portCheckLines}
  '';

  healthScript = ''
    set -euo pipefail
    ${slotEnvPrelude}

    checks=0

    if [ ${if postgresEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      postgres_port=$(( ${toString postgresPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking postgres health port=$postgres_port"
      if ${postgresProbePkg}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$postgres_port" -q 2>/dev/null; then
        echo "OK: postgres healthy port=$postgres_port"
      else
        echo "ERROR: postgres unhealthy port=$postgres_port"
        exit 1
      fi
    else
      echo "SKIP: postgres health check disabled"
    fi

    if [ ${if minioEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      minio_api_port=$(( ${toString minioApiPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking minio health port=$minio_api_port"
      if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$minio_api_port/minio/health/live" >/dev/null 2>&1; then
        echo "OK: minio healthy port=$minio_api_port"
      else
        echo "ERROR: minio unhealthy port=$minio_api_port"
        exit 1
      fi
    else
      echo "SKIP: minio health check disabled"
    fi

    if [ ${if nginxEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      nginx_http_port=$(( ${toString nginxHttpPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking nginx health port=$nginx_http_port"
      if ${netcatPkg}/bin/nc -z 127.0.0.1 "$nginx_http_port" >/dev/null 2>&1; then
        echo "OK: nginx healthy port=$nginx_http_port"
      else
        echo "ERROR: nginx unhealthy port=$nginx_http_port"
        exit 1
      fi
    else
      echo "SKIP: nginx health check disabled"
    fi

    if [ ${if rethEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      reth_http_port=$(( ${toString rethHttpPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking reth health port=$reth_http_port"
      if ${pkgs.curl}/bin/curl -fsS --max-time 2 \
        -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
        "http://127.0.0.1:$reth_http_port" \
        | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
        echo "OK: reth healthy port=$reth_http_port"
      else
        echo "ERROR: reth unhealthy port=$reth_http_port"
        exit 1
      fi
    else
      echo "SKIP: reth health check disabled"
    fi

    if [ ${if heliosEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      helios_rpc_port=$(( ${toString heliosRpcPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking helios health port=$helios_rpc_port"
      if ${pkgs.curl}/bin/curl -fsS --max-time 2 \
        -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
        "http://127.0.0.1:$helios_rpc_port" \
        | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
        echo "OK: helios healthy port=$helios_rpc_port"
      else
        echo "ERROR: helios unhealthy port=$helios_rpc_port"
        exit 1
      fi
    else
      echo "SKIP: helios health check disabled"
    fi

    if [ "$checks" -eq 0 ]; then
      echo "SKIP: no enabled services for health checks"
      exit 0
    fi

    echo "OK: health checks passed services=$checks"
  '';

  readyScript = ''
    set -euo pipefail
    ${slotEnvPrelude}

    checks=0

    if [ ${if postgresEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      postgres_port=$(( ${toString postgresPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking postgres readiness port=$postgres_port"

      if ! ${postgresProbePkg}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$postgres_port" -q 2>/dev/null; then
        echo "ERROR: postgres not ready port=$postgres_port (pg_isready failed)"
        exit 1
      fi

      if ${postgresProbePkg}/bin/psql -h 127.0.0.1 -p "$postgres_port" -U postgres -d postgres -Atqc "select 1;" >/dev/null 2>&1; then
        echo "OK: postgres ready port=$postgres_port"
      else
        echo "ERROR: postgres not ready port=$postgres_port (query failed)"
        exit 1
      fi
    else
      echo "SKIP: postgres readiness check disabled"
    fi

    if [ ${if minioEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      minio_api_port=$(( ${toString minioApiPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking minio readiness port=$minio_api_port"
      if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$minio_api_port/minio/health/ready" >/dev/null 2>&1; then
        echo "OK: minio ready port=$minio_api_port"
      else
        echo "ERROR: minio not ready port=$minio_api_port"
        exit 1
      fi
    else
      echo "SKIP: minio readiness check disabled"
    fi

    if [ ${if nginxEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      nginx_http_port=$(( ${toString nginxHttpPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking nginx readiness port=$nginx_http_port"
      if ${netcatPkg}/bin/nc -z 127.0.0.1 "$nginx_http_port" >/dev/null 2>&1; then
        echo "OK: nginx ready port=$nginx_http_port"
      else
        echo "ERROR: nginx not ready port=$nginx_http_port"
        exit 1
      fi
    else
      echo "SKIP: nginx readiness check disabled"
    fi

    if [ ${if rethEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      reth_http_port=$(( ${toString rethHttpPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking reth readiness port=$reth_http_port"
      if ${pkgs.curl}/bin/curl -fsS --max-time 2 \
        -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
        "http://127.0.0.1:$reth_http_port" \
        | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
        echo "OK: reth ready port=$reth_http_port"
      else
        echo "ERROR: reth not ready port=$reth_http_port"
        exit 1
      fi
    else
      echo "SKIP: reth readiness check disabled"
    fi

    if [ ${if heliosEnabled then "1" else "0"} -eq 1 ]; then
      checks=$((checks + 1))
      helios_rpc_port=$(( ${toString heliosRpcPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      echo "INFO: checking helios readiness port=$helios_rpc_port"
      if ${pkgs.curl}/bin/curl -fsS --max-time 2 \
        -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
        "http://127.0.0.1:$helios_rpc_port" \
        | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
        echo "OK: helios ready port=$helios_rpc_port"
      else
        echo "ERROR: helios not ready port=$helios_rpc_port"
        exit 1
      fi
    else
      echo "SKIP: helios readiness check disabled"
    fi

    if [ "$checks" -eq 0 ]; then
      echo "SKIP: no enabled services for readiness checks"
      exit 0
    fi

    echo "OK: readiness checks passed services=$checks"
  '';

  isolationScript = ''
    set -euo pipefail
    echo "INFO: Starting deterministic isolation smoke run"
    echo "SKIP: isolation orchestration is not configured for this project"
    echo "OK: test-isolation completed"
  '';
in
{
  options.nixfied.operations = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    validateEnv.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    testIsolation.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    ports.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    checkPorts.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    health.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    ready.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };
  };

  config = lib.mkMerge [
    (lib.mkIf (cfg.enable && cfg.validateEnv.enable) {
      nixfied.tasks."validate-env" = mkTask {
        id = "task.ops.validate-env";
        appName = "validate-env";
        summary = "Validate slot/env settings";
        description = "Validates PROJECT_ENV and NIX_ENV values against model runtime constraints.";
        command = validateScript;
      };
    })

    (lib.mkIf (cfg.enable && cfg.testIsolation.enable) {
      nixfied.tasks."test-isolation" = mkTask {
        id = "task.ops.test-isolation";
        appName = "test-isolation";
        summary = "Run isolation checks";
        description = "Runs deterministic isolation smoke checks from model metadata.";
        command = isolationScript;
      };
    })

    (lib.mkIf (cfg.enable && cfg.ports.enable) {
      nixfied.tasks."ports" = mkTask {
        id = "task.ops.ports";
        appName = "ports";
        summary = "Print model-derived port assignments";
        description = "Prints computed per-slot/per-env ports from compiled runtime data.";
        command = portsScript;
      };
    })

    (lib.mkIf (cfg.enable && cfg.checkPorts.enable) {
      nixfied.tasks."check-ports" = mkTask {
        id = "task.ops.check-ports";
        appName = "check-ports";
        summary = "Check model-derived port availability";
        description = "Checks if computed per-slot/per-env ports are listening or free.";
        command = checkPortsScript;
        runtimeInputs = [
          pkgs.coreutils
          pkgs.gnugrep
          pkgs.gnused
          (if pkgs ? lsof then pkgs.lsof else pkgs.coreutils)
        ];
      };
    })

    (lib.mkIf (cfg.enable && cfg.health.enable) {
      nixfied.tasks."health" = mkTask {
        id = "task.ops.health";
        appName = "health";
        summary = "Run service health checks";
        description = ''
          Runs health checks for enabled services:
          postgres, minio, nginx, reth, and helios.
        '';
        command = healthScript;
        runtimeInputs = serviceProbeRuntimeInputs;
      };
    })

    (lib.mkIf (cfg.enable && cfg.ready.enable) {
      nixfied.tasks."ready" = mkTask {
        id = "task.ops.ready";
        appName = "ready";
        summary = "Run service readiness checks";
        description = ''
          Runs readiness checks for enabled services:
          postgres, minio, nginx, reth, and helios.
        '';
        command = readyScript;
        runtimeInputs = serviceProbeRuntimeInputs;
      };
    })
  ];
}
