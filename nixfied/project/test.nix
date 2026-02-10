{ project, ... }:

{
  commands = {
    test = {
      description = "Run tests";
      env = {
        "${project.envVar}" = "test";
      };
      useDeps = true;
      script = ''
        cargo nextest run --workspace
      '';
    };
  };
}
