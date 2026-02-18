{ project, lib, ... }:

let
  # v2 shell-app contract inventory (project-level):
  # - check: typed, outputs=text, wraps cargo-nightly/cargo, failure map owner=project/quality.nix
  # - discovery-refresh: typed, outputs=text, wraps nix run check, failure map owner=project/quality.nix
  failureCodesScript = {
    generic = 1;
    usage = 2;
    precondition = 3;
    unavailable = 4;
    timeout = 5;
  };
  failureCodesCargo = failureCodesScript // {
    cargoFailure = 101;
  };
in
{
  commands = {
    check = {
      description = "Run quality checks";
      api = lib.appApi.mkCommandApi {
        class = "typed";
        name = "check";
        summary = "Run fmt + clippy (nightly)";
        details = "Runs rustfmt and clippy using the pinned nightly toolchain (via cargo-nightly).";
        usage = [ "nix run .#check" ];
        examples = [ "nix run .#check" ];
        category = "quality";
        failureCodes = failureCodesCargo;
      };
      env = {
        RUSTC_WRAPPER = "sccache";
      };
      useDeps = true;
      script = ''
        set -euo pipefail
        cargo-nightly fmt --all -- --check
        cargo-nightly clippy --workspace --lib --examples --tests --benches --all-features -- -D warnings
        cargo-nightly run -p mfm-architecture-verify --
      '';
    };

    discovery-refresh = {
      description = "Regenerate discovery artifacts";
      api = lib.appApi.mkCommandApi {
        class = "typed";
        name = "discovery-refresh";
        summary = "Refresh docs/repo-index.json and docs/repo-map.md";
        details = "Runs the quality check command with --refresh-discovery so discovery artifacts are regenerated.";
        usage = [ "nix run .#discovery-refresh" ];
        examples = [ "nix run .#discovery-refresh" ];
        category = "quality";
        failureCodes = failureCodesScript;
      };
      env = { };
      useDeps = true;
      script = ''
        nix run .#check -- --refresh-discovery
      '';
    };
  };
}
