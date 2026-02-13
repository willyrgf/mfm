# Shared ID helpers
{ pkgs }:

let
  mkUniqueId = pkgs.writeShellScript "mk-unique-id" ''
    set -euo pipefail
    echo "$(${pkgs.coreutils}/bin/date +%Y%m%d-%H%M%S)-$(head -c 4 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  '';

  resolveId = pkgs.writeShellScript "resolve-id" ''
    set -euo pipefail
    VALUE="''${1:-}"

    if [ -z "$VALUE" ]; then
      exec ${mkUniqueId}
    fi

    case "$VALUE" in
      *[!A-Za-z0-9._:-]*)
        echo "ERROR: id contains invalid characters value=$VALUE" >&2
        echo "HINT: use only [A-Za-z0-9._:-]" >&2
        exit 1
        ;;
      *)
        echo "$VALUE"
        ;;
    esac
  '';
in
{
  inherit
    mkUniqueId
    resolveId
    ;
}
