{ project, lib, ... }:

let
  # v2 shell-app contract inventory (project-level):
  # - test: typed, outputs=text, wraps cargo-nextest, failure map owner=project/test.nix
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
    test = {
      description = "Run tests";
      api = lib.appApi.mkCommandApi {
        class = "typed";
        name = "test";
        summary = "Run tests (nextest)";
        details = "Runs the full workspace test suite using cargo-nextest.";
        usage = [ "nix run .#test" ];
        examples = [ "nix run .#test" ];
        category = "test";
        failureCodes = failureCodesCargo;
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
