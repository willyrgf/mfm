{
  lib,
  conf,
  project,
}:
let
  envNames = builtins.attrNames conf.envs;

  envOffsetCase = builtins.concatStringsSep "\n" (
    map (
      envName: "    ${envName}) env_offset=${toString (conf.envs.${envName}.offset or 0)} ;;"
    ) envNames
  );

  postgresPortBase = conf.ports.postgres;
  minioApiPortBase = conf.ports.minioApi or conf.ports.minio;
  minioConsolePortBase = conf.ports.minioConsole or conf.ports.minio_console;
  rethHttpPortBase = conf.ports.rethHttp;
  rethWsPortBase = conf.ports.rethWs;
  rethAuthPortBase = conf.ports.rethAuth;
in
{
  inherit envOffsetCase;

  servicePortPrelude = ''
    slot_var=${lib.escapeShellArg project.slotVar}
    env_var=${lib.escapeShellArg project.envVar}
    slot_default=${toString conf.slots.default}
    env_default=${lib.escapeShellArg "dev"}

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

    POSTGRES_PORT=$(( ${toString postgresPortBase} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
    MINIO_API_PORT=$(( ${toString minioApiPortBase} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
    MINIO_CONSOLE_PORT=$(( ${toString minioConsolePortBase} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
    RETH_HTTP_PORT=$(( ${toString rethHttpPortBase} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
    RETH_WS_PORT=$(( ${toString rethWsPortBase} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
    RETH_AUTH_PORT=$(( ${toString rethAuthPortBase} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
  '';
}
