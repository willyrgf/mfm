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
  nginxCfg = services.nginx;
  minioCfg = services.minio;
  rethCfg = services.reth;
  heliosCfg = services.helios;

  postgresEnabled = postgresCfg.enable;
  nginxEnabled = nginxCfg.enable;
  minioEnabled = minioCfg.enable;
  rethEnabled = rethCfg.enable;
  heliosEnabled = heliosCfg.enable;

  resolvePortBase =
    key:
    if builtins.hasAttr key runtime.ports then
      runtime.ports.${key}
    else
      throw "nixfied.operations: port key '${key}' is not defined in nixfied.runtime.ports";

  postgresPortBase = if postgresEnabled then resolvePortBase postgresCfg.portKey else 0;
  nginxHttpPortBase = if nginxEnabled then resolvePortBase nginxCfg.portKeyHttp else 0;
  minioApiPortBase = if minioEnabled then resolvePortBase minioCfg.portKeyApi else 0;
  minioConsolePortBase = if minioEnabled then resolvePortBase minioCfg.portKeyConsole else 0;
  rethHttpPortBase = if rethEnabled then resolvePortBase rethCfg.portKeyHttp else 0;
  rethWsPortBase = if rethEnabled then resolvePortBase rethCfg.portKeyWs else 0;
  rethAuthPortBase = if rethEnabled then resolvePortBase rethCfg.portKeyAuth else 0;
  heliosRpcPortBase = if heliosEnabled then resolvePortBase heliosCfg.portKeyRpc else 0;

  netcatPkg =
    if pkgs ? netcat then
      pkgs.netcat
    else if pkgs ? netcat-openbsd then
      pkgs.netcat-openbsd
    else
      throw "nixfied.operations: netcat package is required for readiness probes";

  postgresProbePkg = if pkgs ? postgresql_16 then pkgs.postgresql_16 else pkgs.postgresql;
  postgresRuntimePath =
    if postgresCfg.package != null then postgresCfg.package else builtins.toString postgresProbePkg;
  minioRuntimePath = if minioCfg.package != null then minioCfg.package else builtins.toString pkgs.minio;
  minioClientRuntimePath =
    if minioCfg.clientPackage != null then minioCfg.clientPackage else builtins.toString pkgs.minio-client;
  rethRuntimePath = if rethCfg.package != null then rethCfg.package else builtins.toString pkgs.reth;
  rethModeFlags =
    if rethCfg.devMode or false then
      [ "--dev" ]
    else if (rethCfg.network or "") != "" then
      [
        "--chain"
        (rethCfg.network or "local")
      ]
    else
      [ ];
  rethModeArgLines = builtins.concatStringsSep "\n" (
    map (arg: "      reth_cmd+=(${lib.escapeShellArg arg})") rethModeFlags
  );
  rethExtraArgLines = builtins.concatStringsSep "\n" (
    map (arg: "      reth_cmd+=(${lib.escapeShellArg arg})") (rethCfg.extraArgs or [ ])
  );
  minioRootUserDefault = minioCfg.rootUser or "minio";
  minioRootPasswordDefault = minioCfg.rootPassword or "minio123456";
  minioBrowserEnabled = minioCfg.browser or true;
  serviceProbeRuntimeInputs = [
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.curl
    netcatPkg
    postgresProbePkg
  ];
  serviceLifecycleRuntimeInputs = [
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.curl
    postgresProbePkg
    pkgs.minio
    pkgs.minio-client
    pkgs.reth
  ];

  envNames = runtime.env.names;
  envPattern =
    if envNames == [ ] then runtime.env.default else builtins.concatStringsSep "|" envNames;
  isolationSlotVar = runtime.slot.var;
  isolationEnvVar = runtime.env.var;
  isolationMaxSlot = runtime.slot.max;
  isolationSlotsJson = builtins.toJSON cfg.testIsolation.slots;
  isolationEnvsJson = builtins.toJSON cfg.testIsolation.envs;
  isolationRunArgsJson = builtins.toJSON cfg.testIsolation.runArgs;
  isolationRunEnvJson = builtins.toJSON cfg.testIsolation.runEnv;

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
      kind ? "utility",
      runtimeInputs ? [ ],
      passThroughEnv ? [
        "HOME"
        runtime.env.var
        runtime.slot.var
      ],
      env ? { },
      effects ? [ "none" ],
      idempotent ? true,
      exposeApp ? true,
      usage ? [ "nix run .#${appName}" ],
      category ? "ops",
    }:
    {
      inherit
        id
        kind
        summary
        description
        ;
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
          inherit idempotent effects;
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
        inherit
          passThroughEnv
          env
          ;
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
        expose = exposeApp;
        name = appName;
        category = category;
        inherit usage;
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

  servicesStartScript = ''
    set -euo pipefail
    ${slotEnvPrelude}

    artifacts_dir="''${CI_ARTIFACTS_DIR:-/tmp/ci-artifacts}"
    mkdir -p "$artifacts_dir"

    services_root="$artifacts_dir/services"
    mkdir -p "$services_root"

    started=0

    if [ ${if postgresEnabled then "1" else "0"} -eq 1 ]; then
      started=$((started + 1))
      postgres_port=$(( ${toString postgresPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      postgres_root="$services_root/postgres"
      postgres_data="$postgres_root/data"
      postgres_log="$artifacts_dir/postgres-service.log"
      postgres_db=${lib.escapeShellArg (postgresCfg.database or "app")}
      postgres_test_db=${lib.escapeShellArg (postgresCfg.testDatabase or "app_test")}
      mkdir -p "$postgres_data"

      if ! ${postgresRuntimePath}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$postgres_port" -q 2>/dev/null; then
        if [ ! -f "$postgres_data/PG_VERSION" ]; then
          ${postgresRuntimePath}/bin/initdb -D "$postgres_data" -U postgres --no-locale --encoding=UTF8 -A trust >/dev/null
          printf '%s\n' \
            '# TYPE  DATABASE        USER  ADDRESS       METHOD' \
            'local   all             all                 trust' \
            'host    all             all   127.0.0.1/32  trust' \
            'host    all             all   ::1/128       trust' \
            > "$postgres_data/pg_hba.conf"
        fi

        if [ -f "$postgres_data/postmaster.pid" ]; then
          stale_pid="$(head -1 "$postgres_data/postmaster.pid" 2>/dev/null || true)"
          if [ -n "$stale_pid" ] && ! kill -0 "$stale_pid" 2>/dev/null; then
            rm -f "$postgres_data/postmaster.pid"
          fi
        fi

        ${postgresRuntimePath}/bin/pg_ctl -D "$postgres_data" -l "$postgres_log" -o "-p $postgres_port -h 127.0.0.1" start
      fi

      for _ in $(seq 1 120); do
        if ${postgresRuntimePath}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$postgres_port" -q 2>/dev/null; then
          break
        fi
        sleep 0.25
      done

      if ! ${postgresRuntimePath}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$postgres_port" -q 2>/dev/null; then
        echo "ERROR: postgres failed to become ready port=$postgres_port"
        exit 1
      fi

      ${postgresRuntimePath}/bin/createdb -h 127.0.0.1 -p "$postgres_port" -U postgres "$postgres_db" >/dev/null 2>&1 || true
      if [ "$postgres_test_db" != "$postgres_db" ]; then
        ${postgresRuntimePath}/bin/createdb -h 127.0.0.1 -p "$postgres_port" -U postgres "$postgres_test_db" >/dev/null 2>&1 || true
      fi
      echo "OK: postgres ready port=$postgres_port"
    else
      echo "SKIP: postgres lifecycle disabled"
    fi

    if [ ${if minioEnabled then "1" else "0"} -eq 1 ]; then
      started=$((started + 1))
      minio_api_port=$(( ${toString minioApiPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      minio_console_port=$(( ${toString minioConsolePortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      minio_root="$services_root/minio"
      minio_data="$minio_root/data"
      minio_log="$artifacts_dir/minio-service.log"
      minio_root_user="''${AWS_ACCESS_KEY_ID:-}"
      minio_root_password="''${AWS_SECRET_ACCESS_KEY:-}"
      minio_bucket="''${MFM_S3_BUCKET:-mfm-test}"
      mkdir -p "$minio_data"

      if [ -z "$minio_root_user" ]; then
        minio_root_user=${lib.escapeShellArg minioRootUserDefault}
      fi
      if [ -z "$minio_root_password" ]; then
        minio_root_password=${lib.escapeShellArg minioRootPasswordDefault}
      fi

      if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$minio_api_port/minio/health/ready" >/dev/null 2>&1; then
        if [ ${if minioBrowserEnabled then "1" else "0"} -eq 0 ]; then
          export MINIO_BROWSER=off
        fi
        MINIO_ROOT_USER="$minio_root_user" \
        MINIO_ROOT_PASSWORD="$minio_root_password" \
        ${minioRuntimePath}/bin/minio server "$minio_data" \
          --address "127.0.0.1:$minio_api_port" \
          --console-address "127.0.0.1:$minio_console_port" \
          >"$minio_log" 2>&1 &
        echo "$!" > "$minio_root/minio.pid"
      fi

      for _ in $(seq 1 120); do
        if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$minio_api_port/minio/health/ready" >/dev/null 2>&1; then
          break
        fi
        sleep 0.25
      done

      if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$minio_api_port/minio/health/ready" >/dev/null 2>&1; then
        echo "ERROR: minio failed to become ready port=$minio_api_port"
        if [ -f "$minio_log" ]; then
          tail -50 "$minio_log" >&2 || true
        fi
        exit 1
      fi

      ${minioClientRuntimePath}/bin/mc alias set ci "http://127.0.0.1:$minio_api_port" "$minio_root_user" "$minio_root_password" >/dev/null
      ${minioClientRuntimePath}/bin/mc mb --ignore-existing "ci/$minio_bucket" >/dev/null
      echo "OK: minio ready api=$minio_api_port console=$minio_console_port bucket=$minio_bucket"
    else
      echo "SKIP: minio lifecycle disabled"
    fi

    if [ ${if rethEnabled then "1" else "0"} -eq 1 ]; then
      started=$((started + 1))
      reth_http_port=$(( ${toString rethHttpPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      reth_ws_port=$(( ${toString rethWsPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      reth_auth_port=$(( ${toString rethAuthPortBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
      reth_root="$services_root/reth"
      reth_data="$reth_root/data"
      reth_run="$reth_root/run"
      reth_log="$artifacts_dir/reth-service.log"
      mkdir -p "$reth_data" "$reth_run"

      reth_jwt="$reth_root/jwt.hex"
      if [ ! -f "$reth_jwt" ]; then
        printf '%064x\n' 0 > "$reth_jwt"
      fi
      chmod 600 "$reth_jwt" 2>/dev/null || true

      if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 \
        -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
        "http://127.0.0.1:$reth_http_port" \
        | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
        reth_cmd=(
          ${rethRuntimePath}/bin/reth
          node
        )
${rethModeArgLines}
        reth_cmd+=(
          --datadir "$reth_data"
          --ipcpath "$reth_run/reth.ipc"
          --http
          --http.addr 127.0.0.1
          --http.port "$reth_http_port"
          --ws
          --ws.addr 127.0.0.1
          --ws.port "$reth_ws_port"
          --authrpc.addr 127.0.0.1
          --authrpc.port "$reth_auth_port"
          --authrpc.jwtsecret "$reth_jwt"
        )
${rethExtraArgLines}
        "''${reth_cmd[@]}" >"$reth_log" 2>&1 &
        echo "$!" > "$reth_root/reth.pid"
      fi

      for _ in $(seq 1 160); do
        if ${pkgs.curl}/bin/curl -fsS --max-time 2 \
          -H 'content-type: application/json' \
          --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
          "http://127.0.0.1:$reth_http_port" \
          | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
          break
        fi
        sleep 0.25
      done

      if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 \
        -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
        "http://127.0.0.1:$reth_http_port" \
        | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
        echo "ERROR: reth failed to become ready port=$reth_http_port"
        if [ -f "$reth_log" ]; then
          tail -50 "$reth_log" >&2 || true
        fi
        exit 1
      fi
      echo "OK: reth ready http=$reth_http_port ws=$reth_ws_port auth=$reth_auth_port"
    else
      echo "SKIP: reth lifecycle disabled"
    fi

    if [ "$started" -eq 0 ]; then
      echo "SKIP: no enabled services for lifecycle start"
      exit 0
    fi

    echo "OK: services started count=$started"
  '';

  servicesStopScript = ''
    set -euo pipefail

    artifacts_dir="''${CI_ARTIFACTS_DIR:-/tmp/ci-artifacts}"
    services_root="$artifacts_dir/services"

    stopped=0

    if [ ${if rethEnabled then "1" else "0"} -eq 1 ]; then
      reth_pid_file="$services_root/reth/reth.pid"
      if [ -f "$reth_pid_file" ]; then
        reth_pid="$(cat "$reth_pid_file" 2>/dev/null || true)"
        if [ -n "$reth_pid" ] && kill -0 "$reth_pid" 2>/dev/null; then
          kill "$reth_pid" 2>/dev/null || true
          for _ in $(seq 1 40); do
            if ! kill -0 "$reth_pid" 2>/dev/null; then
              break
            fi
            sleep 0.25
          done
          kill -KILL "$reth_pid" 2>/dev/null || true
        fi
        rm -f "$reth_pid_file"
      fi
      stopped=$((stopped + 1))
      echo "OK: reth stopped"
    else
      echo "SKIP: reth lifecycle disabled"
    fi

    if [ ${if minioEnabled then "1" else "0"} -eq 1 ]; then
      minio_pid_file="$services_root/minio/minio.pid"
      if [ -f "$minio_pid_file" ]; then
        minio_pid="$(cat "$minio_pid_file" 2>/dev/null || true)"
        if [ -n "$minio_pid" ] && kill -0 "$minio_pid" 2>/dev/null; then
          kill "$minio_pid" 2>/dev/null || true
          for _ in $(seq 1 40); do
            if ! kill -0 "$minio_pid" 2>/dev/null; then
              break
            fi
            sleep 0.25
          done
          kill -KILL "$minio_pid" 2>/dev/null || true
        fi
        rm -f "$minio_pid_file"
      fi
      stopped=$((stopped + 1))
      echo "OK: minio stopped"
    else
      echo "SKIP: minio lifecycle disabled"
    fi

    if [ ${if postgresEnabled then "1" else "0"} -eq 1 ]; then
      postgres_data="$services_root/postgres/data"
      if [ -f "$postgres_data/postmaster.pid" ]; then
        ${postgresRuntimePath}/bin/pg_ctl -D "$postgres_data" stop -m fast >/dev/null 2>&1 || true
      fi
      stopped=$((stopped + 1))
      echo "OK: postgres stopped"
    else
      echo "SKIP: postgres lifecycle disabled"
    fi

    if [ "$stopped" -eq 0 ]; then
      echo "SKIP: no enabled services for lifecycle stop"
      exit 0
    fi

    echo "OK: services stopped count=$stopped"
  '';

  isolationScript = ''
    set -euo pipefail
    slot_var=${lib.escapeShellArg isolationSlotVar}
    env_var=${lib.escapeShellArg isolationEnvVar}
    slot_max=${toString isolationMaxSlot}
    max_parallel=${toString cfg.testIsolation.maxParallel}
    logs_root=${lib.escapeShellArg cfg.testIsolation.logsDir}
    run_app=${lib.escapeShellArg cfg.testIsolation.runApp}
    validate_app=${lib.escapeShellArg cfg.testIsolation.validateApp}
    keep_logs_success=${if cfg.testIsolation.keepLogsOnSuccess then "1" else "0"}
    keep_logs_failure=${if cfg.testIsolation.keepLogsOnFailure then "1" else "0"}

    slots_json='${isolationSlotsJson}'
    envs_json='${isolationEnvsJson}'
    run_args_json='${isolationRunArgsJson}'
    run_env_json='${isolationRunEnvJson}'
    project_root="$(pwd -P)"

    if [ -z "$run_app" ]; then
      echo "ERROR: test-isolation runApp is empty"
      exit 3
    fi

    if [ -z "$validate_app" ]; then
      echo "ERROR: test-isolation validateApp is empty"
      exit 3
    fi

    if ! [[ "$max_parallel" =~ ^[0-9]+$ ]]; then
      echo "ERROR: test-isolation maxParallel is not an integer: $max_parallel"
      exit 3
    fi

    if [ "$max_parallel" -lt 1 ]; then
      echo "ERROR: test-isolation maxParallel must be >= 1"
      exit 3
    fi

    mapfile -t isolation_slots < <(${pkgs.jq}/bin/jq -r '.[]' <<<"$slots_json")
    mapfile -t isolation_envs < <(${pkgs.jq}/bin/jq -r '.[]' <<<"$envs_json")
    mapfile -t run_args < <(${pkgs.jq}/bin/jq -r '.[]' <<<"$run_args_json")
    mapfile -t run_env_entries < <(${pkgs.jq}/bin/jq -r 'to_entries[]? | [.key, (.value | tostring)] | @tsv' <<<"$run_env_json")

    if [ "''${#isolation_slots[@]}" -eq 0 ]; then
      echo "ERROR: test-isolation matrix has no slots"
      exit 3
    fi

    if [ "''${#isolation_envs[@]}" -eq 0 ]; then
      echo "ERROR: test-isolation matrix has no environments"
      exit 3
    fi

    mkdir -p "$logs_root"
    echo "INFO: test-isolation matrix slots=''${#isolation_slots[@]} envs=''${#isolation_envs[@]}"

    statuses_dir="$(mktemp -d "$logs_root/.status.XXXXXX")"
    semaphore_dir="$(mktemp -d "$logs_root/.semaphore.XXXXXX")"
    semaphore_fifo="$semaphore_dir/tokens.fifo"
    mkfifo "$semaphore_fifo"
    exec 9<>"$semaphore_fifo"
    rm -f "$semaphore_fifo"

    token_count=0
    while [ "$token_count" -lt "$max_parallel" ]; do
      printf 'token\n' >&9
      token_count=$((token_count + 1))
    done

    total=0
    failed=0
    worker_pids=()
    status_files=()

    for slot_value in "''${isolation_slots[@]}"; do
      if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
        echo "ERROR: matrix slot is not an integer: $slot_value"
        failed=$((failed + 1))
        continue
      fi
      if [ "$slot_value" -gt "$slot_max" ]; then
        echo "ERROR: matrix slot exceeds max slot ($slot_max): $slot_value"
        failed=$((failed + 1))
        continue
      fi

      for env_value in "''${isolation_envs[@]}"; do
        total=$((total + 1))

        case "$env_value" in
          ${envPattern})
            ;;
          *)
            echo "ERROR: unsupported environment in matrix: $env_value"
            failed=$((failed + 1))
            continue
            ;;
        esac

        cell_name="slot-''${slot_value}__env-''${env_value}"
        cell_dir="$logs_root/$cell_name"
        artifacts_dir="$cell_dir/artifacts"
        validate_log="$cell_dir/validate.log"
        run_log="$cell_dir/run.log"
        status_file="$statuses_dir/$cell_name.rc"

        mkdir -p "$cell_dir" "$artifacts_dir"
        echo "INFO: isolation cell start slot=$slot_value env=$env_value"
        status_files+=("$status_file")

        IFS= read -r -u 9 _
        (
          set +e
          rc=1
          export "$slot_var=$slot_value"
          export "$env_var=$env_value"
          export CI_ARTIFACTS_DIR="$artifacts_dir"

          for run_env_entry in "''${run_env_entries[@]}"; do
            run_env_key="''${run_env_entry%%$'\t'*}"
            run_env_value="''${run_env_entry#*$'\t'}"
            export "$run_env_key=$run_env_value"
          done

          nix run "path:$project_root"#"$validate_app" > "$validate_log" 2>&1
          rc="$?"
          if [ "$rc" -eq 0 ]; then
            nix run "path:$project_root"#"$run_app" -- "''${run_args[@]}" > "$run_log" 2>&1
            rc="$?"
          fi

          printf '%s\n' "$rc" > "$status_file"
          if [ "$rc" -eq 0 ]; then
            echo "OK: isolation cell passed slot=$slot_value env=$env_value"
            if [ "$keep_logs_success" -eq 0 ]; then
              rm -rf "$cell_dir"
            fi
          else
            echo "ERROR: isolation cell failed slot=$slot_value env=$env_value rc=$rc"
          fi

          printf 'token\n' >&9
          exit 0
        ) &
        worker_pids+=("$!")
      done
    done

    for worker_pid in "''${worker_pids[@]}"; do
      wait "$worker_pid" || true
    done

    exec 9>&-
    exec 9<&-
    rm -rf "$semaphore_dir"

    for status_file in "''${status_files[@]}"; do
      if [ ! -f "$status_file" ]; then
        failed=$((failed + 1))
        echo "ERROR: isolation cell status missing file=$status_file"
        continue
      fi

      rc="$(cat "$status_file")"
      if [ "$rc" != "0" ]; then
        failed=$((failed + 1))
      fi
    done
    rm -rf "$statuses_dir"

    if [ "$total" -eq 0 ]; then
      echo "ERROR: test-isolation matrix did not execute any cells"
      exit 3
    fi

    if [ "$failed" -ne 0 ]; then
      echo "ERROR: test-isolation completed with failures failed=$failed total=$total"
      if [ "$keep_logs_failure" -eq 0 ]; then
        rm -rf "$logs_root"
      fi
      exit 1
    fi

    if [ "$keep_logs_success" -eq 1 ]; then
      echo "INFO: isolation logs preserved at $logs_root"
    fi
    echo "OK: test-isolation completed total=$total"
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

    testIsolation.slots = lib.mkOption {
      type = lib.types.listOf lib.types.int;
      default = [ runtime.slot.default ];
    };

    testIsolation.envs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = runtime.env.names;
    };

    testIsolation.logsDir = lib.mkOption {
      type = lib.types.str;
      default = "/tmp/${config.nixfied.identity.projectId}-isolation";
    };

    testIsolation.keepLogsOnSuccess = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };

    testIsolation.keepLogsOnFailure = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    testIsolation.maxParallel = lib.mkOption {
      type = lib.types.ints.positive;
      default = 4;
    };

    testIsolation.runApp = lib.mkOption {
      type = lib.types.str;
      default = "ci";
    };

    testIsolation.runArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "--summary" ];
    };

    testIsolation.validateApp = lib.mkOption {
      type = lib.types.str;
      default = "validate-env";
    };

    testIsolation.runEnv = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.oneOf [
          lib.types.str
          lib.types.int
          lib.types.bool
        ]
      );
      default = { };
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

    servicesLifecycle.enable = lib.mkOption {
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
        runtimeInputs = [
          pkgs.coreutils
          pkgs.jq
          pkgs.nix
        ];
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
          postgres, nginx, reth, and helios.
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
          postgres, nginx, reth, and helios.
        '';
        command = readyScript;
        runtimeInputs = serviceProbeRuntimeInputs;
      };
    })

    (lib.mkIf (cfg.enable && cfg.servicesLifecycle.enable) {
      nixfied.tasks."services-start" = mkTask {
        id = "task.ops.services-start";
        appName = "services-start";
        kind = "service-op";
        summary = "Start modeled local services";
        description = ''
          Starts enabled local services (postgres, minio, reth) using typed nixfied.services configuration.
        '';
        command = servicesStartScript;
        runtimeInputs = serviceLifecycleRuntimeInputs;
        passThroughEnv = [
          "HOME"
          runtime.env.var
          runtime.slot.var
          "CI_ARTIFACTS_DIR"
          "AWS_ACCESS_KEY_ID"
          "AWS_SECRET_ACCESS_KEY"
          "MFM_S3_BUCKET"
        ];
        effects = [
          "starts-daemon"
          "writes-state"
        ];
        idempotent = true;
      };

      nixfied.tasks."services-stop" = mkTask {
        id = "task.ops.services-stop";
        appName = "services-stop";
        kind = "service-op";
        summary = "Stop modeled local services";
        description = ''
          Stops enabled local services (postgres, minio, reth) that were started by task.ops.services-start.
        '';
        command = servicesStopScript;
        runtimeInputs = serviceLifecycleRuntimeInputs;
        passThroughEnv = [
          "HOME"
          runtime.env.var
          runtime.slot.var
          "CI_ARTIFACTS_DIR"
        ];
        effects = [ "writes-state" ];
        idempotent = true;
      };
    })
  ];
}
