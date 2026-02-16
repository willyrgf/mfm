# Slot + environment utilities for the framework
{ pkgs, project }:

let
  cfg = project;
  projectMeta = cfg.project or { };

  envVar = projectMeta.envVar or "PROJECT_ENV";
  slotVar = projectMeta.slotVar or "NIX_ENV";

  slotsCfg = cfg.slots or { };
  slotMax = slotsCfg.max or 9;
  slotStride = slotsCfg.stride or 1;
  slotDefaultRaw = slotsCfg.default or 0;
  slotDefault =
    if !(builtins.isInt slotDefaultRaw) then
      throw "slots.default must be an integer 0-${toString slotMax} (got type ${builtins.typeOf slotDefaultRaw})"
    else if slotDefaultRaw < 0 || slotDefaultRaw > slotMax then
      throw "slots.default must be 0-${toString slotMax} (got ${toString slotDefaultRaw})"
    else
      slotDefaultRaw;

  envOffsets = builtins.mapAttrs (name: envCfg: envCfg.offset or 0) (cfg.envs or { });
  envNames = builtins.attrNames envOffsets;
  defaultEnv =
    if builtins.hasAttr "dev" envOffsets then
      "dev"
    else if envNames != [ ] then
      builtins.head envNames
    else
      "dev";

  projectId = projectMeta.id or "project";
  projectIdUpper = normalizeName projectId;

  postgresEnabled = (cfg.modules.postgres.enable or false);
  postgresDatabase = cfg.modules.postgres.database or "app";
  postgresTestDatabase = cfg.modules.postgres.testDatabase or "app_test";
  postgresPortKey = cfg.modules.postgres.portKey or "postgres";

  ports = cfg.ports or { };
  portNames = builtins.attrNames ports;

  baseDirExpr = (cfg.directories.base or "\${XDG_DATA_HOME:-$HOME/.local/share}/project");

  servicesCfg = cfg.services or { };
  rawServiceNames = servicesCfg.names or [ ];
  defaultServiceNames = if rawServiceNames != [ ] then rawServiceNames else portNames;
  defaultServiceSockets =
    if builtins.hasAttr "postgres" ports then { postgres = ".s.PGSQL.$PGPORT"; } else { };
  serviceSockets = defaultServiceSockets // (servicesCfg.sockets or { });
  serviceSocketNames = builtins.attrNames serviceSockets;
  uniqueNames =
    names:
    builtins.attrNames (
      builtins.listToAttrs (
        map (name: {
          inherit name;
          value = true;
        }) names
      )
    );
  serviceNames = uniqueNames (defaultServiceNames ++ serviceSocketNames);

  normalizeName =
    name:
    let
      replaced = pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] name;
    in
    pkgs.lib.strings.toUpper replaced;

  portVarName = name: "${normalizeName name}_PORT";

  portAssignments = pkgs.lib.concatMapStringsSep "\n" (name: ''
    ${portVarName name}=$((${toString ports.${name}} + SLOT * ${toString slotStride} + ENV_OFFSET))
  '') portNames;

  portExports = pkgs.lib.concatMapStringsSep "\n" (
    name: "echo \"${portVarName name}=${"$"}${portVarName name}\""
  ) portNames;

  envCase = pkgs.lib.concatMapStringsSep "\n" (
    name: "  ${name}) ENV_OFFSET=${toString envOffsets.${name}} ;;"
  ) envNames;

  envList = pkgs.lib.concatStringsSep " " envNames;

  # Resolve and validate explicit environment selection
  resolveEnv = pkgs.writeShellScript "resolve-env" ''
    set -eo pipefail
    ENV_VAR="${envVar}"

    if [ -z "''${!ENV_VAR:-}" ]; then
      echo "ERROR: $ENV_VAR must be set (example: ${slotVar}=${toString slotDefault} $ENV_VAR=${defaultEnv})" >&2
      exit 1
    fi

    ENV_VALUE="''${!ENV_VAR}"
    case "$ENV_VALUE" in
      ${pkgs.lib.concatStringsSep "|" envNames})
        echo "$ENV_VALUE"
        exit 0
        ;;
      *)
        echo "ERROR: $ENV_VAR must be one of: ${envList} (got '$ENV_VALUE')" >&2
        exit 1
        ;;
    esac
  '';

  # Read and validate slot value (defaults when unset)
  resolveSlot = pkgs.writeShellScript "resolve-slot" ''
    set -eo pipefail
    SLOT_VAR="${slotVar}"

    # Compatibility aliases:
    # - NIXFIED_ENV: alias for the configured slot variable (default: NIX_ENV).
    if [ -z "''${!SLOT_VAR:-}" ] && [ -n "''${NIXFIED_ENV:-}" ]; then
      export "$SLOT_VAR"="''${NIXFIED_ENV}"
    fi

    if [ -z "''${!SLOT_VAR:-}" ]; then
      export "$SLOT_VAR"="${toString slotDefault}"
      echo "INFO: default slot selected slot_var=$SLOT_VAR slot=${toString slotDefault}" >&2
    fi

    SLOT_VALUE="''${!SLOT_VAR}"
    case "$SLOT_VALUE" in
      *[!0-9]*)
        echo "ERROR: $SLOT_VAR must be an integer 0-${toString slotMax} (got '$SLOT_VALUE')" >&2
        exit 1
        ;;
      "")
        echo "ERROR: $SLOT_VAR must be set (example: $SLOT_VAR=${toString slotDefault} ${envVar}=${defaultEnv})" >&2
        exit 1
        ;;
      *)
        ;;
    esac

    if [ "$SLOT_VALUE" -lt 0 ] || [ "$SLOT_VALUE" -gt ${toString slotMax} ]; then
      echo "ERROR: $SLOT_VAR must be 0-${toString slotMax} (got $SLOT_VALUE)" >&2
      exit 1
    fi

    echo "$SLOT_VALUE"
    exit 0
  '';

  # Validate slot/env and emit eval-able variables
  requireSlotEnv = pkgs.writeShellScript "require-slot-env" ''
    set -eo pipefail
    SLOT_VAR="${slotVar}"
    ENV_VAR="${envVar}"

    SLOT=$(${resolveSlot})
    ENV=$(${resolveEnv})

    case "$ENV" in
      ${pkgs.lib.concatStringsSep "|" envNames})
        ;;
      *)
        echo "ERROR: $ENV_VAR must be one of: ${envList} (got '$ENV')" >&2
        exit 1
        ;;
    esac

    case "$ENV" in
    ${envCase}
    esac

    ${portAssignments}

    # Export canonical slot/env hints so child hook launchers do not need to
    # re-default slot selection when callers already resolved slot/env once.
    echo "export ${slotVar}=$SLOT"
    echo "export ${envVar}=$ENV"
    echo "export NIXFIED_ENV=$SLOT"
    echo "SLOT=$SLOT"
    echo "ENV=$ENV"
  '';

  # Get slot + environment configuration as eval-able shell variables
  getSlotInfo = pkgs.writeShellScript "get-slot-info" ''
    set -eo pipefail
    SLOT_VAR="${slotVar}"
    ENV_VAR="${envVar}"

    SLOT=$(${resolveSlot})
    ENV=$(${resolveEnv})

    case "$ENV" in
      ${pkgs.lib.concatStringsSep "|" envNames})
        ;;
      *)
        echo "ERROR: $ENV_VAR must be one of: ${envList} (got '$ENV')" >&2
        exit 1
        ;;
    esac

    case "$ENV" in
    ${envCase}
    esac

    # Ephemeral path override
    if [ "''${${projectIdUpper}_EPHEMERAL:-}" = "1" ] && [ -n "''${${projectIdUpper}_EPHEMERAL_ROOT:-}" ]; then
      BASE_DIR="''${${projectIdUpper}_EPHEMERAL_ROOT}/data/${projectId}"
    else
      BASE_DIR="${baseDirExpr}"
    fi

    LOG_DIR="$BASE_DIR/logs-$SLOT-$ENV"
    RUN_DIR="$BASE_DIR/run-$SLOT-$ENV"
    CONFIG_DIR="$BASE_DIR/config-$SLOT-$ENV"
    STATE_DIR="$BASE_DIR/state-$SLOT-$ENV"
    BACKUP_BASE_DIR="$BASE_DIR/backups/slot-$SLOT-$ENV"

    ${portAssignments}

    ${pkgs.lib.concatMapStringsSep "\n" (
      name:
      let
        upper = normalizeName name;
      in
      ''
        ${upper}_DIR="$BASE_DIR/${name}-$SLOT-$ENV"
        ${upper}_LOG_DIR="$BASE_DIR/${name}-$SLOT-$ENV/logs"
        ${upper}_RUN_DIR="$BASE_DIR/${name}-$SLOT-$ENV/run"
        ${upper}_CONFIG_DIR="$BASE_DIR/${name}-$SLOT-$ENV/config"
        ${upper}_STATE_DIR="$BASE_DIR/${name}-$SLOT-$ENV/state"
        ${upper}_SOCKET_DIR="$BASE_DIR/${name}-$SLOT-$ENV/run/sockets"
      ''
    ) serviceNames}

    ${pkgs.lib.concatMapStringsSep "\n" (
      name:
      let
        upper = normalizeName name;
      in
      ''
        ${upper}_SOCKET_NAME="${serviceSockets.${name}}"
        if [ -n "${"$"}{${upper}_SOCKET_NAME:-}" ]; then
          ${upper}_SOCKET="${"$"}{${upper}_SOCKET_DIR}/${"$"}{${upper}_SOCKET_NAME}"
        fi
      ''
    ) serviceSocketNames}

    # Export canonical slot/env hints so child hook launchers do not need to
    # re-default slot selection when callers already resolved slot/env once.
    echo "export ${slotVar}=$SLOT"
    echo "export ${envVar}=$ENV"
    echo "export NIXFIED_ENV=$SLOT"
    echo "SLOT=$SLOT"
    echo "ENV=$ENV"
    echo "ENV_OFFSET=$ENV_OFFSET"
    echo "SLOT_STRIDE=${toString slotStride}"
    echo "BASE_DIR=$BASE_DIR"
    echo "LOG_DIR=$LOG_DIR"
    echo "RUN_DIR=$RUN_DIR"
    echo "CONFIG_DIR=$CONFIG_DIR"
    echo "STATE_DIR=$STATE_DIR"
    echo "BACKUP_BASE_DIR=$BACKUP_BASE_DIR"
    ${pkgs.lib.concatMapStringsSep "\n" (
      name:
      let
        upper = normalizeName name;
      in
      ''
        echo "${upper}_DIR=${"$"}${upper}_DIR"
        echo "${upper}_LOG_DIR=${"$"}${upper}_LOG_DIR"
        echo "${upper}_RUN_DIR=${"$"}${upper}_RUN_DIR"
        echo "${upper}_CONFIG_DIR=${"$"}${upper}_CONFIG_DIR"
        echo "${upper}_STATE_DIR=${"$"}${upper}_STATE_DIR"
        echo "${upper}_SOCKET_DIR=${"$"}${upper}_SOCKET_DIR"
      ''
    ) serviceNames}
    ${pkgs.lib.concatMapStringsSep "\n" (
      name:
      let
        upper = normalizeName name;
      in
      ''
        if [ -n "${"$"}{${upper}_SOCKET:-}" ]; then
          echo "${upper}_SOCKET=${"$"}${upper}_SOCKET"
        fi
      ''
    ) serviceSocketNames}
    ${portExports}
    ${pkgs.lib.optionalString (postgresEnabled && builtins.hasAttr postgresPortKey ports) ''
      DATABASE_URL="postgresql://localhost:${"$"}${portVarName postgresPortKey}/${postgresDatabase}"
      TEST_DATABASE_URL="postgresql://localhost:${"$"}${portVarName postgresPortKey}/${postgresTestDatabase}"
      echo "DATABASE_URL=$DATABASE_URL"
      echo "TEST_DATABASE_URL=$TEST_DATABASE_URL"
    ''}
  '';

  kvLinesToJson = pkgs.writeShellScript "slot-kv-lines-to-json" ''
    set -euo pipefail
    ${pkgs.jq}/bin/jq -Rn '
      [ inputs
        | select(length > 0)
        | select(startswith("export ") | not)
        | capture("^(?<key>[^=]+)=(?<value>.*)$")
      ]
      | reduce .[] as $entry ({}; . + {($entry.key): $entry.value})
    '
  '';

  getSlotInfoJson = pkgs.writeShellScript "get-slot-info-json" ''
    set -euo pipefail

    RAW="$(${getSlotInfo})"
    VARS_JSON="$(printf '%s\n' "$RAW" | ${kvLinesToJson})"

    ${pkgs.jq}/bin/jq -n --argjson vars "$VARS_JSON" '
      {
        slot: (($vars.SLOT // "0") | tonumber),
        env: ($vars.ENV // ""),
        env_offset: (($vars.ENV_OFFSET // "0") | tonumber),
        slot_stride: (($vars.SLOT_STRIDE // "0") | tonumber),
        directories: {
          base: ($vars.BASE_DIR // ""),
          log: ($vars.LOG_DIR // ""),
          run: ($vars.RUN_DIR // ""),
          config: ($vars.CONFIG_DIR // ""),
          state: ($vars.STATE_DIR // ""),
          backup: ($vars.BACKUP_BASE_DIR // "")
        },
        ports: (
          $vars
          | to_entries
          | map(select(.key | endswith("_PORT")))
          | reduce .[] as $entry ({}; . + {($entry.key): (($entry.value | tonumber?) // $entry.value)})
        ),
        vars: $vars
      }
    '
  '';

  requireSlotEnvJson = pkgs.writeShellScript "require-slot-env-json" ''
    set -euo pipefail

    RAW="$(${requireSlotEnv})"
    VARS_JSON="$(printf '%s\n' "$RAW" | ${kvLinesToJson})"

    ${pkgs.jq}/bin/jq -n --argjson vars "$VARS_JSON" '
      {
        slot: (($vars.SLOT // "0") | tonumber),
        env: ($vars.ENV // ""),
        ports: (
          $vars
          | to_entries
          | map(select(.key | endswith("_PORT")))
          | reduce .[] as $entry ({}; . + {($entry.key): (($entry.value | tonumber?) // $entry.value)})
        ),
        vars: $vars
      }
    '
  '';

  getServiceDir = service: "\${BASE_DIR:-${baseDirExpr}}/${service}-$SLOT-$ENV";

  # Nix-level accessor: calculate ports for a given slot/env
  calculatePorts =
    { slot, env }:
    let
      offset = envOffsets.${env} or 0;
    in
    builtins.mapAttrs (name: base: base + (slot * slotStride) + offset) ports;

  # Nix-level accessor: get database URL for env and port
  getDatabaseUrl =
    env: port:
    "postgresql://localhost:${toString port}/${
      if env == "test" then postgresTestDatabase else postgresDatabase
    }";

  # Nix-level accessor: validate slot/env pair
  validateSlotEnv =
    { slot, env }:
    if slot < 0 || slot > slotMax then
      {
        valid = false;
        error = "${slotVar} must be 0-${toString slotMax} (got ${toString slot})";
      }
    else if !(builtins.hasAttr env envOffsets) then
      {
        valid = false;
        error = "${envVar} must be one of: ${envList} (got '${env}')";
      }
    else
      {
        valid = true;
        error = "";
      };

  # Nix-level accessor: full config for a slot/env
  getFullConfig =
    { slot, env }:
    let
      computed = calculatePorts { inherit slot env; };
      offset = envOffsets.${env} or 0;
    in
    {
      ports = computed;
      directories = {
        base = baseDirExpr;
        log = "logs-${toString slot}-${env}";
        run = "run-${toString slot}-${env}";
        config = "config-${toString slot}-${env}";
        state = "state-${toString slot}-${env}";
        backup = "backups/slot-${toString slot}-${env}";
      };
      urls = pkgs.lib.optionalAttrs (postgresEnabled && builtins.hasAttr postgresPortKey computed) {
        database = getDatabaseUrl env computed.${postgresPortKey};
        testDatabase = getDatabaseUrl "test" computed.${postgresPortKey};
      };
      environment = {
        inherit slot env offset;
        stride = slotStride;
      };
    };

in
{
  inherit
    baseDirExpr
    envOffsets
    ports
    portVarName
    normalizeName
    resolveEnv
    resolveSlot
    requireSlotEnv
    requireSlotEnvJson
    getSlotInfo
    getSlotInfoJson
    getServiceDir
    serviceNames
    serviceSockets
    slotMax
    slotStride
    calculatePorts
    getDatabaseUrl
    validateSlotEnv
    getFullConfig
    ;
}
