{ project, ... }:

{
  commands = {
    build = {
      description = "Build artifacts";
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
