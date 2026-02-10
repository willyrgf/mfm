{ project, ... }:

{
  commands = {
    check = {
      description = "Run quality checks";
      env = { };
      useDeps = true;
      script = ''
        cargo-nightly fmt --all -- --check
        cargo-nightly clippy --workspace --lib --examples --tests --benches --all-features
      '';
    };
  };
}
