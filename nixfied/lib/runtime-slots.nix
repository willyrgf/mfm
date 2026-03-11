{
  pkgs,
  lib ? pkgs.lib,
}:
{ model }:
let
  runtime = model.runtime;
  envOffsets = runtime.env.offsets or { };
  envNames = builtins.sort builtins.lessThan (builtins.attrNames envOffsets);
  portNames = builtins.sort builtins.lessThan (builtins.attrNames (runtime.ports or { }));

  upperSnake =
    value:
    let
      chars = lib.stringToCharacters value;
      folded = builtins.foldl' (
        acc: ch:
        let
          isUpper = builtins.match "[A-Z]" ch != null;
          isLower = builtins.match "[a-z]" ch != null;
          isDigit = builtins.match "[0-9]" ch != null;
          isWord = isUpper || isLower || isDigit;
          needsDelimiter =
            (!isWord && acc.out != "" && !acc.prevUnderscore)
            || (isUpper && acc.prevLowerOrDigit);
          nextOut =
            acc.out
            + (if needsDelimiter then "_" else "")
            + (if isWord then lib.toUpper ch else "");
        in
        {
          out = nextOut;
          prevUnderscore = if isWord then false else nextOut != "";
          prevLowerOrDigit = isLower || isDigit;
        }
      ) {
        out = "";
        prevUnderscore = false;
        prevLowerOrDigit = false;
      } chars;
    in
    folded.out;

  portVarName = key: "${upperSnake key}_PORT";

  envOffsetCase = builtins.concatStringsSep "\n" (
    map (envName: ''
      ${lib.escapeShellArg envName})
        env_offset=${toString (envOffsets.${envName} or 0)}
        ;;
    '') envNames
  );

  portMappingsTsv = builtins.concatStringsSep "\n" (
    map (portName: "${portVarName portName}\t${toString runtime.ports.${portName}}") portNames
  );

  getSlotInfoJson = pkgs.writeShellScript "nixfied-slot-info-json" ''
    set -euo pipefail

    slot_var=${lib.escapeShellArg runtime.slot.var}
    env_var=${lib.escapeShellArg runtime.env.var}
    slot_default=${lib.escapeShellArg (toString runtime.slot.default)}
    env_default=${lib.escapeShellArg runtime.env.default}
    slot_stride=${lib.escapeShellArg (toString runtime.slot.stride)}
    port_mappings_tsv=${lib.escapeShellArg portMappingsTsv}

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
      echo "ERROR: $slot_var must be an integer" >&2
      exit 3
    fi

    env_offset=""
    case "$env_value" in
${envOffsetCase}
      *)
        echo "ERROR: unsupported $env_var '$env_value'" >&2
        exit 3
        ;;
    esac

    reuse_service_root="''${NIXFIED_REUSE_SERVICE_ROOT:-}"
    runtime_service_root="''${NIXFIED_SERVICE_ROOT:-}"
    run_dir="$reuse_service_root"
    if [ -z "$run_dir" ]; then
      run_dir="$runtime_service_root"
    fi

    ports_json="$(
      while IFS=$'\t' read -r port_var port_base || [ -n "$port_var" ]; do
        if [ -z "$port_var" ] || [ -z "$port_base" ]; then
          continue
        fi
        printf '%s\t%s\n' "$port_var" "$(( port_base + env_offset + (slot_value * slot_stride) ))"
      done <<< "$port_mappings_tsv" | ${pkgs.jq}/bin/jq -Rn '
        reduce inputs as $line (
          {};
          ($line | split("\t")) as $parts
          | if ($parts | length) >= 2 then
              . + { ($parts[0]): ($parts[1] | tonumber) }
            else
              .
            end
        )
      '
    )"

    ${pkgs.jq}/bin/jq -cn \
      --arg slot "$slot_value" \
      --arg env "$env_value" \
      --arg slotVar "$slot_var" \
      --arg envVar "$env_var" \
      --arg reuseServiceRoot "$reuse_service_root" \
      --arg runtimeServiceRoot "$runtime_service_root" \
      --arg runDir "$run_dir" \
      --arg runtimeScope "''${NIXFIED_RUNTIME_DIR_SCOPE:-}" \
      --arg runtimeBase "''${NIXFIED_RUNTIME_DIR_BASE:-}" \
      --arg registryRoot "''${REGISTRY_ROOT:-}" \
      --arg artifactsDir "''${CI_ARTIFACTS_DIR:-}" \
      --argjson ports "$ports_json" \
      '
        {
          slot: $slot,
          env: $env,
          vars:
            (
              {
                ($slotVar): $slot,
                ($envVar): $env,
                NIXFIED_RUNTIME_SLOT: $slot,
                NIXFIED_RUNTIME_ENV: $env
              }
              + (if $reuseServiceRoot != "" then { NIXFIED_REUSE_SERVICE_ROOT: $reuseServiceRoot } else { } end)
              + (if $runtimeServiceRoot != "" then { NIXFIED_SERVICE_ROOT: $runtimeServiceRoot } else { } end)
              + (if $registryRoot != "" then { REGISTRY_ROOT: $registryRoot } else { } end)
              + (if $artifactsDir != "" then { CI_ARTIFACTS_DIR: $artifactsDir } else { } end)
              + ($ports | with_entries(.value |= tostring))
            ),
          ports: $ports,
          directories: {
            run: $runDir,
            runtimeScope: $runtimeScope,
            runtimeBase: $runtimeBase,
            reuseServiceRoot: $reuseServiceRoot,
            runtimeServiceRoot: $runtimeServiceRoot
          }
        }
      '
  '';

  getSlotInfo = pkgs.writeShellScript "nixfied-slot-info" ''
    set -euo pipefail

    slot_info_json="$(${getSlotInfoJson})" || exit $?

    exec ${pkgs.jq}/bin/jq -r '
      def emit($name; $value): "\($name)=\($value | @sh)";
      [
        emit("SLOT"; (.slot // "")),
        emit("ENV"; (.env // ""))
      ]
      + (
          (.vars // {})
          | to_entries
          | map("export " + .key + "=" + (.value | tostring | @sh))
        )
      | .[]
    ' <<<"$slot_info_json"
  '';
in
{
  inherit
    portVarName
    getSlotInfo
    getSlotInfoJson
    ;

  getServiceDir =
    dataDirName:
    "\${NIXFIED_REUSE_SERVICE_ROOT:-\${NIXFIED_SERVICE_ROOT:-/tmp/nixfied-services}}/${dataDirName}";
}
