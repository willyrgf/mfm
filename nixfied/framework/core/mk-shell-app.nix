{ pkgs }:
{
  appName,
  body,
  binPrefix ? "nixfied",
}:
let
  suffix = builtins.substring 0 10 (builtins.hashString "sha256" appName);
  binName = "${binPrefix}-${suffix}";
  script = pkgs.writeShellScriptBin binName ''
    set -euo pipefail
    ${body}
  '';
in
{
  type = "app";
  program = "${script}/bin/${binName}";
}
