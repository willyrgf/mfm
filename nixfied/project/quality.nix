{ project, ... }:

{
  commands = {
    check = {
      description = "Run quality checks";
      api = {
        version = 1;
        summary = "Run fmt + clippy (nightly)";
        details = "Runs rustfmt and clippy using the pinned nightly toolchain (via cargo-nightly).";
        usage = [ "nix run .#check" ];
        examples = [ "nix run .#check" ];
        category = "quality";
      };
      env = { };
      useDeps = true;
      script = ''
        ./scripts/check-no-ad-hoc-prints.sh
        cargo-nightly fmt --all -- --check
        cargo-nightly clippy --workspace --lib --examples --tests --benches --all-features
      '';
    };
  };
}
