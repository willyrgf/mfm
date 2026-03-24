# Shared shell snippets for loading slot/env runtime context from compiled shell assignments.
{ pkgs }:

let
  lib = pkgs.lib;
in
rec {
  evalAssignments =
    {
      assignmentsVar,
    }:
    ''
      eval "${"$" + assignmentsVar}"
    '';

  loadSlotEnvFromJson =
    {
      jsonVar,
      slotVar ? "SLOT",
      envVar ? "ENV",
    }:
    ''
      ${evalAssignments { assignmentsVar = jsonVar; }}
      ${lib.optionalString (slotVar != "SLOT") ''
        ${slotVar}="''${SLOT:-}"
      ''}
      ${lib.optionalString (envVar != "ENV") ''
        ${envVar}="''${ENV:-}"
      ''}
    '';

  exportVarsFromJson =
    {
      jsonVar,
    }:
    ''
      ${evalAssignments { assignmentsVar = jsonVar; }}
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
      fieldExpr,
    }:
    ''
      ${evalAssignments { assignmentsVar = jsonVar; }}
      __slot_info_field_var=""
      case ${lib.escapeShellArg fieldExpr} in
        .slot)
          __slot_info_field_var="SLOT"
          ;;
        .env)
          __slot_info_field_var="ENV"
          ;;
        .directories.run)
          __slot_info_field_var="RUN_DIR"
          ;;
        .directories.log)
          __slot_info_field_var="LOG_DIR"
          ;;
        .directories.config)
          __slot_info_field_var="CONFIG_DIR"
          ;;
        *)
          echo "ERROR: unsupported slot info expression ${fieldExpr}" >&2
          exit 1
          ;;
      esac
      ${targetVar}="''${!__slot_info_field_var:-}"
    '';

  readPortFromJson =
    {
      targetVar,
      jsonVar,
      keyExpr,
    }:
    ''
      ${evalAssignments { assignmentsVar = jsonVar; }}
      __slot_info_port_var="${keyExpr}"
      ${targetVar}="''${!__slot_info_port_var:-}"
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
