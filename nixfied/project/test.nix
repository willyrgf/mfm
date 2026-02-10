{ project, ... }:

{
  commands = {
    test = {
      description = "Run tests";
      api = {
        version = 1;
        summary = "Run tests (nextest)";
        details = "Runs the full workspace test suite using cargo-nextest.";
        usage = [ "nix run .#test" ];
        examples = [ "nix run .#test" ];
        category = "test";
      };
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
