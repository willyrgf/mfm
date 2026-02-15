{ project, lib, ... }:

{
  commands = {
    check = {
      description = "Run quality checks";
      api = lib.appApi.mkApi {
        name = "check";
        summary = "Run fmt + clippy (nightly)";
        details = "Runs rustfmt and clippy using the pinned nightly toolchain (via cargo-nightly).";
        usage = [ "nix run .#check" ];
        examples = [ "nix run .#check" ];
        category = "quality";
      };
      env = { };
      useDeps = true;
      script = ''
        cargo-nightly fmt --all -- --check
        cargo-nightly clippy --workspace --lib --examples --tests --benches --all-features
        cargo run -p mfm-architecture-verify --
      '';
    };

    discovery-refresh = {
      description = "Regenerate discovery artifacts";
      api = lib.appApi.mkApi {
        name = "discovery-refresh";
        summary = "Refresh docs/repo-index.json and docs/repo-map.md";
        details = "Runs the quality check command with --refresh-discovery so discovery artifacts are regenerated.";
        usage = [ "nix run .#discovery-refresh" ];
        examples = [ "nix run .#discovery-refresh" ];
        category = "quality";
      };
      env = { };
      useDeps = true;
      script = ''
        nix run .#check -- --refresh-discovery
      '';
    };
  };
}
