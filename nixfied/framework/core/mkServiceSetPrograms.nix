{
  pkgs,
  model,
  serviceSet,
  resolvedServices,
  serviceRuntimeSurfaces,
  operations ? null,
}:
let
  lib = pkgs.lib;
  shellCommon = import ./shell-common.nix { inherit pkgs; };
  skipPolicy = import ../runtime/helpers/skip-policy.nix { inherit pkgs; };
  serviceConfigLib = import ./service-config.nix {
    inherit
      lib
      pkgs
      ;
  };
  requiredServices = serviceSet.services.required or [ ];
  optionalHealthServices = serviceSet.services.optional or [ ];
  availableRuntimeServiceNames = builtins.sort builtins.lessThan (
    builtins.attrNames (serviceRuntimeSurfaces.serviceApis or { })
  );
  runtimeRequiredServices = builtins.filter (
    serviceName: builtins.elem serviceName availableRuntimeServiceNames
  ) requiredServices;
  runtimeOptionalHealthServices = builtins.filter (
    serviceName: builtins.elem serviceName availableRuntimeServiceNames
  ) optionalHealthServices;
  healthServices = builtins.sort builtins.lessThan (
    lib.unique (runtimeRequiredServices ++ runtimeOptionalHealthServices)
  );
  enabledOperations =
    if operations == null then
      [
        "start"
        "stop"
        "status"
        "health"
        "ready"
        "export"
      ]
    else
      builtins.sort builtins.lessThan (lib.unique operations);
  normalizeToken =
    value: lib.toUpper (lib.replaceStrings [ "." "-" ":" "/" " " ] [ "_" "_" "_" "_" "_" ] value);

  slotEnvPrelude = ''
    slot_var=${lib.escapeShellArg model.runtime.slot.var}
    env_var=${lib.escapeShellArg model.runtime.env.var}
    slot_default=${toString model.runtime.slot.default}
    env_default=${lib.escapeShellArg model.runtime.env.default}

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
      nixfied_exit_precondition "$slot_var must be an integer"
    fi

    case "$env_value" in
      ${builtins.concatStringsSep "\n" (
        map (
          envName: "    ${envName}) env_offset=${toString (model.runtime.env.offsets.${envName} or 0)} ;;"
        ) (model.runtime.env.names or [ model.runtime.env.default ])
      )}
      *)
        nixfied_exit_precondition "unsupported $env_var '$env_value'"
        ;;
    esac
  '';

  resolvePortBase =
    key:
    if builtins.hasAttr key model.runtime.ports then
      model.runtime.ports.${key}
    else
      throw "mkServiceSetPrograms: port key '${key}' is not defined in model.runtime.ports";

  probeRuntimeFor =
    memberNames:
    import ../../modules/operations/probes.nix {
      inherit
        lib
        pkgs
        skipPolicy
        serviceConfigLib
        resolvePortBase
        slotEnvPrelude
        shellCommon
        ;
      runtime = model.runtime;
      services = lib.getAttrs memberNames resolvedServices;
      serviceNames = memberNames;
    };

  healthProbeRuntime = probeRuntimeFor healthServices;
  readyProbeRuntime = probeRuntimeFor runtimeRequiredServices;

  renderStringCases =
    values:
    builtins.concatStringsSep "\n" (
      map (value: "      ${lib.escapeShellArg value}) return 0 ;;") values
    );

  renderServiceProgramCases =
    operation: members:
    builtins.concatStringsSep "\n" (
      map (
        serviceName:
        let
          appName = "svc::${serviceName}::${operation}";
          program =
            if builtins.hasAttr appName serviceRuntimeSurfaces.serviceApps then
              serviceRuntimeSurfaces.serviceApps.${appName}.program
            else
              "";
        in
        ''
          ${lib.escapeShellArg serviceName})
            printf '%s' ${lib.escapeShellArg program}
            return 0
            ;;
        ''
      ) members
    );

  renderServiceArtifactCases =
    artifactKey: members:
    builtins.concatStringsSep "\n" (
      map (
        serviceName:
        let
          artifacts = serviceRuntimeSurfaces.serviceApis.${serviceName}.artifacts or { };
          value = if builtins.hasAttr artifactKey artifacts then toString artifacts.${artifactKey} else "";
        in
        ''
          ${lib.escapeShellArg serviceName})
            printf '%s' ${lib.escapeShellArg value}
            return 0
            ;;
        ''
      ) members
    );

  renderExportRecordLines =
    members:
    builtins.concatStringsSep "\n" (
      map (
        serviceName:
        let
          api = serviceRuntimeSurfaces.serviceApis.${serviceName};
        in
        ''
          printf '%s\n' ${
            lib.escapeShellArg (
              builtins.toJSON {
                service = serviceName;
                required = builtins.elem serviceName requiredServices;
                artifacts = api.artifacts or { };
                operations = builtins.sort builtins.lessThan (builtins.attrNames (api.operations or { }));
              }
            )
          }
        ''
      ) members
    );

  renderOperationHelp =
    {
      appName,
      operation,
      allowSource ? false,
      allowFormat ? false,
      members,
    }:
    let
      baseOptions = [
        "  --service <name|all>: Select one service in the set or 'all' (default)."
      ]
      ++ lib.optionals allowSource [
        "  --source <key>: Override source for the selected service (requires --service)."
      ]
      ++ lib.optionals allowFormat [
        "  --format <json|env>: Choose export format."
      ]
      ++ [ "  -h, --help: Show this help." ];
    in
    builtins.concatStringsSep "\n" (
      [
        "${appName} - ${serviceSet.summary} ${operation}"
        ""
        "Usage:"
        "  nix run .#${appName} -- [options]"
        ""
        "Members:"
        "  ${if members == [ ] then "none" else builtins.concatStringsSep ", " members}"
        ""
        "Options:"
      ]
      ++ baseOptions
    );

  mkMemberSelectorPrelude =
    {
      members,
      allowSource ? false,
      allowFormat ? false,
      defaultFormat ? "json",
    }:
    let
      memberCases = renderStringCases members;
      memberArrayInit =
        if members == [ ] then
          "set_members=()"
        else
          "set_members=(" + builtins.concatStringsSep " " (map lib.escapeShellArg members) + ")";
    in
    ''
      target_service="all"
      target_source=""
      output_format=${lib.escapeShellArg defaultFormat}

      is_known_member() {
        case "$1" in
      ${memberCases}
          *) return 1 ;;
        esac
      }

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
          ${lib.optionalString allowSource ''
            --source)
              target_source="$(nixfied_require_next_arg --source "a value" "$@")"
              shift 2
              ;;
            --source=*)
              target_source="''${1#--source=}"
              shift
              ;;
          ''}
          ${lib.optionalString allowFormat ''
            --format)
              output_format="$(nixfied_require_next_arg --format "json|env" "$@")"
              shift 2
              ;;
            --format=*)
              output_format="''${1#--format=}"
              shift
              ;;
          ''}
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

      ${memberArrayInit}
      selected_services=()

      if [ "$target_service" = "all" ] && [ -n "''${NIXFIED_SELECTED_SERVICES_CSV:-}" ]; then
        IFS=, read -r -a selected_services <<< "''${NIXFIED_SELECTED_SERVICES_CSV}"
      elif [ "$target_service" = "all" ]; then
        selected_services=("''${set_members[@]}")
      else
        if ! is_known_member "$target_service"; then
          nixfied_exit_usage "unknown --service '$target_service'"
        fi
        selected_services=("$target_service")
      fi

      filtered_services=()
      for service_name in "''${selected_services[@]}"; do
        if ! is_known_member "$service_name"; then
          continue
        fi
        filtered_services+=("$service_name")
      done
      selected_services=("''${filtered_services[@]}")

      if [ ${toString (builtins.length members)} -gt 0 ] && [ "''${#selected_services[@]}" -eq 0 ]; then
        echo "SKIP: no services selected for service set ${serviceSet.name}"
        exit 0
      fi
    '';

  groupedControlScript =
    {
      operation,
      appName,
      members,
    }:
    let
      scriptName = "nixfied-service-set-${normalizeToken serviceSet.name}-${operation}";
      drv = pkgs.writeShellScriptBin scriptName ''
              set -euo pipefail
              ${shellCommon}
              ${skipPolicy.skipPolicyFunctions}

              render_usage() {
                cat <<'NIXFIED_USAGE'
        ${renderOperationHelp { inherit appName operation members; }}
        NIXFIED_USAGE
              }

              if [ "$#" -gt 0 ]; then
                case "$1" in
                  --help|-h)
                    render_usage
                    exit 0
                    ;;
                esac
              fi

              ${mkMemberSelectorPrelude {
                inherit members;
              }}

              service_program() {
                case "$1" in
              ${renderServiceProgramCases operation members}
                  *)
                    printf '%s' ""
                    return 1
                    ;;
                esac
              }

              service_log_expr() {
                case "$1" in
              ${renderServiceArtifactCases "logFile" members}
                  *)
                    printf '%s' ""
                    return 1
                    ;;
                esac
              }

              resolve_expr_value() {
                local expr="$1"
                local resolved=""
                if [ -z "$expr" ]; then
                  printf '%s' ""
                  return 0
                fi
                eval "resolved=$expr"
                printf '%s' "$resolved"
              }

              failure_count=0

              for service_name in "''${selected_services[@]}"; do
                service_program_path="$(service_program "$service_name")"
                if [ -z "$service_program_path" ]; then
                  echo "ERROR: service '$service_name' does not expose '${operation}'" >&2
                  failure_count=$((failure_count + 1))
                  continue
                fi

                output_file="$TMPDIR/service-set-${serviceSet.name}-${operation}-''${service_name}.log"
                if "$service_program_path" >"$output_file" 2>&1; then
                  cat "$output_file"
                else
                  rc="$?"
                  cat "$output_file" >&2 || true
                  if [ ${if serviceSet.failureLogs.capture then "1" else "0"} = "1" ]; then
                    log_expr="$(service_log_expr "$service_name" || true)"
                    log_path="$(resolve_expr_value "$log_expr")"
                    if [ -n "$log_path" ] && [ -f "$log_path" ]; then
                      echo "ERROR: service-set ${serviceSet.name} ${operation} failed service=$service_name log=$log_path" >&2
                      ${pkgs.coreutils}/bin/tail -n ${toString serviceSet.failureLogs.tailLines} "$log_path" >&2 || true
                    fi
                  fi
                  echo "ERROR: service-set ${serviceSet.name} ${operation} failed service=$service_name rc=$rc" >&2
                  failure_count=$((failure_count + 1))
                fi
              done

              if [ "$failure_count" -gt 0 ]; then
                exit 1
              fi

              echo "OK: service-set ${serviceSet.name} ${operation} passed services=''${#selected_services[@]}"
      '';
    in
    {
      inherit drv;
      program = "${drv}/bin/${scriptName}";
    };

  exportScript =
    appName:
    let
      exportLines = renderExportRecordLines runtimeRequiredServices;
      scriptName = "nixfied-service-set-${normalizeToken serviceSet.name}-export";
      drv = pkgs.writeShellScriptBin scriptName ''
              set -euo pipefail
              ${shellCommon}
              source <(${serviceRuntimeSurfaces.slots.getSlotInfo})

              render_usage() {
                cat <<'NIXFIED_USAGE'
        ${renderOperationHelp {
          inherit appName;
          operation = "export";
          members = runtimeRequiredServices;
          allowFormat = true;
        }}
        NIXFIED_USAGE
              }

              if [ "$#" -gt 0 ]; then
                case "$1" in
                  --help|-h)
                    render_usage
                    exit 0
                    ;;
                esac
              fi

              ${mkMemberSelectorPrelude {
                members = runtimeRequiredServices;
                allowFormat = true;
                defaultFormat = serviceSet.export.defaultFormat;
              }}

              emit_records() {
                :
        ${exportLines}
              }

              resolve_artifact_value() {
                local key="$1"
                local raw="$2"
                local resolved=""

                if [ -z "$raw" ]; then
                  printf '%s' ""
                  return 0
                fi

                case "$key" in
                  *PortVar)
                    printf '%s' "''${!raw:-}"
                    ;;
                  *)
                    eval "resolved=$raw"
                    printf '%s' "$resolved"
                    ;;
                esac
              }

              selected_json="$(${pkgs.jq}/bin/jq -cn '[]')"
              for service_name in "''${selected_services[@]}"; do
                selected_json="$(${pkgs.jq}/bin/jq -cn --argjson current "$selected_json" --arg value "$service_name" '$current + [$value]')"
              done

              records_json="$(
                emit_records | ${pkgs.jq}/bin/jq -cs --argjson selected "$selected_json" '
                  map(select(.service as $service | $selected | index($service)))
                '
              )"

              resolved_records="$(${pkgs.jq}/bin/jq -cn '[]')"
              while IFS= read -r record_json; do
                [ -n "$record_json" ] || continue
                resolved_artifacts="$(${pkgs.jq}/bin/jq -cn '{}')"
                while IFS=$'\t' read -r artifact_key artifact_raw || [ -n "$artifact_key" ]; do
                  [ -n "$artifact_key" ] || continue
                  artifact_value="$(resolve_artifact_value "$artifact_key" "$artifact_raw")"
                  resolved_artifacts="$(
                    ${pkgs.jq}/bin/jq -cn \
                      --argjson current "$resolved_artifacts" \
                      --arg key "$artifact_key" \
                      --arg value "$artifact_value" \
                      '$current + {($key): $value}'
                  )"
                done < <(printf '%s' "$record_json" | ${pkgs.jq}/bin/jq -r '.artifacts | to_entries[]? | [.key, (.value | tostring)] | @tsv')

                resolved_record="$(
                  ${pkgs.jq}/bin/jq -cn \
                    --argjson record "$record_json" \
                    --argjson artifacts "$resolved_artifacts" \
                    --arg serviceSetId ${lib.escapeShellArg serviceSet.id} \
                    --arg policyId ${lib.escapeShellArg serviceSet.state.policy.id} \
                    --arg policyKind ${lib.escapeShellArg serviceSet.state.policy.kind} \
                    --arg runtimeBase ${lib.escapeShellArg serviceSet.state.policy.runtimeBase} \
                    --arg registryRoot ${lib.escapeShellArg serviceSet.state.policy.registryRoot} \
                    --arg artifactsRoot ${lib.escapeShellArg serviceSet.state.policy.artifactsRoot} \
                    '
                      $record
                      + {
                          serviceSetId: $serviceSetId,
                          statePolicy: {
                            id: $policyId,
                            kind: $policyKind,
                            runtimeBase: $runtimeBase,
                            registryRoot: $registryRoot,
                            artifactsRoot: $artifactsRoot
                          },
                          resolvedArtifacts: $artifacts
                        }
                    '
                )"
                resolved_records="$(${pkgs.jq}/bin/jq -cn --argjson current "$resolved_records" --argjson record "$resolved_record" '$current + [$record]')"
              done < <(printf '%s' "$records_json" | ${pkgs.jq}/bin/jq -c '.[]')

              case "$output_format" in
                json)
                  printf '%s\n' "$resolved_records"
                  ;;
                env)
                  printf 'NIXFIED_SERVICE_SET_ID=%s\n' ${lib.escapeShellArg serviceSet.id}
                  printf 'NIXFIED_SERVICE_SET_POLICY_ID=%s\n' ${lib.escapeShellArg serviceSet.state.policy.id}
                  while IFS= read -r record_json; do
                    [ -n "$record_json" ] || continue
                    service_name="$(printf '%s' "$record_json" | ${pkgs.jq}/bin/jq -r '.service')"
                    service_token="$(printf '%s' "$service_name" | ${pkgs.coreutils}/bin/tr '[:lower:].-:/ ' '[:upper:]______' | ${pkgs.coreutils}/bin/tr -c 'A-Z0-9_' '_')"
                    printf 'NIXFIED_SERVICE_SET_%s_SERVICE=%s\n' "$service_token" "$service_name"
                    while IFS=$'\t' read -r artifact_key artifact_value || [ -n "$artifact_key" ]; do
                      [ -n "$artifact_key" ] || continue
                      artifact_token="$(printf '%s' "$artifact_key" | ${pkgs.coreutils}/bin/tr '[:lower:].-:/ ' '[:upper:]______' | ${pkgs.coreutils}/bin/tr -c 'A-Z0-9_' '_')"
                      printf 'NIXFIED_SERVICE_SET_%s_%s=%s\n' "$service_token" "$artifact_token" "$artifact_value"
                    done < <(printf '%s' "$record_json" | ${pkgs.jq}/bin/jq -r '.resolvedArtifacts | to_entries[]? | [.key, (.value | tostring)] | @tsv')
                  done < <(printf '%s' "$resolved_records" | ${pkgs.jq}/bin/jq -c '.[]')
                  ;;
                *)
                  nixfied_exit_usage "--format must be json or env"
                  ;;
              esac
      '';
    in
    {
      inherit drv;
      program = "${drv}/bin/${scriptName}";
    };
