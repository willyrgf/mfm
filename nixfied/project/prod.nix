{ project, lib, ... }:

let
  # v2 shell-app contract inventory (project-level):
  # - build: typed, outputs=text, wraps cargo build, failure map owner=project/prod.nix
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
    build = {
      description = "Build artifacts";
      api = lib.appApi.mkCommandApi {
        class = "typed";
        name = "build";
        summary = "Build release artifacts";
        details = "Builds the workspace in release mode with all features enabled.";
        usage = [ "nix run .#build" ];
        examples = [ "nix run .#build" ];
        category = "build";
        failureCodes = failureCodesCargo;
      };
      env = {
        "${project.envVar}" = "prod";
        RUSTC_WRAPPER = "sccache";
      };
      useDeps = true;
      script = ''
        cargo-nightly build --release --all-features
      '';
    };
  };
}
