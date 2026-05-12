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
