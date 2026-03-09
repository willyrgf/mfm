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
  probeCommands = import ../.framework/lib/probe-commands.nix { inherit pkgs; };
  probeRuntime = import ../.framework/lib/operations-probe-runtime.nix {
    inherit
      lib
      pkgs
      probeCommands
      resolveServicePortBase
      ;
    inherit postgresProbePkg;
    runtimeStride = runtime.slot.stride;
  };
  serviceConfigLib = import ../lib/service-config.nix { inherit lib; };
  testIsolationRuntime = import ../.framework/lib/test-isolation-runtime.nix { inherit lib pkgs; };

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

  serviceNames = [
    "postgres"
    "nginx"
    "minio"
    "reth"
    "helios"
  ];

  serviceEnabledByName = {
    postgres = postgresEnabled;
    nginx = nginxEnabled;
    minio = minioEnabled;
    reth = rethEnabled;
    helios = heliosEnabled;
  };

  serviceConfigByName = {
    postgres = postgresCfg;
    nginx = nginxCfg;
    minio = minioCfg;
    reth = rethCfg;
    helios = heliosCfg;
  };

  resolvedServiceConfigByName = builtins.mapAttrs (
    serviceName: serviceCfg:
    serviceConfigLib.normalizeServiceConfig {
      name = serviceName;
      config = serviceCfg;
    }
  ) serviceConfigByName;

  enabledServiceNames = builtins.filter (
    serviceName: serviceEnabledByName.${serviceName}
  ) serviceNames;

  resolvePortBase =
    key:
    if builtins.hasAttr key runtime.ports then
      runtime.ports.${key}
    else
      throw "nixfied.operations: port key '${key}' is not defined in nixfied.runtime.ports";

  resolveServicePortBase =
    serviceName: endpointName:
    resolvePortBase
      resolvedServiceConfigByName.${serviceName}.resolved.endpoints.${endpointName}.portKey;

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
    pkgs.jq
    netcatPkg
    postgresProbePkg
  ];

  envNames = runtime.env.names;
  envPattern =
    if envNames == [ ] then runtime.env.default else builtins.concatStringsSep "|" envNames;
  isolationEnvValues = if envNames == [ ] then [ runtime.env.default ] else envNames;
  isolationSlotVar = runtime.slot.var;
  isolationEnvVar = runtime.env.var;
  isolationMaxSlot = runtime.slot.max;
  isolationSlotsJson = builtins.toJSON cfg.testIsolation.slots;
  isolationEnvsJson = builtins.toJSON cfg.testIsolation.envs;
  isolationRunTaskId = cfg.testIsolation.runTaskId;
  isolationValidateTaskId = cfg.testIsolation.validateTaskId;
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

  serviceSelectionContractArgs = [
    {
      name = "service";
      kind = "option";
      long = "--service";
      type = "enum";
      values = serviceNames ++ [ "all" ];
      description = "Select one enabled service or 'all' (default).";
    }
    {
      name = "source";
      kind = "option";
      long = "--source";
      type = "string";
      description = "Override source key for selected service (requires --service).";
    }
  ];

  testIsolationContractArgs = [
    {
      name = "slot";
      kind = "option";
      long = "--slot";
      type = "int";
      description = "Run a single isolation slot (requires --env).";
    }
    {
      name = "env";
      kind = "option";
      long = "--env";
      type = "enum";
      values = isolationEnvValues;
      description = "Run a single isolation environment (requires --slot).";
    }
    {
      name = "max-parallel";
      kind = "option";
      long = "--max-parallel";
      type = "int";
      min = 1;
      description = "Override the isolation worker cap for this invocation.";
    }
  ];

  knownServiceCase = builtins.concatStringsSep "\n" (
    map (serviceName: "      ${serviceName}) return 0 ;;") serviceNames
  );

  serviceEnabledCase = builtins.concatStringsSep "\n" (
    map (
      serviceName:
      "      ${serviceName}) echo ${if serviceEnabledByName.${serviceName} then "1" else "0"} ;;"
    ) serviceNames
  );

  serviceDefaultSourceCase = builtins.concatStringsSep "\n" (
    map (
      serviceName:
      "      ${serviceName}) printf '%s' ${
              lib.escapeShellArg (resolvedServiceConfigByName.${serviceName}.defaultSource or "")
            } ;;"
    ) serviceNames
  );

  serviceHasSourceCase = builtins.concatStringsSep "\n" (
    map (
      serviceName:
      let
        sourceKeys = resolvedServiceConfigByName.${serviceName}.sourceKeys or [ ];
        sourceArgs = builtins.concatStringsSep " " (map lib.escapeShellArg sourceKeys);
      in
      ''
        ${serviceName})
          source_key_matches "$source"${if sourceArgs == "" then "" else " ${sourceArgs}"}
          return $?
          ;;
      ''
    ) serviceNames
  );

  enabledServiceArrayInit =
    if enabledServiceNames == [ ] then
      "selected_services=()"
    else
      "selected_services=("
      + builtins.concatStringsSep " " (map lib.escapeShellArg enabledServiceNames)
      + ")";

  serviceSelectionPrelude = ''
        target_service="all"
        target_source=""

        while [ "$#" -gt 0 ]; do
          case "$1" in
            --service)
              if [ "$#" -lt 2 ]; then
                echo "ERROR: --service requires a value"
                exit 2
              fi
              target_service="$2"
              shift 2
              ;;
            --service=*)
              target_service="''${1#--service=}"
              shift
              ;;
            --source)
              if [ "$#" -lt 2 ]; then
                echo "ERROR: --source requires a value"
                exit 2
              fi
              target_source="$2"
              shift 2
              ;;
            --source=*)
              target_source="''${1#--source=}"
              shift
              ;;
            --)
              shift
              break
              ;;
            *)
              echo "ERROR: unknown argument '$1'"
              exit 2
              ;;
          esac
        done

        if [ "$#" -gt 0 ]; then
          echo "ERROR: unexpected positional arguments: $*"
          exit 2
        fi

        is_known_service() {
          case "$1" in
    ${knownServiceCase}
            *) return 1 ;;
          esac
        }

        is_service_enabled() {
          case "$1" in
    ${serviceEnabledCase}
            *) echo "0" ;;
          esac
        }

        service_default_source() {
          case "$1" in
    ${serviceDefaultSourceCase}
            *) printf '%s' "" ;;
          esac
        }

        source_key_matches() {
          local wanted="$1"
          shift
          local candidate
          for candidate in "$@"; do
            if [ "$candidate" = "$wanted" ]; then
              return 0
            fi
          done
          return 1
        }

        source_kind_disallowed() {
          local source_kind="$1"
          shift
          local blocked_kind
          for blocked_kind in "$@"; do
            if [ "$blocked_kind" = "$source_kind" ]; then
              return 0
            fi
          done
          return 1
        }

        service_has_source() {
          local service="$1"
          local source="$2"
          case "$service" in
    ${serviceHasSourceCase}
            *)
              return 1
              ;;
          esac
        }

        service_selected() {
          local service="$1"
          local selected
          for selected in "''${selected_services[@]}"; do
            if [ "$selected" = "$service" ]; then
              return 0
            fi
          done
          return 1
        }

        resolve_service_source() {
          local service="$1"
          if [ -n "$target_source" ]; then
            printf '%s' "$target_source"
            return 0
          fi
          service_default_source "$service"
        }

        if [ "$target_service" != "all" ] && ! is_known_service "$target_service"; then
          echo "ERROR: unknown --service '$target_service'"
          exit 2
        fi

        if [ "$target_service" = "all" ]; then
          ${enabledServiceArrayInit}
        else
          if [ "$(is_service_enabled "$target_service")" != "1" ]; then
            echo "ERROR: selected service '$target_service' is disabled"
            exit 3
          fi
          selected_services=("$target_service")
        fi

        if [ -n "$target_source" ]; then
          if [ "$target_service" = "all" ]; then
            echo "ERROR: --source requires --service"
            exit 2
          fi
          if ! service_has_source "$target_service" "$target_source"; then
            echo "ERROR: unknown source '$target_source' for service '$target_service'"
            exit 3
          fi
        fi

  '';

  mkServiceProbeSection =
    mode: serviceName: spec:
    let
      modeLabel = if mode == "health" then "health" else "readiness";
      skipMessage = spec.skipMessage or "SKIP: ${serviceName} ${modeLabel} check not selected";
    in
    ''
      if service_selected "${serviceName}"; then
        service_source="$(resolve_service_source "${serviceName}")"
        if [ -z "$service_source" ]; then
          service_source="unspecified"
        fi
        checks=$((checks + ${toString spec.count}))
        ${spec.body}
      else
        echo ${lib.escapeShellArg skipMessage}
      fi
    '';

  mkProbeScript =
    {
      mode,
      serviceSpecs,
      emptyMessage,
      successMessage,
    }:
    ''
      set -euo pipefail
      ${slotEnvPrelude}
      ${serviceSelectionPrelude}

      if [ "$target_service" = "all" ] && [ ${toString (builtins.length enabledServiceNames)} -eq 0 ]; then
        echo ${lib.escapeShellArg emptyMessage}
        exit 0
      fi

      checks=0

      ${builtins.concatStringsSep "\n\n" (
        map (serviceName: mkServiceProbeSection mode serviceName serviceSpecs.${serviceName}) serviceNames
      )}

      if [ "$checks" -eq 0 ]; then
        echo ${lib.escapeShellArg emptyMessage}
        exit 0
      fi

      printf '%s services=%s\n' ${lib.escapeShellArg successMessage} "$checks"
    '';

  probePlan =
    mode: serviceName:
    resolvedServiceConfigByName.${serviceName}.resolved.operationProbes.${mode} or {
      count = 0;
      steps = [ ];
    };
  renderProbeStep = probeRuntime.renderProbeStep;

  mkServiceProbeSpec =
    mode: serviceName:
    let
      plan = probePlan mode serviceName;
      steps = plan.steps or [ ];
    in
    {
      count = if plan ? count then plan.count else builtins.length steps;
      body = builtins.concatStringsSep "\n" (map (step: renderProbeStep mode serviceName step) steps);
    };

  healthProbeSpecs = builtins.listToAttrs (
    map (serviceName: {
      name = serviceName;
      value = mkServiceProbeSpec "health" serviceName;
    }) serviceNames
  );

  readyProbeSpecs = builtins.listToAttrs (
    map (serviceName: {
      name = serviceName;
      value = mkServiceProbeSpec "ready" serviceName;
    }) serviceNames
  );

  mkTask =
    {
      id,
      appName,
      summary,
      description,
      command,
      runtimeInputs ? [ ],
      contractArgs ? [ ],
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
        input = {
          args = {
            parser = "typed";
            allowUnknown = false;
            spec = contractArgs;
          };
          env = {
            schemaRef = "runtimePrimitives";
            extra = [ ];
          };
        };
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
        ownerFile = "nixfied/modules/operations.nix";
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

  healthScript = mkProbeScript {
    mode = "health";
    serviceSpecs = healthProbeSpecs;
    emptyMessage = "SKIP: no enabled services for health checks";
    successMessage = "OK: health checks passed";
  };

  readyScript = mkProbeScript {
    mode = "ready";
    serviceSpecs = readyProbeSpecs;
    emptyMessage = "SKIP: no enabled services for readiness checks";
    successMessage = "OK: readiness checks passed";
  };

  isolationScript = testIsolationRuntime.mkIsolationScript {
    slotVar = isolationSlotVar;
    envVar = isolationEnvVar;
    slotMax = isolationMaxSlot;
    maxParallelDefault = cfg.testIsolation.maxParallel;
    logsRootBase = cfg.testIsolation.logsDir;
    runTaskId = isolationRunTaskId;
    validateTaskId = isolationValidateTaskId;
    keepLogsSuccess = cfg.testIsolation.keepLogsOnSuccess;
    keepLogsFailure = cfg.testIsolation.keepLogsOnFailure;
    slotsJson = isolationSlotsJson;
    envsJson = isolationEnvsJson;
    runArgsJson = isolationRunArgsJson;
    runEnvJson = isolationRunEnvJson;
    inherit envPattern;
  };
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

    testIsolation.runTaskId = lib.mkOption {
      type = lib.types.str;
      default = "task.ci";
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

    testIsolation.validateTaskId = lib.mkOption {
      type = lib.types.str;
      default = "task.ops.validate-env";
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
        contractArgs = testIsolationContractArgs;
        runtimeInputs = [
          pkgs.coreutils
          pkgs.jq
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
          Runs health checks for selected enabled services:
          postgres, nginx (http+https), minio (api+console),
          reth (http+ws+auth), and helios (rpc+execution).
          Optional selectors: --service <name|all> and --source <key>.
        '';
        command = healthScript;
        runtimeInputs = serviceProbeRuntimeInputs;
        contractArgs = serviceSelectionContractArgs;
      };
    })

    (lib.mkIf (cfg.enable && cfg.ready.enable) {
      nixfied.tasks."ready" = mkTask {
        id = "task.ops.ready";
        appName = "ready";
        summary = "Run service readiness checks";
        description = ''
          Runs readiness checks for selected enabled services:
          postgres, nginx (http+https), minio (api+console),
          reth (http+ws+auth), and helios (rpc+execution).
          Optional selectors: --service <name|all> and --source <key>.
        '';
        command = readyScript;
        runtimeInputs = serviceProbeRuntimeInputs;
        contractArgs = serviceSelectionContractArgs;
      };
    })
  ];
}
