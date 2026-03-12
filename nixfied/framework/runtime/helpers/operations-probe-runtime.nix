{
  lib,
  pkgs,
  probeCommands,
  postgresProbePkg,
  runtimeStride,
  resolveServicePortBase,
}:

let
  portValueExpr =
    base: "$(( ${toString base} + env_offset + (slot_value * ${toString runtimeStride}) ))";
  shellVar = name: "$" + name;
  probePortVar = serviceName: endpointName: "${serviceName}_${endpointName}_port";

  renderSourceKindCase =
    sourceKinds:
    builtins.concatStringsSep "\n" (
      map (
        sourceName:
        "        ${lib.escapeShellArg sourceName}) helios_source_kind_value=${
                  lib.escapeShellArg (sourceKinds.${sourceName} or "unknown")
                } ;;"
      ) (builtins.sort builtins.lessThan (builtins.attrNames sourceKinds))
    );

  mkTcpProbeBody =
    {
      serviceLabel,
      phaseLabel,
      successLabel,
      failureLabel,
      portVar,
      portBase,
    }:
    ''
      ${portVar}=${portValueExpr portBase}
      echo "INFO: checking ${serviceLabel} ${phaseLabel} port=${shellVar portVar} source=$service_source"
      if ${
        probeCommands.tcpOpenCmd {
          portExpr = shellVar portVar;
        }
      } then
        echo "OK: ${serviceLabel} ${successLabel} port=${shellVar portVar}"
      else
        echo "ERROR: ${serviceLabel} ${failureLabel} port=${shellVar portVar}"
        exit 1
      fi
    '';

  mkPostgresPgIsReadyBody =
    {
      serviceLabel,
      phaseLabel,
      successLabel,
      failureLabel,
      host,
      failureSuffix ? "",
      portVar,
      portBase,
    }:
    ''
      ${portVar}=${portValueExpr portBase}
      echo "INFO: checking ${serviceLabel} ${phaseLabel} port=${shellVar portVar} source=$service_source"
      if ${
        probeCommands.pgIsReadyCmd {
          postgres = postgresProbePkg;
          inherit host;
          portExpr = shellVar portVar;
        }
      } then
        echo "OK: ${serviceLabel} ${successLabel} port=${shellVar portVar}"
      else
        echo "ERROR: ${serviceLabel} ${failureLabel} port=${shellVar portVar}${failureSuffix}"
        exit 1
      fi
    '';

  mkPostgresQueryBody =
    {
      serviceLabel,
      phaseLabel,
      successLabel,
      failureLabel,
      host,
      database,
      query,
      failureSuffix ? "",
      portVar,
      portBase,
    }:
    ''
      ${portVar}=${portValueExpr portBase}
      echo "INFO: checking ${serviceLabel} ${phaseLabel} port=${shellVar portVar} source=$service_source"
      if ${
        probeCommands.psqlQueryCmd {
          postgres = postgresProbePkg;
          inherit host query;
          portExpr = shellVar portVar;
          databaseExpr = database;
        }
      } then
        echo "OK: ${serviceLabel} ${successLabel} port=${shellVar portVar}"
      else
        echo "ERROR: ${serviceLabel} ${failureLabel} port=${shellVar portVar}${failureSuffix}"
        exit 1
      fi
    '';

  mkJsonRpcProbeBody =
    {
      serviceLabel,
      phaseLabel,
      successLabel,
      failureLabel,
      method,
      portVar,
      portBase,
    }:
    ''
      ${portVar}=${portValueExpr portBase}
      echo "INFO: checking ${serviceLabel} ${phaseLabel} port=${shellVar portVar} source=$service_source"
      if ${
        probeCommands.jsonRpcHasResultCmd {
          urlExpr = "http://127.0.0.1:${shellVar portVar}";
          inherit method;
        }
      } then
        echo "OK: ${serviceLabel} ${successLabel} port=${shellVar portVar}"
      else
        echo "ERROR: ${serviceLabel} ${failureLabel} port=${shellVar portVar}"
        exit 1
      fi
    '';

  mkHeliosReadyBody =
    {
      portVar,
      portBase,
      sourceKinds,
      readinessProfile,
      requireNotSyncing,
      disallowSourceKinds,
    }:
    let
      sourceKindCase = renderSourceKindCase sourceKinds;
      disallowArgs = builtins.concatStringsSep " " (map lib.escapeShellArg disallowSourceKinds);
    in
    ''
            ${portVar}=${portValueExpr portBase}
            helios_source_kind_value="unknown"
            case "$service_source" in
      ${sourceKindCase}
              *)
                helios_source_kind_value="unknown"
                ;;
            esac
            helios_readiness_profile=${lib.escapeShellArg readinessProfile}
            helios_require_not_syncing=${if requireNotSyncing then "1" else "0"}
            echo "INFO: checking helios readiness port=${shellVar portVar} source=$service_source source_kind=$helios_source_kind_value profile=$helios_readiness_profile"
            if source_kind_disallowed "$helios_source_kind_value"${
              if disallowArgs == "" then "" else " " + disallowArgs
            }; then
              echo "ERROR: helios not ready port=${shellVar portVar} source=$service_source source_kind=$helios_source_kind_value profile=$helios_readiness_profile (source kind disallowed)"
              exit 1
            fi

            helios_block_json="$(${
              probeCommands.jsonRpcRequestCmd {
                urlExpr = "http://127.0.0.1:${shellVar portVar}";
                method = "eth_blockNumber";
              }
            })" || true
            helios_block_number="$(printf '%s' "$helios_block_json" | ${pkgs.jq}/bin/jq -r '.result // empty')" || true
            if [ -z "$helios_block_number" ] || ! [[ "$helios_block_number" =~ ^0x[0-9a-fA-F]+$ ]]; then
              echo "ERROR: helios not ready port=${shellVar portVar} source=$service_source source_kind=$helios_source_kind_value (invalid eth_blockNumber result)"
              exit 1
            fi
            echo "OK: helios ready port=${shellVar portVar} block_number=$helios_block_number"

            if [ "$helios_require_not_syncing" = "1" ]; then
              helios_syncing_result="$(${
                probeCommands.jsonRpcFieldCmd {
                  urlExpr = "http://127.0.0.1:${shellVar portVar}";
                  method = "eth_syncing";
                  jqExpr = ".result";
                  raw = false;
                }
              })" || true
              if [ "$helios_syncing_result" != "false" ]; then
                echo "ERROR: helios not ready port=${shellVar portVar} source=$service_source source_kind=$helios_source_kind_value profile=$helios_readiness_profile (eth_syncing=$helios_syncing_result)"
                exit 1
              fi
              echo "OK: helios sync status ready port=${shellVar portVar}"
            else
              echo "SKIP: helios sync gate disabled profile=$helios_readiness_profile"
            fi
    '';

  renderProbeStep =
    mode: serviceName: step:
    let
      portVar = probePortVar serviceName step.endpoint;
      portBase = resolveServicePortBase serviceName step.endpoint;
    in
    if step.kind == "tcp" then
      mkTcpProbeBody {
        inherit
          portVar
          portBase
          ;
        serviceLabel = step.serviceLabel;
        phaseLabel = step.phaseLabel;
        successLabel = step.successLabel;
        failureLabel = step.failureLabel;
      }
    else if step.kind == "jsonrpc" then
      mkJsonRpcProbeBody {
        inherit
          portVar
          portBase
          ;
        serviceLabel = step.serviceLabel;
        phaseLabel = step.phaseLabel;
        successLabel = step.successLabel;
        failureLabel = step.failureLabel;
        method = step.method;
      }
    else if step.kind == "postgres-pg-isready" then
      mkPostgresPgIsReadyBody {
        inherit
          portVar
          portBase
          ;
        serviceLabel = step.serviceLabel;
        phaseLabel = step.phaseLabel;
        successLabel = step.successLabel;
        failureLabel = step.failureLabel;
        host = step.host or "127.0.0.1";
        failureSuffix = step.failureSuffix or "";
      }
    else if step.kind == "postgres-query" then
      mkPostgresQueryBody {
        inherit
          portVar
          portBase
          ;
        serviceLabel = step.serviceLabel;
        phaseLabel = step.phaseLabel;
        successLabel = step.successLabel;
        failureLabel = step.failureLabel;
        host = step.host or "127.0.0.1";
        database = step.database;
        query = step.query;
        failureSuffix = step.failureSuffix or "";
      }
    else if step.kind == "helios-ready" then
      mkHeliosReadyBody {
        inherit
          portVar
          portBase
          ;
        sourceKinds = step.sourceKinds or { };
        readinessProfile = step.readinessProfile or "fast";
        requireNotSyncing = step.requireNotSyncing or false;
        disallowSourceKinds = step.disallowSourceKinds or [ ];
      }
    else
      throw "nixfied.operations: unsupported probe kind '${step.kind}' for service '${serviceName}' mode '${mode}'";
in
{
  inherit
    mkTcpProbeBody
    mkPostgresPgIsReadyBody
    mkPostgresQueryBody
    mkJsonRpcProbeBody
    mkHeliosReadyBody
    renderProbeStep
    ;
}
