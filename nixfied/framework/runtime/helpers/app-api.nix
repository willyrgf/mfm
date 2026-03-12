{
  pkgs,
  shellContract ? import ./shell-contract.nix { inherit pkgs; },
  mkApp,
}:
let
  lib = pkgs.lib;

  runtimeEnvSpecs = [
    {
      name = shellContract.runtimeLogLevelEnvName;
      type = "enum";
      required = false;
      aliases = shellContract.runtimeLogLevelAliases;
      values = shellContract.runtimeLogLevels;
      default = shellContract.runtimeLogLevelDefault;
    }
    {
      name = shellContract.runtimeOutputModeEnvName;
      type = "enum";
      required = false;
      aliases = shellContract.runtimeOutputModeAliases;
      values = shellContract.runtimeOutputModes;
      default = shellContract.runtimeOutputModeDefault;
    }
  ];

  normalizeEnvSpec =
    spec:
    spec
    // {
      type = spec.type or "string";
      required = spec.required or false;
      aliases = spec.aliases or [ ];
    };

  mergeEnvSpecs =
    envSpecs:
    let
      existing = map (spec: spec.name) envSpecs;
      extras = lib.filter (spec: !(builtins.elem spec.name existing)) runtimeEnvSpecs;
    in
    map normalizeEnvSpec envSpecs ++ extras;

  mkCommandApi =
    {
      class ? "typed",
      name,
      summary,
      details ? "",
      usage ? [ ],
      examples ? [ ],
      args ? [ ],
      env ? [ ],
      category ? "core",
      idempotent ? false,
      allowUnknownArgs ? class == "passthrough",
      outputs ? null,
      failureCodes ? {
        generic = 1;
        usage = 2;
        precondition = 3;
      },
    }:
    let
      resolvedOutputs =
        if outputs != null then
          outputs
        else if class == "json" then
          {
            mode = "json";
            keys = [ ];
          }
        else
          {
            mode = "text";
            keys = [ ];
          };
    in
    {
      version = 2;
      inherit
        summary
        details
        usage
        examples
        category
        ;
      appContract = {
        version = 2;
        inherit
          name
          args
          allowUnknownArgs
          idempotent
          failureCodes
          ;
        commandClass = class;
        env = mergeEnvSpecs env;
        outputs = resolvedOutputs;
      };
    };

  mkNixfiedApp =
    {
      name,
      script,
      fixtures ? null,
      env ? { },
      useDeps ? false,
      fixtureProfile ? "default",
      description ? null,
      meta ? { },
      api ? null,
    }:
    mkApp {
      inherit
        name
        script
        fixtures
        env
        useDeps
        fixtureProfile
        description
        meta
        api
        ;
    };
in
{
  inherit mkCommandApi mkNixfiedApp;
}
