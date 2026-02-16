# Nixfied app API + shell contract helpers (validation + mkNixfiedApp)
{
  pkgs,
  mkApp,
  shellContract,
}:

let
  lib = pkgs.lib;
  validation = import ./validation.nix { inherit pkgs; };
  inherit (validation)
    isNonEmptyString
    isNonEmptyList
    expect
    renderErrors
    isListOfNonEmptyStrings
    sortedAttrNames
    optionalAttrSatisfies
    isKVSpecList
    ;

  mkFixHint =
    commandName: ''
      Fix:
        - For project commands: define commands.${commandName}.api = { version = 2; summary = "..."; details = "..."; usage = [ "nix run .#${commandName}" ]; appContract = { commandClass = "typed"; ... }; };
        - For generated/internal apps: set app.meta.nixfied.api (or use lib.appApi.mkNixfiedApp).
    '';
  throwNamedViolation =
    name: errs:
    throw ''
      Nixfied app API contract violated for "${name}":
      ${renderErrors errs}

      ${mkFixHint name}
    '';

  validateApiErrors =
    { name, api }:
    let
      base = [ ];
    in
    if api == null then
      [ "${name}: missing api" ]
    else if !builtins.isAttrs api then
      [ "${name}: api must be an attribute set" ]
    else
      base
      ++ expect (api ? version) "${name}: api.version is required"
      ++ expect (builtins.isInt (api.version or null)) "${name}: api.version must be an integer"
      ++ expect ((api.version or null) == 2) "${name}: api.version must be 2"
      ++ expect (api ? summary) "${name}: api.summary is required"
      ++ expect (isNonEmptyString (api.summary or "")) "${name}: api.summary must be a non-empty string"
      ++ expect (api ? details) "${name}: api.details is required"
      ++ expect (builtins.isString (api.details or null)) "${name}: api.details must be a string"
      ++ expect (api ? usage) "${name}: api.usage is required"
      ++ expect (isNonEmptyList (api.usage or null)) "${name}: api.usage must be a non-empty list"
      ++ expect (isListOfNonEmptyStrings (
        api.usage or [ ]
      )) "${name}: api.usage must be a list of non-empty strings"
      ++ expect (optionalAttrSatisfies api "examples" isListOfNonEmptyStrings) "${name}: api.examples must be a list of non-empty strings"
      ++ expect (optionalAttrSatisfies api "args" isKVSpecList) "${name}: api.args must be a list of { name, description }"
      ++ expect (optionalAttrSatisfies api "env" isKVSpecList) "${name}: api.env must be a list of { name, description }"
      ++ expect (optionalAttrSatisfies api "category" isNonEmptyString) "${name}: api.category must be a non-empty string"
      ++ shellContract.validateAppContractErrors {
        inherit name;
        contract = api.appContract or null;
      };

  validateApi =
    { name, api }:
    let
      errs = validateApiErrors { inherit name api; };
    in
    if errs == [ ] then
      api
    else
      throwNamedViolation name errs;

  failureProfiles = {
    script = shellContract.defaultFailureCodes;
    cargo = shellContract.defaultFailureCodes // { cargoFailure = 101; };
    supervisor = shellContract.defaultFailureCodes // {
      interrupted = 130;
      terminated = 143;
    };
    foundry = shellContract.defaultFailureCodes // {
      assertion = 134;
    };
  };

  arg = {
    flag =
      {
        name,
        long,
        short ? null,
        required ? false,
      }:
      {
        inherit
          name
          long
          required
          ;
        kind = "flag";
        type = "bool";
      }
      // lib.optionalAttrs (short != null) { inherit short; };

    option =
      {
        name,
        long,
        short ? null,
        type ? "string",
        required ? false,
        values ? [ ],
        min ? null,
        max ? null,
      }:
      {
        inherit
          name
          long
          type
          required
          ;
        kind = "option";
      }
      // lib.optionalAttrs (short != null) { inherit short; }
      // lib.optionalAttrs (values != [ ]) { inherit values; }
      // lib.optionalAttrs (min != null) { inherit min; }
      // lib.optionalAttrs (max != null) { inherit max; };

    positional =
      {
        name,
        type ? "string",
        required ? false,
        values ? [ ],
        min ? null,
        max ? null,
      }:
      {
        inherit
          name
          type
          required
          ;
        kind = "positional";
      }
      // lib.optionalAttrs (values != [ ]) { inherit values; }
      // lib.optionalAttrs (min != null) { inherit min; }
      // lib.optionalAttrs (max != null) { inherit max; };
  };

  env = rec {
    typed =
      {
        name,
        type ? "string",
        required ? false,
        aliases ? [ ],
        sensitive ? false,
        values ? [ ],
        min ? null,
        max ? null,
        default ? null,
      }:
      {
        inherit
          name
          type
          required
          aliases
          sensitive
          ;
      }
      // lib.optionalAttrs (values != [ ]) { inherit values; }
      // lib.optionalAttrs (min != null) { inherit min; }
      // lib.optionalAttrs (max != null) { inherit max; }
      // lib.optionalAttrs (default != null) { inherit default; };

    string =
      {
        name,
        required ? false,
        aliases ? [ ],
        sensitive ? false,
        default ? null,
      }:
      env.typed {
        inherit
          name
          required
          aliases
          sensitive
          default
          ;
        type = "string";
      };
  };

  mkApi =
    {
      name,
      summary,
      details,
      usage,
      examples ? [ ],
      args ? [ ],
      env ? [ ],
      category ? "core",
      appContract ? null,
      allowUnknownArgs ? false,
      commandClass ? "typed",
      idempotent ? true,
      outputsMode ? "text",
      failureCodes ? failureProfiles.script,
    }:
    let
      contract =
        if appContract == null then
          shellContract.mkDefaultAppContract {
            inherit
              name
              args
              env
              allowUnknownArgs
              commandClass
              idempotent
              outputsMode
              failureCodes
              ;
          }
        else
          appContract;
      api =
        {
          version = 2;
          inherit
            summary
            details
            usage
            category
            appContract
            ;
        }
        // lib.optionalAttrs (examples != [ ]) { inherit examples; }
        // lib.optionalAttrs (args != [ ]) { inherit args; }
        // lib.optionalAttrs (env != [ ]) { inherit env; };
      apiFinal = api // { appContract = contract; };
      _ = validateApi {
        inherit name;
        api = apiFinal;
      };
    in
    apiFinal;

  mkCommandClassApi =
    {
      name,
      summary,
      details,
      usage,
      examples ? [ ],
      args ? [ ],
      env ? [ ],
      category ? "core",
      appContract ? null,
      contractArgs ? null,
      contractEnv ? null,
      outputsKeys ? [ ],
      allowUnknownArgs,
      commandClass,
      idempotent ? true,
      outputsMode ? "text",
      failureCodes ? failureProfiles.script,
    }:
    let
      defaultContract =
        shellContract.mkDefaultAppContract {
          inherit
            name
            args
            env
            allowUnknownArgs
            commandClass
            idempotent
            outputsMode
            failureCodes
            ;
        };

      contractWithSpecOverrides =
        defaultContract
        // lib.optionalAttrs (contractArgs != null) { args = contractArgs; }
        // lib.optionalAttrs (contractEnv != null) { env = contractEnv; }
        // lib.optionalAttrs (outputsKeys != [ ]) {
          outputs = defaultContract.outputs // { keys = outputsKeys; };
        };

      contract0 =
        if appContract == null then
          contractWithSpecOverrides
        else
          appContract // { commandClass = appContract.commandClass or commandClass; };

      allowUnknownActual = contract0.allowUnknownArgs or false;
      outputModeActual = ((contract0.outputs or { mode = "text"; }).mode or "text");

      _classPolicy =
        if commandClass == "typed" then
          if allowUnknownActual then
            throw "${name}: typed command class requires allowUnknownArgs=false"
          else if outputModeActual == "json" then
            throw "${name}: typed command class cannot use outputs.mode=json"
          else
            null
        else if commandClass == "passthrough" then
          if allowUnknownActual then
            null
          else
            throw "${name}: passthrough command class requires allowUnknownArgs=true"
        else if commandClass == "json" then
          if outputModeActual != "json" then
            throw "${name}: json command class requires outputs.mode=json"
          else if allowUnknownActual then
            throw "${name}: json command class requires allowUnknownArgs=false"
          else
            null
        else if commandClass == "batch-runner" then
          if allowUnknownActual then
            throw "${name}: batch-runner command class requires allowUnknownArgs=false"
          else
            null
        else
          null;
    in
    builtins.seq _classPolicy (
      mkApi {
        inherit
          name
          summary
          details
          usage
          examples
          args
          env
          category
          idempotent
          ;
        appContract = contract0;
        commandClass = commandClass;
        allowUnknownArgs = allowUnknownActual;
        outputsMode = outputModeActual;
        failureCodes = contract0.failureCodes or failureCodes;
      }
    );

  mkTypedCommandApi =
    args:
    mkCommandClassApi (
      args
      // {
        allowUnknownArgs = false;
        commandClass = "typed";
      }
    );

  mkPassthroughCommandApi =
    args:
    mkCommandClassApi (
      args
      // {
        allowUnknownArgs = true;
        commandClass = "passthrough";
      }
    );

  mkJsonCommandApi =
    args:
    mkCommandClassApi (
      args
      // {
        allowUnknownArgs = false;
        commandClass = "json";
        outputsMode = "json";
      }
    );

  mkBatchRunnerCommandApi =
    args:
    mkCommandClassApi (
      args
      // {
        allowUnknownArgs = false;
        commandClass = "batch-runner";
        idempotent = args.idempotent or false;
      }
    );

  validateAppErrors =
    { name, app }:
    if !builtins.isAttrs app then
      [ "${name}: app must be an attribute set" ]
    else if (app.type or null) != "app" then
      [ "${name}: app.type must be \"app\"" ]
    else
      let
        meta = app.meta or { };
        api = ((meta.nixfied or { }).api or null);
      in
      if api == null then
        [ "${name}: missing meta.nixfied.api" ]
      else
        validateApiErrors { inherit name api; };

  validateApp =
    { name, app }:
    let
      errs = validateAppErrors { inherit name app; };
    in
    if errs == [ ] then
      app
    else
      throwNamedViolation name errs;

  validateApps =
    apps:
    let
      names = sortedAttrNames apps;
      errs = builtins.concatLists (
        map (
          name:
          validateAppErrors {
            inherit name;
            app = apps.${name};
          }
        ) names
      );
    in
    if errs == [ ] then
      apps
    else
      throw ''
        Nixfied app API contract violated:
        ${renderErrors errs}

        ${mkFixHint "<name>"}
      '';

  mkNixfiedApp =
    {
      name,
      script,
      fixtures ? null,
      env ? { },
      useDeps ? false,
      fixtureProfile ? "default",
      api,
      meta ? { },
    }:
    let
      apiFinal = validateApi { inherit name api; };
      mergedMeta = lib.recursiveUpdate { nixfied.api = apiFinal; } meta;
    in
    mkApp {
      inherit
        name
        script
        fixtures
        env
        useDeps
        fixtureProfile
        ;
      description = apiFinal.summary;
      api = apiFinal;
      meta = mergedMeta;
    };
in
{
  inherit
    mkApi
    mkTypedCommandApi
    mkPassthroughCommandApi
    mkJsonCommandApi
    mkBatchRunnerCommandApi
    validateApi
    validateApp
    validateApps
    mkNixfiedApp
    failureProfiles
    arg
    env
    ;
}
