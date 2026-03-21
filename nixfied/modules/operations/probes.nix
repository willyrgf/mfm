{
  lib,
  pkgs,
  runtime,
  services,
  serviceNames,
  resolvePortBase,
  shellCommon,
  slotEnvPrelude,
  skipPolicy,
  serviceConfigLib,
}:
let
  probeCommands = import ../../framework/runtime/helpers/probe-commands.nix { inherit pkgs; };
  postgresProbePkg = if pkgs ? postgresql_16 then pkgs.postgresql_16 else pkgs.postgresql;
  probePlanRuntime = import ../../framework/runtime/helpers/probe-plan-runtime.nix {
    inherit
      lib
      pkgs
      probeCommands
      postgresProbePkg
      ;
  };

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

  resolvedServiceConfigByName = builtins.listToAttrs (
    map (serviceName: {
      name = serviceName;
      value = serviceConfigLib.normalizeServiceConfig {
        name = serviceName;
        config = services.${serviceName};
      };
    }) serviceNames
  );

  serviceEnabledByName = builtins.listToAttrs (
    map (serviceName: {
      name = serviceName;
      value = services.${serviceName}.enable or false;
    }) serviceNames
  );

  enabledServiceNames = builtins.filter (
    serviceName: serviceEnabledByName.${serviceName}
  ) serviceNames;

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

  resolveServicePortBase =
    serviceName: endpointName:
    resolvePortBase
      resolvedServiceConfigByName.${serviceName}.resolved.endpoints.${endpointName}.portKey;

  mkServiceProbeSpec =
    mode: serviceName:
    let
      plan =
        resolvedServiceConfigByName.${serviceName}.resolved.probePlans.${mode}
          or resolvedServiceConfigByName.${serviceName}.resolved.operationProbes.${mode} or {
            count = 0;
            steps = [ ];
            wait = { };
          };
    in
    {
      count = if plan ? count then plan.count else builtins.length (plan.steps or [ ]);
      body = ''
        service_source="$(resolve_service_source "${serviceName}")"
        if [ -z "$service_source" ]; then
          service_source="unspecified"
        fi
        ${probePlanRuntime.renderPlanBody {
          inherit
            mode
            serviceName
            plan
            ;
          endpoints = resolvedServiceConfigByName.${serviceName}.resolved.endpoints or { };
          portExprForEndpoint =
            endpointName:
            let
              portBase = resolveServicePortBase serviceName endpointName;
            in
            "$(( ${toString portBase} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))";
        }}
      '';
    };

  mkProbeScript =
    {
      mode,
      emptyMessage,
      successMessage,
    }:
    let
      serviceSelectionPrelude = ''
            target_service="all"
            target_source=""

            while [ "$#" -gt 0 ]; do
              case "$1" in
                --service)
                  target_service="$(nixfied_require_next_arg --service "a value" "$@")"
                  shift 2
                  ;;
                --service=*)
                  target_service="''${1#--service=}"
                  shift
                  ;;
                --source)
                  target_source="$(nixfied_require_next_arg --source "a value" "$@")"
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
                  nixfied_unknown_arg "$1"
                  ;;
              esac
            done

            nixfied_unexpected_positional_args "$@"

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

            ${skipPolicy.skipPolicyFunctions}
            service_skip_env_var_name() { workflow_service_skip_env_var "$@"; }
            service_is_skipped() { is_service_skipped "$@"; }

            filter_skipped_services() {
              local -a filtered_services=()
              local candidate_service

              for candidate_service in "''${selected_services[@]}"; do
                if service_is_skipped "$candidate_service"; then
                  continue
                fi
                filtered_services+=("$candidate_service")
              done
              selected_services=("''${filtered_services[@]}")
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
              nixfied_exit_usage "unknown --service '$target_service'"
            fi

            workflow_unit_closure_services_csv="''${NIXFIED_SELECTED_SERVICES_CSV:-}"
            if [ "$target_service" = "all" ] && [ -n "$workflow_unit_closure_services_csv" ]; then
              selected_services=()
              IFS=, read -r -a selected_services <<< "$workflow_unit_closure_services_csv"
            elif [ "$target_service" = "all" ]; then
              ${enabledServiceArrayInit}
            else
              if [ "$(is_service_enabled "$target_service")" != "1" ]; then
                nixfied_exit_precondition "selected service '$target_service' is disabled"
              fi
              selected_services=("$target_service")
            fi
            filter_skipped_services

            if [ -z "''${NIXFIED_PARENT_WORKFLOW_ID:-}" ] && [ -n "$target_source" ]; then
              if [ "$target_service" = "all" ]; then
                nixfied_exit_usage "--source requires --service"
              fi
              if ! service_has_source "$target_service" "$target_source"; then
                nixfied_exit_precondition "unknown source '$target_source' for service '$target_service'"
              fi
            fi
      '';

      mkServiceProbeSection =
        serviceName: spec:
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

      serviceSpecs = builtins.listToAttrs (
        map (serviceName: {
          name = serviceName;
          value = mkServiceProbeSpec mode serviceName;
        }) serviceNames
      );
    in
    ''
      set -euo pipefail
      ${shellCommon}
      ${slotEnvPrelude}
      ${serviceSelectionPrelude}

      if [ "$target_service" = "all" ] && [ ${toString (builtins.length enabledServiceNames)} -eq 0 ]; then
        echo ${lib.escapeShellArg emptyMessage}
        exit 0
      fi

      checks=0

      ${builtins.concatStringsSep "\n\n" (
        map (serviceName: mkServiceProbeSection serviceName serviceSpecs.${serviceName}) serviceNames
      )}

      if [ "$checks" -eq 0 ]; then
        echo ${lib.escapeShellArg emptyMessage}
        exit 0
      fi

      printf '%s services=%s\n' ${lib.escapeShellArg successMessage} "$checks"
    '';
in
{
  inherit
    mkProbeScript
    serviceSelectionContractArgs
    ;
}
