{ project, lib, ... }:

{
  commands = {
    build = {
      description = "Build artifacts";
      api = lib.appApi.mkApi {
        name = "build";
        summary = "Build release artifacts";
        details = "Builds the workspace in release mode with all features enabled.";
        usage = [ "nix run .#build" ];
        examples = [ "nix run .#build" ];
        category = "build";
      };
      env = {
        "${project.envVar}" = "prod";
      };
      useDeps = true;
      script = ''
        cargo build --release --all-features
      '';
    };
  };
}