in
{
  programsByOperation =
    lib.optionalAttrs (builtins.elem "start" enabledOperations) {
      start = groupedControlScript {
        operation = "start";
        appName = "svcset::${serviceSet.name}::start";
        members = runtimeRequiredServices;
      };
    }
    // lib.optionalAttrs (builtins.elem "stop" enabledOperations) {
      stop = groupedControlScript {
        operation = "stop";
        appName = "svcset::${serviceSet.name}::stop";
        members = runtimeRequiredServices;
      };
    }
    // lib.optionalAttrs (builtins.elem "status" enabledOperations) {
      status = groupedControlScript {
        operation = "status";
        appName = "svcset::${serviceSet.name}::status";
        members = runtimeRequiredServices;
      };
    }
    // lib.optionalAttrs (builtins.elem "health" enabledOperations) {
      health =
        let
          scriptName = "nixfied-service-set-${normalizeToken serviceSet.name}-health";
          drv = pkgs.writeShellScriptBin scriptName ''
                  if [ "$#" -gt 0 ] && { [ "$1" = "--help" ] || [ "$1" = "-h" ]; }; then
                    cat <<'NIXFIED_USAGE'
            ${renderOperationHelp {
              appName = "svcset::${serviceSet.name}::health";
              operation = "health";
              members = healthServices;
              allowSource = true;
            }}
            NIXFIED_USAGE
                    exit 0
                  fi
                  ${healthProbeRuntime.mkProbeScript {
                    mode = "health";
                    emptyMessage = "SKIP: no enabled services for health checks";
                    successMessage = "OK: service-set ${serviceSet.name} health checks passed";
                  }}
          '';
        in
        {
          inherit drv;
          program = "${drv}/bin/${scriptName}";
        };
    }
    // lib.optionalAttrs (builtins.elem "ready" enabledOperations) {
      ready =
        let
          scriptName = "nixfied-service-set-${normalizeToken serviceSet.name}-ready";
          drv = pkgs.writeShellScriptBin scriptName ''
                  if [ "$#" -gt 0 ] && { [ "$1" = "--help" ] || [ "$1" = "-h" ]; }; then
                    cat <<'NIXFIED_USAGE'
            ${renderOperationHelp {
              appName = "svcset::${serviceSet.name}::ready";
              operation = "ready";
              members = runtimeRequiredServices;
              allowSource = true;
            }}
            NIXFIED_USAGE
                    exit 0
                  fi
                  ${readyProbeRuntime.mkProbeScript {
                    mode = "ready";
                    emptyMessage = "SKIP: no enabled services for readiness checks";
                    successMessage = "OK: service-set ${serviceSet.name} readiness checks passed";
                  }}
          '';
        in
        {
          inherit drv;
          program = "${drv}/bin/${scriptName}";
        };
    }
    // lib.optionalAttrs (builtins.elem "export" enabledOperations) {
      export = exportScript "svcset::${serviceSet.name}::export";
    };
}
