{
  pkgs,
}:

let
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
  tcpOpenCmd =
    {
      portExpr,
      host ? "127.0.0.1",
    }:
    ''
      ${netcatPkg}/bin/nc -z ${host} "${portExpr}" >/dev/null 2>&1
    '';

  httpGetOkCmd =
    {
      urlExpr,
      maxTime ? 2,
    }:
    ''
      ${pkgs.curl}/bin/curl -fsS --max-time ${toString maxTime} "${urlExpr}" >/dev/null 2>&1
    '';

  jsonRpcRequestCmd =
    {
      urlExpr,
      method,
      params ? [ ],
      maxTime ? 2,
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
      maxTime ? 2,
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
      maxTime ? 2,
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
      host ? "localhost",
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
      host ? "localhost",
      user ? "postgres",
      extraArgs ? "-Atqc",
    }:
    ''
      ${postgres}/bin/psql -h ${host} -p "${portExpr}" -U ${user} -d "${databaseExpr}" ${extraArgs} ${pkgs.lib.escapeShellArg query}
    '';
}
