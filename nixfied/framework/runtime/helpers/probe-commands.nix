{
  pkgs,
}:

let
  runtimeDefaults = import ../../core/runtime-defaults.nix;

  netcatPkg =
    if pkgs ? netcat then
      pkgs.netcat
    else if pkgs ? netcat-openbsd then
      pkgs.netcat-openbsd
    else
      throw "probe-commands: netcat package is required";

  jsonRpcPayload =
    {
      method,
      params ? [ ],
    }:
    builtins.toJSON {
      jsonrpc = "2.0";
      id = 1;
      inherit
        method
        params
        ;
    };
in
rec {
  endpointUrlExpr =
    {
      portExpr,
      scheme ? "http",
      host ? runtimeDefaults.hosts.loopbackIp,
      path ? "",
    }:
    "${scheme}://${host}:${portExpr}${path}";

  localHttpUrlExpr = portExpr: endpointUrlExpr { inherit portExpr; };

  tcpOpenCmd =
    {
      portExpr,
      host ? runtimeDefaults.hosts.loopbackIp,
    }:
    ''
      ${netcatPkg}/bin/nc -z ${host} "${portExpr}" >/dev/null 2>&1
    '';

  httpGetOkCmd =
    {
      urlExpr,
      maxTime ? runtimeDefaults.probes.httpMaxTimeSeconds,
    }:
    ''
      ${pkgs.curl}/bin/curl -fsS --max-time ${toString maxTime} "${urlExpr}" >/dev/null 2>&1
    '';

  jsonRpcRequestCmd =
    {
      urlExpr,
      method,
      params ? [ ],
      maxTime ? runtimeDefaults.probes.httpMaxTimeSeconds,
    }:
    ''
      ${pkgs.curl}/bin/curl -fsS --max-time ${toString maxTime} \
        -H 'content-type: application/json' \
        --data '${jsonRpcPayload { inherit method params; }}' \
        "${urlExpr}"
    '';

  jsonRpcHasResultCmd =
    {
      urlExpr,
      method,
      params ? [ ],
      maxTime ? runtimeDefaults.probes.httpMaxTimeSeconds,
    }:
    ''
      (
        ${jsonRpcRequestCmd {
          inherit
            urlExpr
            method
            params
            maxTime
            ;
        }}
      ) | ${pkgs.gnugrep}/bin/grep -q '"result"'
    '';

  jsonRpcFieldCmd =
    {
      urlExpr,
      method,
      params ? [ ],
      jqExpr ? ".result // empty",
      raw ? true,
      maxTime ? runtimeDefaults.probes.httpMaxTimeSeconds,
    }:
    ''
      (
        ${jsonRpcRequestCmd {
          inherit
            urlExpr
            method
            params
            maxTime
            ;
        }}
      ) | ${pkgs.jq}/bin/jq -${if raw then "r" else "c"} '${jqExpr}'
    '';

  pgIsReadyCmd =
    {
      postgres,
      portExpr,
      host ? runtimeDefaults.hosts.localhost,
      user ? "postgres",
      quiet ? true,
    }:
    ''
      ${postgres}/bin/pg_isready -U ${user} -h ${host} -p "${portExpr}" ${if quiet then "-q" else ""} 2>/dev/null
    '';

  psqlQueryCmd =
    {
      postgres,
      portExpr,
      databaseExpr,
      query,
      host ? runtimeDefaults.hosts.localhost,
      user ? "postgres",
      extraArgs ? "-Atqc",
    }:
    ''
      ${postgres}/bin/psql -h ${host} -p "${portExpr}" -U ${user} -d "${databaseExpr}" ${extraArgs} ${pkgs.lib.escapeShellArg query}
    '';
}
