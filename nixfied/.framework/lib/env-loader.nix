# Shared .env loading utilities.
{
  pkgs,
  project ? { },
}:

let
  lib = pkgs.lib;
  validation = import ./validation.nix { inherit pkgs; };
  inherit (validation)
    expect
    renderErrors
    isEnvVarName
    isScalar
    ;

  envFileCfg = ((project.tooling or { }).envFile or { });
  envFileEnabled = envFileCfg.enable or true;
  envFileStrict = envFileCfg.strict or false;
  allowSpecsRawValue = envFileCfg.allow or [ ];
  allowSpecsRaw = if builtins.isList allowSpecsRawValue then allowSpecsRawValue else [ ];

  supportedTypes = [
    "string"
    "int"
    "bool"
    "pathAbs"
    "pathRel"
    "port"
    "durationSec"
    "json"
  ];
  specErrors =
    builtins.concatLists (
      lib.imap0 (
        index: spec:
        let
          prefix = "tooling.envFile.allow[${toString index}]";
          type = spec.type or "string";
        in
        if !builtins.isAttrs spec then
          [ "${prefix}: entry must be an attribute set" ]
        else
          expect (isEnvVarName (spec.name or "")) "${prefix}.name must be a shell-safe env var token"
          ++ expect (
            !(spec ? type) || builtins.elem type supportedTypes
          ) "${prefix}.type must be one of ${builtins.concatStringsSep ", " supportedTypes}"
          ++ expect (
            !(spec ? required) || builtins.isBool spec.required
          ) "${prefix}.required must be a boolean when set"
          ++ expect (
            !(spec ? default) || isScalar spec.default
          ) "${prefix}.default must be a scalar when set"
      ) allowSpecsRaw
    );
  specNames = map (spec: if builtins.isAttrs spec then (spec.name or "") else "") allowSpecsRaw;
  duplicateSpecNames = builtins.filter (
    name: (builtins.length (builtins.filter (candidate: candidate == name) specNames)) > 1
  ) (lib.unique specNames);
  topLevelErrs =
    expect (builtins.isList allowSpecsRawValue) "tooling.envFile.allow must be a list"
    ++ expect (duplicateSpecNames == [ ]) "tooling.envFile.allow contains duplicate names: ${builtins.concatStringsSep ", " duplicateSpecNames}";

  allErrs = topLevelErrs ++ specErrors;

  normalizedAllowSpecs = map (
    spec:
    if builtins.isAttrs spec then
      {
        name = spec.name or "";
        type = spec.type or "string";
        required = spec.required or false;
        hasDefault = spec ? default;
        default = spec.default or null;
      }
    else
      {
        name = "";
        type = "string";
        required = false;
        hasDefault = false;
        default = null;
      }
  ) allowSpecsRaw;

  _ =
    if allErrs == [ ] then
      null
    else
      throw ''
        Nixfied env-file config violated:
        ${renderErrors allErrs}
      '';

  allowSpecsFile = pkgs.writeText "nixfied-env-file-specs.json" (builtins.toJSON normalizedAllowSpecs);

  loadEnvFile = pkgs.writeShellScript "nixfied-load-env-file" ''
    set -euo pipefail

    ENV_FILE="''${1:-.env}"
    ENV_FILE_ENABLED="${if envFileEnabled then "1" else "0"}"
    ENV_FILE_STRICT="${if envFileStrict then "1" else "0"}"
    ENV_SPECS_FILE="${allowSpecsFile}"

    if [ "$ENV_FILE_ENABLED" != "1" ]; then
      exit 0
    fi

    validate_env_value() {
      local key="$1"
      local type="$2"
      local value="$3"

      case "$type" in
        string)
          return 0
          ;;
        int)
          if ! printf '%s' "$value" | ${pkgs.gnugrep}/bin/grep -Eq '^-?[0-9]+$'; then
            echo "ERROR: .env key $key expects int (got '$value')" >&2
            return 1
          fi
          ;;
        bool)
          case "$value" in
            1|0|true|false|TRUE|FALSE|yes|no|on|off)
              ;;
            *)
              echo "ERROR: .env key $key expects bool (got '$value')" >&2
              return 1
              ;;
          esac
          ;;
        pathAbs)
          case "$value" in
            /*) ;;
            *)
              echo "ERROR: .env key $key expects absolute path (got '$value')" >&2
              return 1
              ;;
          esac
          ;;
        pathRel)
          case "$value" in
            ""|/*)
              echo "ERROR: .env key $key expects relative path (got '$value')" >&2
              return 1
              ;;
            *)
              ;;
          esac
          ;;
        port)
          if ! printf '%s' "$value" | ${pkgs.gnugrep}/bin/grep -Eq '^[0-9]+$'; then
            echo "ERROR: .env key $key expects TCP port (got '$value')" >&2
            return 1
          fi
          if [ "$value" -lt 1 ] || [ "$value" -gt 65535 ]; then
            echo "ERROR: .env key $key expects TCP port 1-65535 (got '$value')" >&2
            return 1
          fi
          ;;
        durationSec)
          if ! printf '%s' "$value" | ${pkgs.gnugrep}/bin/grep -Eq '^[0-9]+$'; then
            echo "ERROR: .env key $key expects durationSec integer (got '$value')" >&2
            return 1
          fi
          if [ "$value" -le 0 ]; then
            echo "ERROR: .env key $key expects durationSec > 0 (got '$value')" >&2
            return 1
          fi
          ;;
        json)
          if ! printf '%s' "$value" | ${pkgs.jq}/bin/jq -e . >/dev/null 2>&1; then
            echo "ERROR: .env key $key expects valid JSON" >&2
            return 1
          fi
          ;;
        *)
          echo "ERROR: unsupported env spec type key=$key type=$type" >&2
          return 1
          ;;
      esac
      return 0
    }

    if [ ! -f "$ENV_FILE" ]; then
      if [ "$ENV_FILE_STRICT" = "1" ]; then
        while IFS= read -r SPEC_B64; do
          [ -z "$SPEC_B64" ] && continue
          SPEC_JSON="$(printf '%s' "$SPEC_B64" | ${pkgs.coreutils}/bin/base64 -d)"
          SPEC_NAME="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.name')"
          SPEC_REQUIRED="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.required')"
          if [ "$SPEC_REQUIRED" = "true" ] && [ -z "''${!SPEC_NAME:-}" ]; then
            echo "ERROR: required env key missing key=$SPEC_NAME source=.env" >&2
            exit 1
          fi
        done < <(${pkgs.jq}/bin/jq -r '.[] | @base64' "$ENV_SPECS_FILE")
      fi
      exit 0
    fi

    SPECS_MAP_JSON="$(${pkgs.jq}/bin/jq -c 'reduce .[] as $spec ({}; . + {($spec.name): $spec})' "$ENV_SPECS_FILE")"

    while IFS= read -r RAW_LINE || [ -n "$RAW_LINE" ]; do
      LINE="$(printf '%s' "$RAW_LINE" | ${pkgs.gnused}/bin/sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
      case "$LINE" in
        \#*|"")
          continue
          ;;
      esac

      case "$LINE" in
        *=*)
          ;;
        *)
          echo "ERROR: invalid .env line (missing '=') line='$LINE'" >&2
          exit 1
          ;;
      esac

      key="$(printf '%s' "$LINE" | ${pkgs.gnused}/bin/sed -E 's/=.*$//; s/[[:space:]]+$//')"
      value="$(printf '%s' "$LINE" | ${pkgs.gnused}/bin/sed -E 's/^[^=]*=//')"

      if ! printf '%s' "$key" | ${pkgs.gnugrep}/bin/grep -Eq '^[A-Z_][A-Z0-9_]*$'; then
        echo "ERROR: invalid .env key token key='$key'" >&2
        exit 1
      fi

      value="$(printf '%s' "$value" | ${pkgs.gnused}/bin/sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")"

      SPEC_JSON="$(printf '%s\n' "$SPECS_MAP_JSON" | ${pkgs.jq}/bin/jq -c --arg key "$key" '.[$key] // null')"
      KNOWN_KEY="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r 'if . == null then "0" else "1" end')"

      if [ "$KNOWN_KEY" != "1" ] && [ "$ENV_FILE_STRICT" = "1" ]; then
        echo "ERROR: unknown .env key key=$key (strict mode enabled)" >&2
        exit 1
      fi

      if [ "$KNOWN_KEY" = "1" ]; then
        SPEC_TYPE="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.type // "string"')"
        if [ -z "''${!key:-}" ]; then
          validate_env_value "$key" "$SPEC_TYPE" "$value" || exit 1
        else
          validate_env_value "$key" "$SPEC_TYPE" "''${!key}" || exit 1
        fi
      fi

      if [ -z "''${!key:-}" ]; then
        export "$key=$value"
      fi
    done < "$ENV_FILE"

    while IFS= read -r SPEC_B64; do
      [ -z "$SPEC_B64" ] && continue
      SPEC_JSON="$(printf '%s' "$SPEC_B64" | ${pkgs.coreutils}/bin/base64 -d)"
      SPEC_NAME="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.name')"
      SPEC_TYPE="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.type // "string"')"
      SPEC_REQUIRED="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.required')"
      SPEC_HAS_DEFAULT="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.hasDefault')"

      if [ -z "''${!SPEC_NAME:-}" ]; then
        if [ "$SPEC_HAS_DEFAULT" = "true" ]; then
          SPEC_DEFAULT="$(printf '%s\n' "$SPEC_JSON" | ${pkgs.jq}/bin/jq -r '.default | tostring')"
          validate_env_value "$SPEC_NAME" "$SPEC_TYPE" "$SPEC_DEFAULT" || exit 1
          export "$SPEC_NAME=$SPEC_DEFAULT"
        elif [ "$SPEC_REQUIRED" = "true" ]; then
          echo "ERROR: required env key missing key=$SPEC_NAME source=.env" >&2
          exit 1
        fi
      else
        validate_env_value "$SPEC_NAME" "$SPEC_TYPE" "''${!SPEC_NAME}" || exit 1
      fi
    done < <(${pkgs.jq}/bin/jq -r '.[] | @base64' "$ENV_SPECS_FILE")
  '';

  loadEnv = pkgs.writeShellScript "nixfied-load-env" ''
    set -euo pipefail
    ${loadEnvFile} ".env"
  '';
in
{
  inherit
    loadEnvFile
    loadEnv
    ;
}
