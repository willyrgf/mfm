# Shared shell snippets for loading slot/env runtime context from JSON helpers.
{ pkgs }:

let
  lib = pkgs.lib;
  jqBin = "${pkgs.jq}/bin/jq";
  base64Bin = "${pkgs.coreutils}/bin/base64";
in
rec {
  loadSlotEnvFromJson =
    {
      jsonVar,
      slotVar ? "SLOT",
      envVar ? "ENV",
    }:
    ''
      ${slotVar}="$(${jqBin} -r '.slot' <<<"${"$" + jsonVar}")"
      ${envVar}="$(${jqBin} -r '.env' <<<"${"$" + jsonVar}")"
    '';

  exportVarsFromJson =
    {
      jsonVar,
    }:
    ''
      while IFS= read -r ENTRY_B64; do
        [ -z "$ENTRY_B64" ] && continue
        KEY="$(printf '%s' "$ENTRY_B64" | ${base64Bin} -d | ${jqBin} -r '.key')"
        VALUE="$(printf '%s' "$ENTRY_B64" | ${base64Bin} -d | ${jqBin} -r '.value | tostring')"
        export "$KEY=$VALUE"
      done < <(printf '%s\n' "${"$" + jsonVar}" | ${jqBin} -r '.vars // {} | to_entries[] | @base64')
    '';

  loadSlotEnvAndVarsFromJson =
    {
      jsonVar,
      slotVar ? "SLOT",
      envVar ? "ENV",
    }:
    ''
      ${loadSlotEnvFromJson {
        inherit
          jsonVar
          slotVar
          envVar
          ;
      }}
      ${exportVarsFromJson { inherit jsonVar; }}
    '';

  loadJsonFromCommand =
    {
      outVar,
      command,
      slotVar ? "SLOT",
      envVar ? "ENV",
      exportVars ? false,
    }:
    ''
      ${outVar}="$(${command})" || exit 1
      ${
        if exportVars then
          loadSlotEnvAndVarsFromJson {
            jsonVar = outVar;
            inherit
              slotVar
              envVar
              ;
          }
        else
          loadSlotEnvFromJson {
            jsonVar = outVar;
            inherit
              slotVar
              envVar
              ;
          }
      }
    '';

  readJsonField =
    {
      targetVar,
      jsonVar,
      jqExpr,
    }:
    ''
      ${targetVar}="$(${jqBin} -r ${lib.escapeShellArg jqExpr} <<<"${"$" + jsonVar}")"
    '';

  readPortFromJson =
    {
      targetVar,
      jsonVar,
      keyExpr,
    }:
    ''
      ${targetVar}="$(${jqBin} -r --arg key "${keyExpr}" '.ports[$key] // empty' <<<"${"$" + jsonVar}")"
    '';

  requireSlotEnvJson =
    {
      outVar ? "SLOT_ENV_JSON",
      cmdVar ? "REQUIRE_SLOT_ENV_JSON_CMD",
      slotVar ? "SLOT",
      envVar ? "ENV",
      exportVars ? true,
      checkExecutable ? false,
      missingMsg ? "ERROR: REQUIRE_SLOT_ENV_JSON is not set; run via nixfied app/hook context.",
      notExecMsg ? "ERROR: REQUIRE_SLOT_ENV_JSON is not executable: $REQUIRE_SLOT_ENV_JSON_CMD",
    }:
    ''
      ${cmdVar}="''${REQUIRE_SLOT_ENV_JSON:-}"
      if [ -z "${"$" + cmdVar}" ]; then
        echo "${missingMsg}" >&2
        exit 1
      fi
      ${pkgs.lib.optionalString checkExecutable ''
        if [ ! -x "${"$" + cmdVar}" ]; then
          echo "${notExecMsg}" >&2
          exit 1
        fi
      ''}

      ${outVar}="$("${"$" + cmdVar}")" || exit 1
      ${
        if exportVars then
          loadSlotEnvAndVarsFromJson {
            jsonVar = outVar;
            inherit
              slotVar
              envVar
              ;
          }
        else
          loadSlotEnvFromJson {
            jsonVar = outVar;
            inherit
              slotVar
              envVar
              ;
          }
      }
    '';

  requireSlotInfoJson =
    {
      outVar ? "SLOT_INFO_JSON_OUT",
      slotVar ? "SLOT",
      envVar ? "ENV",
      exportVars ? true,
      missingMsg ? "ERROR: SLOT_INFO_JSON not available",
    }:
    ''
      if [ -z "''${SLOT_INFO_JSON:-}" ] || [ ! -x "$SLOT_INFO_JSON" ]; then
        echo "${missingMsg}" >&2
        exit 1
      fi

      ${outVar}="$("$SLOT_INFO_JSON")" || exit 1
      ${
        if exportVars then
          loadSlotEnvAndVarsFromJson {
            jsonVar = outVar;
            inherit
              slotVar
              envVar
              ;
          }
        else
          loadSlotEnvFromJson {
            jsonVar = outVar;
            inherit
              slotVar
              envVar
              ;
          }
      }
    '';
}
