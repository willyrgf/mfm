{
  pkgs,
  lib,
}:
{
  name ? "nix-checks",
  flakeRef ? "path:.",
  formatterPkg ? (if pkgs ? nixfmt then pkgs.nixfmt else pkgs.nixfmt-rfc-style),
}:
let
  plainShellLogging = import ./plain-shell-logging.nix;
in
pkgs.writeShellScriptBin name ''
    set -euo pipefail

    mode="quick"
    flake_ref=${lib.escapeShellArg flakeRef}

    ${plainShellLogging {
      includeWarn = false;
      errorToStderr = true;
    }}

    usage() {
      cat <<'EOF'
  Usage: nix-checks [--mode <quick|full>] [--quick] [--full] [--flake <ref>] [--help]

  Modes:
    quick  Run nixfmt --check, nix flake show, and nix run .#help.
    full   Run quick mode plus nix flake check.
  EOF
    }

    run_nixfmt_check() {
      local -a nix_files

      mapfile -t nix_files < <(
        ${pkgs.findutils}/bin/find . -type f -name '*.nix' | ${pkgs.coreutils}/bin/sort
      )

      if [ "''${#nix_files[@]}" -eq 0 ]; then
        log_skip "no nix files found for formatting check"
        return 0
      fi

      log_info "checking nix formatting files=''${#nix_files[@]}"
      ${formatterPkg}/bin/nixfmt --check "''${nix_files[@]}"
      log_ok "nix formatting check passed files=''${#nix_files[@]}"
    }

    run_flake_show_check() {
      log_info "checking flake output surface ref=$flake_ref"
      ${pkgs.nix}/bin/nix flake show --no-write-lock-file "$flake_ref" > /dev/null
      log_ok "flake output surface check passed ref=$flake_ref"
    }

    run_help_check() {
      local help_ref
      help_ref="$flake_ref#help"
      log_info "checking help surface ref=$help_ref"
      ${pkgs.nix}/bin/nix run "$help_ref" > /dev/null
      log_ok "help surface check passed ref=$help_ref"
    }

    run_flake_check() {
      log_info "checking flake checks ref=$flake_ref"
      ${pkgs.nix}/bin/nix flake check -L --no-write-lock-file "$flake_ref"
      log_ok "flake checks passed ref=$flake_ref"
    }

    should_skip_flake_check() {
      [ -n "''${NIX_BUILD_TOP:-}" ] || [ -n "''${NIXFIED_PARENT_WORKFLOW_ID:-}" ]
    }

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --mode)
          if [ "$#" -lt 2 ]; then
            log_error "--mode requires a value"
            usage >&2
            exit 2
          fi
          mode="$2"
          shift 2
          ;;
        --quick)
          mode="quick"
          shift
          ;;
        --full)
          mode="full"
          shift
          ;;
        --flake)
          if [ "$#" -lt 2 ]; then
            log_error "--flake requires a value"
            usage >&2
            exit 2
          fi
          flake_ref="$2"
          shift 2
          ;;
        --help)
          usage
          exit 0
          ;;
        *)
          log_error "unknown argument: $1"
          usage >&2
          exit 2
          ;;
      esac
    done

    case "$mode" in
      quick|full)
        ;;
      *)
        log_error "unsupported mode: $mode"
        usage >&2
        exit 2
        ;;
    esac

    log_info "running nix checks mode=$mode flake=$flake_ref"
    run_nixfmt_check
    run_flake_show_check
    run_help_check

    if [ "$mode" = "full" ]; then
      if should_skip_flake_check; then
        log_skip "flake checks skipped inside nix build sandbox or parent workflow ref=$flake_ref"
      else
        run_flake_check
      fi
    fi

    log_ok "nix checks passed mode=$mode flake=$flake_ref"
''
