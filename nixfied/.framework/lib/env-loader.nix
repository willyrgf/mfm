# Shared .env loading utilities.
{ pkgs }:

let
  loadEnvFile = pkgs.writeShellScript "nixfied-load-env-file" ''
    set -euo pipefail

    ENV_FILE="''${1:-.env}"
    if [ ! -f "$ENV_FILE" ]; then
      exit 0
    fi

    while IFS='=' read -r key value || [ -n "$key" ]; do
      case "$key" in
        \#*|"")
          continue
          ;;
      esac

      value=$(echo "$value" | sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")
      if [ -z "''${!key:-}" ]; then
        export "$key=$value"
      fi
    done < "$ENV_FILE"
  '';

  loadEnv = pkgs.writeShellScript "nixfied-load-env" ''
    set -euo pipefail
    ${loadEnvFile} ".env"
  '';
in
{
  inherit
    loadEnvFile
    loadEnv
    ;
}
