{
  lib,
  pkgs,
  probeCommands,
  postgresProbePkg,
}:

let
  runtimeDefaults = import ../../core/runtime-defaults.nix;

  normalizeEnvToken =
    value: lib.toUpper (lib.replaceStrings [ "-" "." ":" "/" " " ] [ "_" "_" "_" "_" "_" ] value);

  endpointProtocol =
    endpoints: endpointName:
    let
      endpoint = endpoints.${endpointName} or { };
      protocol = endpoint.protocol or "http";
    in
    if protocol == "https" then "https" else "http";

  mkExecEnvBlock =
    {
      mode,
      serviceName,
      endpoints,
      portExprForEndpoint,
    }:
    let
      endpointNames = builtins.sort builtins.lessThan (builtins.attrNames endpoints);
      endpointExports = builtins.concatStringsSep "\n" (
        map (
          endpointName:
          let
            envName = "NIXFIED_PROBE_${normalizeEnvToken endpointName}_PORT";
          in
          ''
            export ${envName}="${portExprForEndpoint endpointName}"
          ''
        ) endpointNames
      );
    in
    ''
      export NIXFIED_PROBE_MODE=${lib.escapeShellArg mode}
      export NIXFIED_PROBE_SERVICE=${lib.escapeShellArg serviceName}
      export NIXFIED_PROBE_SOURCE="''${service_source:-unspecified}"
      ${endpointExports}
    '';

  mkTcpProbeBody =
    {
      step,
      portExpr,
    }:
    ''
      probe_source="''${service_source:-unspecified}"
      echo "INFO: checking ${step.serviceLabel} ${step.phaseLabel} port=${portExpr} source=$probe_source"
      if ${
        probeCommands.tcpOpenCmd {
          portExpr = portExpr;
        }
      } then
        echo "OK: ${step.serviceLabel} ${step.successLabel} port=${portExpr}"
      else
        echo "ERROR: ${step.serviceLabel} ${step.failureLabel} port=${portExpr}"
        exit 1
      fi
    '';

  mkHttpProbeBody =
    {
      endpoints,
      step,
      portExpr,
    }:
    let
      scheme = endpointProtocol endpoints step.endpoint;
      urlExpr = probeCommands.endpointUrlExpr {
        inherit
          scheme
          portExpr
          ;
        path = step.path;
      };
    in
    ''
      probe_source="''${service_source:-unspecified}"
      echo "INFO: checking ${step.serviceLabel} ${step.phaseLabel} url=${urlExpr} source=$probe_source"
      if ${
        probeCommands.httpGetOkCmd {
          urlExpr = urlExpr;
        }
      } then
        echo "OK: ${step.serviceLabel} ${step.successLabel} url=${urlExpr}"
      else
        echo "ERROR: ${step.serviceLabel} ${step.failureLabel} url=${urlExpr}"
        exit 1
      fi
    '';

  mkPostgresPgIsReadyBody =
    {
      step,
      portExpr,
    }:
    ''
      probe_source="''${service_source:-unspecified}"
      echo "INFO: checking ${step.serviceLabel} ${step.phaseLabel} port=${portExpr} source=$probe_source"
      if ${
        probeCommands.pgIsReadyCmd {
          postgres = postgresProbePkg;
          host = step.host or runtimeDefaults.hosts.loopbackIp;
          portExpr = portExpr;
        }
      } then
        echo "OK: ${step.serviceLabel} ${step.successLabel} port=${portExpr}"
      else
        echo "ERROR: ${step.serviceLabel} ${step.failureLabel} port=${portExpr}${step.failureSuffix or ""}"
        exit 1
      fi
    '';

  mkPostgresQueryBody =
    {
      step,
      portExpr,
    }:
    ''
      probe_source="''${service_source:-unspecified}"
      echo "INFO: checking ${step.serviceLabel} ${step.phaseLabel} port=${portExpr} source=$probe_source"
      if ${
        probeCommands.psqlQueryCmd {
          postgres = postgresProbePkg;
          host = step.host or runtimeDefaults.hosts.loopbackIp;
          portExpr = portExpr;
          databaseExpr = step.database;
          query = step.query;
        }
      } >/dev/null 2>&1; then
        echo "OK: ${step.serviceLabel} ${step.successLabel} port=${portExpr}"
      else
        echo "ERROR: ${step.serviceLabel} ${step.failureLabel} port=${portExpr}${step.failureSuffix or ""}"
        exit 1
      fi
    '';

  mkJsonRpcProbeBody =
    {
      step,
      portExpr,
    }:
    let
      urlExpr = probeCommands.localHttpUrlExpr portExpr;
    in
    ''
      probe_source="''${service_source:-unspecified}"
      echo "INFO: checking ${step.serviceLabel} ${step.phaseLabel} port=${portExpr} source=$probe_source"
      if ${
        probeCommands.jsonRpcHasResultCmd {
          urlExpr = urlExpr;
          method = step.method;
        }
      } then
        echo "OK: ${step.serviceLabel} ${step.successLabel} port=${portExpr}"
      else
        echo "ERROR: ${step.serviceLabel} ${step.failureLabel} port=${portExpr}"
        exit 1
      fi
    '';

  mkHeliosReadyBody =
    {
      step,
      portExpr,
    }:
    let
      sourceKindCase = builtins.concatStringsSep "\n" (
        map (
          sourceName:
          "        ${lib.escapeShellArg sourceName}) helios_source_kind_value=${
                    lib.escapeShellArg (step.sourceKinds.${sourceName} or "unknown")
                  } ;;"
        ) (builtins.sort builtins.lessThan (builtins.attrNames (step.sourceKinds or { })))
      );
      disallowArgs = builtins.concatStringsSep " " (
        map lib.escapeShellArg (step.disallowSourceKinds or [ ])
      );
    in
    ''
            probe_source="''${service_source:-unspecified}"
            helios_source_kind_value="unknown"
            case "$probe_source" in
      ${sourceKindCase}
              *)
                helios_source_kind_value="unknown"
                ;;
            esac
            helios_readiness_profile=${lib.escapeShellArg (step.readinessProfile or "fast")}
            helios_require_not_syncing=${if step.requireNotSyncing or false then "1" else "0"}
            echo "INFO: checking ${step.serviceLabel} ${step.phaseLabel} port=${portExpr} source=$probe_source source_kind=$helios_source_kind_value profile=$helios_readiness_profile"
            if source_kind_disallowed "$helios_source_kind_value"${
              if disallowArgs == "" then "" else " " + disallowArgs
            }; then
              echo "ERROR: ${step.serviceLabel} ${step.failureLabel} port=${portExpr} source=$probe_source source_kind=$helios_source_kind_value profile=$helios_readiness_profile (source kind disallowed)"
              exit 1
            fi

            helios_block_json="$(${
              probeCommands.jsonRpcRequestCmd {
                urlExpr = probeCommands.localHttpUrlExpr portExpr;
                method = "eth_blockNumber";
              }
            })" || true
            helios_block_number="$(printf '%s' "$helios_block_json" | ${pkgs.jq}/bin/jq -r '.result // empty')" || true
            if [ -z "$helios_block_number" ] || ! [[ "$helios_block_number" =~ ^0x[0-9a-fA-F]+$ ]]; then
              if [ "${if step.allowLocalHealthFallback or false then "1" else "0"}" = "1" ] && ${
                probeCommands.jsonRpcHasResultCmd {
                  urlExpr = probeCommands.localHttpUrlExpr portExpr;
                  method = "eth_chainId";
                }
              }
              then
                echo "OK: ${step.serviceLabel} ${step.successLabel} port=${portExpr} mode=local_chainid_fallback"
                exit 0
              fi
              echo "ERROR: ${step.serviceLabel} ${step.failureLabel} port=${portExpr} source=$probe_source source_kind=$helios_source_kind_value (invalid eth_blockNumber result)"
              exit 1
            fi
            echo "OK: ${step.serviceLabel} ${step.successLabel} port=${portExpr} block_number=$helios_block_number"

            if [ "$helios_require_not_syncing" = "1" ]; then
              helios_syncing_result="$(${
                probeCommands.jsonRpcFieldCmd {
                  urlExpr = probeCommands.localHttpUrlExpr portExpr;
                  method = "eth_syncing";
                  fieldExpr = ".result";
                  raw = false;
                }
              })" || true
              if [ "$helios_syncing_result" != "false" ]; then
                echo "ERROR: ${step.serviceLabel} ${step.failureLabel} port=${portExpr} source=$probe_source source_kind=$helios_source_kind_value profile=$helios_readiness_profile (eth_syncing=$helios_syncing_result)"
                exit 1
              fi
              echo "OK: ${step.serviceLabel} sync status ready port=${portExpr}"
            else
              echo "SKIP: helios sync gate disabled profile=$helios_readiness_profile"
            fi
    '';

  mkExecProbeBody =
    {
      mode,
      serviceName,
      step,
      endpoints,
      portExprForEndpoint,
    }:
    ''
      ${mkExecEnvBlock {
        inherit
          mode
          serviceName
          endpoints
          portExprForEndpoint
          ;
      }}
      probe_source="''${service_source:-unspecified}"
      echo "INFO: checking ${step.serviceLabel} ${step.phaseLabel} source=$probe_source kind=exec"
      if ${pkgs.runtimeShell} -c ${lib.escapeShellArg step.command}; then
        echo "OK: ${step.serviceLabel} ${step.successLabel}"
      else
        echo "ERROR: ${step.serviceLabel} ${step.failureLabel}"
        exit 1
      fi
    '';

  renderProbeStep =
    {
      mode,
      serviceName,
      step,
      endpoints,
      portExprForEndpoint,
    }:
    let
      portExpr =
        if step ? endpoint && step.endpoint != null then portExprForEndpoint step.endpoint else null;
    in
    if step.kind == "tcp" then
      mkTcpProbeBody {
        inherit step portExpr;
      }
    else if step.kind == "http" then
      mkHttpProbeBody {
        inherit
          endpoints
          step
          portExpr
          ;
      }
    else if step.kind == "jsonrpc" then
      mkJsonRpcProbeBody {
        inherit step portExpr;
      }
    else if step.kind == "postgres-pg-isready" then
      mkPostgresPgIsReadyBody {
        inherit step portExpr;
      }
    else if step.kind == "postgres-query" then
      mkPostgresQueryBody {
        inherit step portExpr;
      }
    else if step.kind == "helios-ready" then
      mkHeliosReadyBody {
        inherit step portExpr;
      }
    else if step.kind == "exec" then
      mkExecProbeBody {
        inherit
          mode
          serviceName
          step
          endpoints
          portExprForEndpoint
          ;
      }
    else
      throw "probe-plan-runtime: unsupported probe kind '${step.kind}' for service '${serviceName}' mode '${mode}'";

  renderPlanBody =
    {
      mode,
      serviceName,
      plan,
      endpoints,
      portExprForEndpoint,
    }:
    builtins.concatStringsSep "\n" (
      map (
        step:
        renderProbeStep {
          inherit
            mode
            serviceName
            step
            endpoints
            portExprForEndpoint
            ;
        }
      ) (plan.steps or [ ])
    );
in
{
  inherit
    renderProbeStep
    renderPlanBody
    ;
}
