# Typed shell app contract helpers and runtime validator.
{ pkgs }:

let
  lib = pkgs.lib;
  validation = import ./validation.nix { inherit pkgs; };
  inherit (validation)
    expect
    renderErrors
    isNonEmptyString
    isNonEmptyList
    isListOfNonEmptyStrings
    isEnvVarName
    ;

  supportedTypes = [
    "string"
    "int"
    "bool"
    "enum"
    "pathAbs"
    "pathRel"
    "port"
    "durationSec"
    "json"
  ];

  supportedArgKinds = [
    "flag"
    "option"
    "positional"
  ];

  supportedOutputModes = [
    "text"
    "kv"
    "json"
  ];

  supportedCommandClasses = [
    "typed"
    "passthrough"
    "json"
    "batch-runner"
  ];

  defaultFailureCodes = {
    generic = 1;
    usage = 2;
    precondition = 3;
    unavailable = 4;
    timeout = 5;
  };

  runtimeLogLevels = [
    "error"
    "warn"
    "info"
    "debug"
    "trace"
  ];

  runtimeOutputModes = [
    "stdout"
    "logs"
    "both"
  ];

  runtimeLogLevelEnvName = "LOG_LEVEL";
  runtimeLogLevelAliases = [ "NIXFIED_LOG_LEVEL" ];
  runtimeLogLevelDefault = "info";

  runtimeOutputModeEnvName = "OUTPUT_MODE";
  runtimeOutputModeAliases = [ "NIXFIED_OUTPUT_MODE" ];
  runtimeOutputModeDefault = "stdout";

  hasPrefix =
    prefix: value:
    builtins.isString value
    && (builtins.stringLength value) >= (builtins.stringLength prefix)
    && (builtins.substring 0 (builtins.stringLength prefix) value) == prefix;

  isShortOpt = value: builtins.isString value && (builtins.match "^-.[^[:space:]]*$" value) != null;

  isLongOpt = value: builtins.isString value && (builtins.match "^--[^[:space:]]+$" value) != null;

  isSupportedType = value: builtins.elem value supportedTypes;
  isSupportedKind = value: builtins.elem value supportedArgKinds;
  isSupportedOutputMode = value: builtins.elem value supportedOutputModes;
  isSupportedCommandClass = value: builtins.elem value supportedCommandClasses;
  isPositiveExitCode = value: builtins.isInt value && value > 0 && value < 256;

  duplicatesOf =
    values:
    let
      uniq = lib.unique values;
    in
    builtins.filter (
      value: (builtins.length (builtins.filter (candidate: candidate == value) values)) > 1
    ) uniq;

  normalizeStringSet = values: lib.sort (a: b: a < b) (lib.unique values);

  sameStringSet = expected: actual: normalizeStringSet expected == normalizeStringSet actual;

  mkRuntimePrimitiveEnvSpecs =
    {
      logLevelDefault ? runtimeLogLevelDefault,
      outputModeDefault ? runtimeOutputModeDefault,
    }:
    [
      {
        name = runtimeLogLevelEnvName;
        type = "enum";
        required = false;
        aliases = runtimeLogLevelAliases;
        values = runtimeLogLevels;
        default = logLevelDefault;
      }
      {
        name = runtimeOutputModeEnvName;
        type = "enum";
        required = false;
        aliases = runtimeOutputModeAliases;
        values = runtimeOutputModes;
        default = outputModeDefault;
      }
    ];

  mkServiceRuntimePrimitivesV1 =
    {
      logLevelDefault ? runtimeLogLevelDefault,
      outputModeDefault ? runtimeOutputModeDefault,
    }:
    let
      _logDefaultCheck =
        if builtins.elem logLevelDefault runtimeLogLevels then
          null
        else
          throw "runtime primitive default invalid: logLevel.default must be one of ${builtins.concatStringsSep "|" runtimeLogLevels}";
      _outputDefaultCheck =
        if builtins.elem outputModeDefault runtimeOutputModes then
          null
        else
          throw "runtime primitive default invalid: outputMode.default must be one of ${builtins.concatStringsSep "|" runtimeOutputModes}";
    in
    builtins.seq _logDefaultCheck (
      builtins.seq _outputDefaultCheck {
        version = 1;
        logLevel = {
          env = runtimeLogLevelEnvName;
          aliases = runtimeLogLevelAliases;
          values = runtimeLogLevels;
          default = logLevelDefault;
        };
        outputMode = {
          env = runtimeOutputModeEnvName;
          aliases = runtimeOutputModeAliases;
          values = runtimeOutputModes;
          default = outputModeDefault;
        };
      }
    );

  findEnvSpecByName =
    {
      env,
      name,
    }:
    lib.findFirst (envSpec: (envSpec.name or "") == name) null env;

  validateRuntimePrimitiveEnvErrors =
    {
      name,
      env,
      envName,
      aliases,
      values,
    }:
    let
      envSpec = findEnvSpecByName {
        inherit env;
        name = envName;
      };
      prefix = "${name}: appContract.env[${envName}]";
      actualAliases = if envSpec == null then [ ] else (envSpec.aliases or [ ]);
      actualValues = if envSpec == null then [ ] else (envSpec.values or [ ]);
      actualDefault = if envSpec == null || !(envSpec ? default) then null else envSpec.default;
      actualType = if envSpec == null then null else (envSpec.type or "string");
      actualRequired = if envSpec == null then false else (envSpec.required or false);
    in
    expect (envSpec != null) "${name}: appContract.env must include ${envName} runtime primitive"
    ++ expect (envSpec == null || actualType == "enum") "${prefix}: type must be enum"
    ++ expect (
      envSpec == null || sameStringSet aliases actualAliases
    ) "${prefix}: aliases must be exactly [${builtins.concatStringsSep ", " aliases}]"
    ++ expect (
      envSpec == null || sameStringSet values actualValues
    ) "${prefix}: values must be exactly [${builtins.concatStringsSep ", " values}]"
    ++ expect (envSpec == null || actualRequired == false) "${prefix}: required must be false"
    ++ expect (
      envSpec == null || actualDefault == null || builtins.elem actualDefault values
    ) "${prefix}: default must be one of [${builtins.concatStringsSep ", " values}] when set";

  validateArgSpecErrors =
    {
      appName,
      argSpec,
      index,
    }:
    let
      prefix = "${appName}.appContract.args[${toString index}]";
      kind =
        if !(argSpec ? kind) || argSpec.kind == null then
          if (argSpec ? long) || (argSpec ? short) then "option" else "positional"
        else
          argSpec.kind;
      type = argSpec.type or (if kind == "flag" then "bool" else "string");
      values = argSpec.values or [ ];
      hasDefault = argSpec ? default;
      min = argSpec.min or null;
      max = argSpec.max or null;
    in
    if !builtins.isAttrs argSpec then
      [ "${prefix}: arg spec must be an attribute set" ]
    else
      expect (isNonEmptyString (argSpec.name or "")) "${prefix}: name must be a non-empty string"
      ++ expect (isSupportedKind kind) "${prefix}: kind must be one of ${builtins.concatStringsSep "|" supportedArgKinds}"
      ++ expect (isSupportedType type) "${prefix}: type must be one of ${builtins.concatStringsSep "|" supportedTypes}"
      ++ expect (
        kind == "positional" || isLongOpt (argSpec.long or "")
      ) "${prefix}: long must be set to --<token> for flag/option args"
      ++ expect (
        !(argSpec ? short) || isShortOpt (argSpec.short or "")
      ) "${prefix}: short must use -<char> format when set"
      ++ expect (
        kind != "positional" || (!(argSpec ? long) && !(argSpec ? short))
      ) "${prefix}: positional args cannot set long/short"
      ++ expect (kind != "flag" || type == "bool") "${prefix}: flag args must use type=bool"
      ++ expect (
        !hasDefault
        || (
          builtins.isString argSpec.default
          || builtins.isInt argSpec.default
          || builtins.isBool argSpec.default
        )
      ) "${prefix}: default must be string/int/bool when set"
      ++ expect (
        type != "enum" || (isNonEmptyList values && isListOfNonEmptyStrings values)
      ) "${prefix}: enum args must define values=[\"...\"]"
      ++ expect (type != "enum" || !(argSpec ? pattern)) "${prefix}: enum args cannot also define pattern"
      ++ expect (min == null || builtins.isInt min) "${prefix}: min must be an integer when set"
      ++ expect (max == null || builtins.isInt max) "${prefix}: max must be an integer when set"
      ++ expect ((min == null || max == null) || min <= max) "${prefix}: min must be <= max";

  validateEnvSpecErrors =
    {
      appName,
      envSpec,
      index,
    }:
    let
      prefix = "${appName}.appContract.env[${toString index}]";
      type = envSpec.type or "string";
      values = envSpec.values or [ ];
      min = envSpec.min or null;
      max = envSpec.max or null;
      aliases = envSpec.aliases or [ ];
      hasDefault = envSpec ? default;
    in
    if !builtins.isAttrs envSpec then
      [ "${prefix}: env spec must be an attribute set" ]
    else
      expect (isEnvVarName (envSpec.name or "")) "${prefix}: name must be a valid env var token (A-Z0-9_)"
      ++ expect (isSupportedType type) "${prefix}: type must be one of ${builtins.concatStringsSep "|" supportedTypes}"
      ++ expect (
        type != "enum" || (isNonEmptyList values && isListOfNonEmptyStrings values)
      ) "${prefix}: enum env vars must define values=[\"...\"]"
      ++ expect (
        !(envSpec ? required) || builtins.isBool (envSpec.required or null)
      ) "${prefix}: required must be a boolean when set"
      ++ expect (
        !hasDefault
        || (
          builtins.isString envSpec.default
          || builtins.isInt envSpec.default
          || builtins.isBool envSpec.default
        )
      ) "${prefix}: default must be string/int/bool when set"
      ++ expect (
        !(envSpec ? sensitive) || builtins.isBool (envSpec.sensitive or null)
      ) "${prefix}: sensitive must be a boolean when set"
      ++ expect (
        builtins.isList aliases && builtins.all isEnvVarName aliases
      ) "${prefix}: aliases must be env var tokens"
      ++ expect (min == null || builtins.isInt min) "${prefix}: min must be an integer when set"
      ++ expect (max == null || builtins.isInt max) "${prefix}: max must be an integer when set"
      ++ expect ((min == null || max == null) || min <= max) "${prefix}: min must be <= max";

  validateFailureCodesErrors =
    {
      appName,
      failureCodes,
    }:
    let
      prefix = "${appName}.appContract.failureCodes";
      names = if builtins.isAttrs failureCodes then builtins.attrNames failureCodes else [ ];
      badKeys = builtins.filter (key: !isNonEmptyString key) names;
      badValues = builtins.filter (key: !(isPositiveExitCode failureCodes.${key})) names;
    in
    expect (builtins.isAttrs failureCodes) "${prefix}: failureCodes must be an attribute set"
    ++ expect (badKeys == [ ]) "${prefix}: failure code names must be non-empty strings"
    ++
      expect (badValues == [ ])
        "${prefix}: failure code values must be integers in 1..255 (invalid: ${builtins.concatStringsSep ", " badValues})";

  validateAppContractErrors =
    {
      name,
      contract,
    }:
    let
      args = contract.args or [ ];
      env = contract.env or [ ];
      commandClass = contract.commandClass or null;
      allowUnknownArgs = contract.allowUnknownArgs or false;
      outputs = contract.outputs or { mode = "text"; };
      mode = outputs.mode or "text";
      argErrs = builtins.concatLists (
        lib.imap0 (
          index: argSpec:
          validateArgSpecErrors {
            appName = name;
            inherit argSpec index;
          }
        ) args
      );
      envErrs = builtins.concatLists (
        lib.imap0 (
          index: envSpec:
          validateEnvSpecErrors {
            appName = name;
            inherit envSpec index;
          }
        ) env
      );
      argNames = map (argSpec: argSpec.name or "") args;
      longNames = builtins.filter (token: token != "") (map (argSpec: argSpec.long or "") args);
      shortNames = builtins.filter (token: token != "") (map (argSpec: argSpec.short or "") args);
      envNames = map (envSpec: envSpec.name or "") env;
      envAliases = builtins.concatLists (map (envSpec: envSpec.aliases or [ ]) env);
      duplicateArgNames = duplicatesOf argNames;
      duplicateLongNames = duplicatesOf longNames;
      duplicateShortNames = duplicatesOf shortNames;
      duplicateEnvNames = duplicatesOf envNames;
      duplicateEnvAliases = duplicatesOf envAliases;
      runtimePrimitiveErrs =
        validateRuntimePrimitiveEnvErrors {
          inherit
            name
            env
            ;
          envName = runtimeLogLevelEnvName;
          aliases = runtimeLogLevelAliases;
          values = runtimeLogLevels;
        }
        ++ validateRuntimePrimitiveEnvErrors {
          inherit
            name
            env
            ;
          envName = runtimeOutputModeEnvName;
          aliases = runtimeOutputModeAliases;
          values = runtimeOutputModes;
        };
    in
    if contract == null then
      [ "${name}: missing appContract" ]
    else if !builtins.isAttrs contract then
      [ "${name}: appContract must be an attribute set" ]
    else
      expect (contract ? version) "${name}: appContract.version is required"
      ++ expect (builtins.isInt (
        contract.version or null
      )) "${name}: appContract.version must be an integer"
      ++ expect ((contract.version or null) == 2) "${name}: appContract.version must be 2"
      ++ expect (isNonEmptyString (
        contract.name or ""
      )) "${name}: appContract.name must be a non-empty string"
      ++ expect (
        !(contract ? allowUnknownArgs) || builtins.isBool (contract.allowUnknownArgs or null)
      ) "${name}: appContract.allowUnknownArgs must be a boolean when set"
      ++ expect (contract ? commandClass) "${name}: appContract.commandClass is required"
      ++ expect (isNonEmptyString commandClass) "${name}: appContract.commandClass must be a non-empty string"
      ++ expect (isSupportedCommandClass commandClass) "${name}: appContract.commandClass must be one of ${builtins.concatStringsSep "|" supportedCommandClasses}"
      ++ expect (builtins.isList args) "${name}: appContract.args must be a list"
      ++ expect (builtins.isList env) "${name}: appContract.env must be a list"
      ++ expect (builtins.isAttrs outputs) "${name}: appContract.outputs must be an attribute set"
      ++ expect (isSupportedOutputMode mode) "${name}: appContract.outputs.mode must be one of ${builtins.concatStringsSep "|" supportedOutputModes}"
      ++ expect (
        !(outputs ? keys) || isListOfNonEmptyStrings (outputs.keys or [ ])
      ) "${name}: appContract.outputs.keys must be a list of non-empty strings when set"
      ++ expect (
        !(contract ? idempotent) || builtins.isBool (contract.idempotent or null)
      ) "${name}: appContract.idempotent must be a boolean when set"
      ++ expect (
        commandClass != "typed" || allowUnknownArgs == false
      ) "${name}: appContract.commandClass=typed requires allowUnknownArgs=false"
      ++ expect (
        commandClass != "typed" || mode != "json"
      ) "${name}: appContract.commandClass=typed cannot use outputs.mode=json"
      ++ expect (
        commandClass != "passthrough" || allowUnknownArgs == true
      ) "${name}: appContract.commandClass=passthrough requires allowUnknownArgs=true"
      ++ expect (
        commandClass != "json" || mode == "json"
      ) "${name}: appContract.commandClass=json requires outputs.mode=json"
      ++ expect (
        commandClass != "json" || allowUnknownArgs == false
      ) "${name}: appContract.commandClass=json requires allowUnknownArgs=false"
      ++ expect (
        commandClass != "batch-runner" || allowUnknownArgs == false
      ) "${name}: appContract.commandClass=batch-runner requires allowUnknownArgs=false"
      ++ validateFailureCodesErrors {
        appName = name;
        failureCodes = contract.failureCodes or defaultFailureCodes;
      }
      ++
        expect (duplicateArgNames == [ ])
          "${name}: appContract.args has duplicate names: ${builtins.concatStringsSep ", " duplicateArgNames}"
      ++
        expect (duplicateLongNames == [ ])
          "${name}: appContract.args has duplicate long options: ${builtins.concatStringsSep ", " duplicateLongNames}"
      ++
        expect (duplicateShortNames == [ ])
          "${name}: appContract.args has duplicate short options: ${builtins.concatStringsSep ", " duplicateShortNames}"
      ++
        expect (duplicateEnvNames == [ ])
          "${name}: appContract.env has duplicate names: ${builtins.concatStringsSep ", " duplicateEnvNames}"
      ++
        expect (duplicateEnvAliases == [ ])
          "${name}: appContract.env has duplicate aliases: ${builtins.concatStringsSep ", " duplicateEnvAliases}"
      ++ argErrs
      ++ envErrs
      ++ runtimePrimitiveErrs;

  validateAppContract =
    {
      name,
      contract,
    }:
    let
      errs = validateAppContractErrors { inherit name contract; };
    in
    if errs == [ ] then
      contract
    else
      throw ''
        Nixfied shell app contract violated for "${name}":
        ${renderErrors errs}
      '';

  tokenToArgName =
    token:
    let
      base0 =
        if hasPrefix "--" token then
          builtins.substring 2 ((builtins.stringLength token) - 2) token
        else
          token;
      base1 =
        if hasPrefix "-" base0 then
          builtins.substring 1 ((builtins.stringLength base0) - 1) base0
        else
          base0;
      base2 =
        let
          equalMatch = builtins.match "^([^=]+)=.*$" base1;
        in
        if equalMatch == null then base1 else builtins.head equalMatch;
    in
    builtins.replaceStrings [ "-" "." ":" " " "/" ] [ "_" "_" "_" "_" "_" ] base2;

  mkDefaultArgFromDoc =
    arg:
    let
      token =
        if builtins.isAttrs arg then
          (arg.name or "")
        else if builtins.isString arg then
          arg
        else
          "";
      name = tokenToArgName token;
      hasLong = hasPrefix "--" token;
      hasShort = (!hasLong) && hasPrefix "-" token;
      kind =
        if hasLong || hasShort then
          if (builtins.match "^--[^=]+=.+$" token) != null then "option" else "flag"
        else
          "positional";
      type = if kind == "flag" then "bool" else "string";
    in
    {
      inherit
        name
        kind
        type
        ;
    }
    // lib.optionalAttrs hasLong { long = token; }
    // lib.optionalAttrs hasShort { short = token; };

  mkDefaultEnvFromDoc =
    envDoc:
    let
      name =
        if builtins.isAttrs envDoc then
          (envDoc.name or "")
        else if builtins.isString envDoc then
          envDoc
        else
          "";
    in
    {
      inherit name;
      type = "string";
      required = false;
    };

  mkDefaultAppContract =
    {
      name,
      args ? [ ],
      env ? [ ],
      allowUnknownArgs ? false,
      commandClass ? "typed",
      outputsMode ? "text",
      failureCodes ? defaultFailureCodes,
      idempotent ? true,
    }:
    let
      envSpecs0 = map mkDefaultEnvFromDoc env;
      upsertEnvSpec =
        acc: spec: (builtins.filter (envSpec: (envSpec.name or "") != (spec.name or "")) acc) ++ [ spec ];
      envSpecs = builtins.foldl' upsertEnvSpec envSpecs0 (mkRuntimePrimitiveEnvSpecs { });
    in
    {
      version = 2;
      inherit
        name
        allowUnknownArgs
        commandClass
        idempotent
        failureCodes
        ;
      args = map mkDefaultArgFromDoc args;
      env = envSpecs;
      outputs = {
        mode = outputsMode;
      };
    };

  valueToString =
    value:
    if value == null then
      ""
    else if builtins.isBool value then
      if value then "true" else "false"
    else
      toString value;

  normalizeArgSpec =
    argSpec:
    let
      kind =
        if !(argSpec ? kind) || argSpec.kind == null then
          if (argSpec ? long) || (argSpec ? short) then "option" else "positional"
        else
          argSpec.kind;
      type = argSpec.type or (if kind == "flag" then "bool" else "string");
    in
    {
      name = argSpec.name or "";
      inherit
        kind
        type
        ;
      long = argSpec.long or "";
      short = argSpec.short or "";
      required = argSpec.required or false;
      values = argSpec.values or [ ];
      min = if argSpec ? min then argSpec.min else null;
      max = if argSpec ? max then argSpec.max else null;
    };

  normalizeEnvSpec =
    envSpec:
    {
      name = envSpec.name or "";
      type = envSpec.type or "string";
      required = envSpec.required or false;
      hasDefault = envSpec ? default;
      default = if envSpec ? default then envSpec.default else null;
      min = if envSpec ? min then envSpec.min else null;
      max = if envSpec ? max then envSpec.max else null;
      values = envSpec.values or [ ];
      aliases = envSpec.aliases or [ ];
    };

  mkShellArray =
    var: values: ''
      declare -ag ${var}=(
    ${lib.concatStringsSep "\n" (map (value: "  ${lib.escapeShellArg value}") values)}
      )
    '';

  mkShellAssoc =
    var: entries: ''
      declare -Ag ${var}=()
    ${lib.concatStringsSep "\n" (
      map (entry: "${var}[${lib.escapeShellArg entry.key}]=${lib.escapeShellArg entry.value}") entries
    )}
    '';

  mkContractRuntime =
    { name, contract }:
    let
      validated = validateAppContract { inherit name contract; };
      noneSentinel = "__NIXFIED_NONE__";
      normalizedArgs = map normalizeArgSpec (validated.args or [ ]);
      normalizedEnv = map normalizeEnvSpec (validated.env or [ ]);
      failureCodes = validated.failureCodes or defaultFailureCodes;
      positionalArgNames = map (argSpec: argSpec.name) (
        builtins.filter (argSpec: argSpec.kind == "positional") normalizedArgs
      );
      argEntries = extractor: map (argSpec: {
        key = argSpec.name;
        value = extractor argSpec;
      }) normalizedArgs;
      envEntries = extractor: map (envSpec: {
        key = envSpec.name;
        value = extractor envSpec;
      }) normalizedEnv;
      longEntries = builtins.filter (entry: entry.key != "") (map (argSpec: {
        key = argSpec.long;
        value = argSpec.name;
      }) normalizedArgs);
      shortEntries = builtins.filter (entry: entry.key != "") (map (argSpec: {
        key = argSpec.short;
        value = argSpec.name;
      }) normalizedArgs);
      failureCodeEntries = map (failureName: {
        key = toString failureCodes.${failureName};
        value = failureName;
      }) (builtins.attrNames failureCodes);
    in
    pkgs.writeText "${name}-app-contract-runtime.sh" ''
      NIXFIED_CONTRACT_PLAN_NAME=${lib.escapeShellArg validated.name}
      NIXFIED_CONTRACT_ALLOW_UNKNOWN=${lib.escapeShellArg (if validated.allowUnknownArgs or false then "true" else "false")}

      ${mkShellArray "NIXFIED_CONTRACT_ARG_NAMES" (map (argSpec: argSpec.name) normalizedArgs)}
      ${mkShellArray "NIXFIED_CONTRACT_POSITIONAL_SPECS" positionalArgNames}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_KIND" (argEntries (argSpec: argSpec.kind))}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_TYPE" (argEntries (argSpec: argSpec.type))}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_REQUIRED" (
        argEntries (argSpec: if argSpec.required then "true" else "false")
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_VALUES" (
        argEntries (argSpec: lib.concatStringsSep "\n" argSpec.values)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_VALUES_LABEL" (
        argEntries (argSpec: lib.concatStringsSep "," argSpec.values)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_MIN" (
        argEntries (argSpec: if argSpec.min == null then noneSentinel else toString argSpec.min)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_MAX" (
        argEntries (argSpec: if argSpec.max == null then noneSentinel else toString argSpec.max)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_BY_LONG" longEntries}
      ${mkShellAssoc "NIXFIED_CONTRACT_ARG_BY_SHORT" shortEntries}

      ${mkShellArray "NIXFIED_CONTRACT_ENV_NAMES" (map (envSpec: envSpec.name) normalizedEnv)}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_TYPE" (envEntries (envSpec: envSpec.type))}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_REQUIRED" (
        envEntries (envSpec: if envSpec.required then "true" else "false")
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_DEFAULT" (
        envEntries (
          envSpec: if envSpec.hasDefault then valueToString envSpec.default else noneSentinel
        )
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_MIN" (
        envEntries (envSpec: if envSpec.min == null then noneSentinel else toString envSpec.min)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_MAX" (
        envEntries (envSpec: if envSpec.max == null then noneSentinel else toString envSpec.max)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_VALUES" (
        envEntries (envSpec: lib.concatStringsSep "\n" envSpec.values)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_VALUES_LABEL" (
        envEntries (envSpec: lib.concatStringsSep "," envSpec.values)
      )}
      ${mkShellAssoc "NIXFIED_CONTRACT_ENV_ALIASES" (
        envEntries (envSpec: lib.concatStringsSep "\n" envSpec.aliases)
      )}

      ${mkShellAssoc "NIXFIED_CONTRACT_FAILURE_CODE" failureCodeEntries}
    '';

  runtime = pkgs.writeShellScript "nixfied-shell-contract-runtime" ''
    NIXFIED_CONTRACT_JQ="${pkgs.jq}/bin/jq"
    NIXFIED_CONTRACT_NONE="__NIXFIED_NONE__"
    NIXFIED_CONTRACT_RESOLVED_VALUE=""
    NIXFIED_CONTRACT_PLAN_LOADED="0"
    NIXFIED_CONTRACT_PLAN_FILE=""
    NIXFIED_CONTRACT_RUNTIME_LOG_LEVEL_ALIASES=${
      lib.escapeShellArg (lib.concatStringsSep "\n" runtimeLogLevelAliases)
    }
    NIXFIED_CONTRACT_RUNTIME_LOG_LEVEL_VALUES=${lib.escapeShellArg (lib.concatStringsSep "\n" runtimeLogLevels)}
    NIXFIED_CONTRACT_RUNTIME_LOG_LEVEL_VALUES_LABEL=${
      lib.escapeShellArg (lib.concatStringsSep "," runtimeLogLevels)
    }
    NIXFIED_CONTRACT_RUNTIME_OUTPUT_MODE_ALIASES=${
      lib.escapeShellArg (lib.concatStringsSep "\n" runtimeOutputModeAliases)
    }
    NIXFIED_CONTRACT_RUNTIME_OUTPUT_MODE_VALUES=${
      lib.escapeShellArg (lib.concatStringsSep "\n" runtimeOutputModes)
    }
    NIXFIED_CONTRACT_RUNTIME_OUTPUT_MODE_VALUES_LABEL=${
      lib.escapeShellArg (lib.concatStringsSep "," runtimeOutputModes)
    }

    _nixfied_contract_err() {
      if command -v log_error >/dev/null 2>&1; then
        log_error "$*"
      else
        printf '%s\n' "ERROR: $*" >&2
      fi
    }

    _nixfied_contract_is_int() {
      local value="''${1:-}"
      if [[ "$value" =~ ^-?[0-9]+$ ]]; then
        return 0
      fi
      return 1
    }

    _nixfied_contract_sanitize_name() {
      local value="$1"
      printf '%s' "$value" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_'
    }

    _nixfied_contract_var_is_set() {
      local name="$1"
      [ "''${!name+x}" = "x" ]
    }

    _nixfied_contract_load_runtime_plan() {
      local contract_file="''${1:-}"
      local runtime_file="''${NIXFIED_APP_CONTRACT_RUNTIME:-}"

      if [ "''${NIXFIED_CONTRACT_PLAN_LOADED:-0}" = "1" ] && [ "$runtime_file" = "''${NIXFIED_CONTRACT_PLAN_FILE:-}" ]; then
        return 0
      fi

      if [ -z "$runtime_file" ]; then
        _nixfied_contract_err "app contract runtime plan not configured"
        if [ -n "$contract_file" ]; then
          _nixfied_contract_err "set NIXFIED_APP_CONTRACT_RUNTIME alongside NIXFIED_APP_CONTRACT_FILE=$contract_file"
        fi
        return 1
      fi

      if [ ! -f "$runtime_file" ]; then
        _nixfied_contract_err "app contract runtime plan missing path=$runtime_file"
        return 1
      fi

      # shellcheck source=/dev/null
      source "$runtime_file"
      NIXFIED_CONTRACT_PLAN_FILE="$runtime_file"
      NIXFIED_CONTRACT_PLAN_LOADED="1"
      return 0
    }

    _nixfied_contract_resolve_env_with_aliases() {
      local name="$1"
      local aliases_block="$2"
      local default="$3"
      local label="$4"
      local strict="$5"
      local canonical_set=0 canonical_value=""
      local alias_set=0 alias_name="" alias_value=""
      local current_alias="" current_value=""

      NIXFIED_CONTRACT_RESOLVED_VALUE=""

      if _nixfied_contract_var_is_set "$name"; then
        canonical_set=1
        canonical_value="''${!name-}"
      fi

      if [ -n "$aliases_block" ]; then
        while IFS= read -r current_alias || [ -n "$current_alias" ]; do
          [ -z "$current_alias" ] && continue
          if ! _nixfied_contract_var_is_set "$current_alias"; then
            continue
          fi
          current_value="''${!current_alias-}"
          if [ "$alias_set" -eq 0 ]; then
            alias_set=1
            alias_name="$current_alias"
            alias_value="$current_value"
            continue
          fi
          if [ "$strict" = "1" ] && [ "$current_value" != "$alias_value" ]; then
            _nixfied_contract_err "$label has conflicting alias values alias=$alias_name and alias=$current_alias; set one alias or use matching values"
            return 1
          fi
        done <<< "$aliases_block"
      fi

      if [ "$strict" = "1" ]; then
        if [ "$canonical_set" -eq 1 ] && [ -z "$canonical_value" ]; then
          _nixfied_contract_err "$label cannot be empty when set; unset $name to use defaults"
          return 1
        fi
        if [ "$alias_set" -eq 1 ] && [ -z "$alias_value" ]; then
          _nixfied_contract_err "$label alias=$alias_name cannot be empty when set; unset $alias_name to use defaults"
          return 1
        fi
        if [ "$canonical_set" -eq 1 ] && [ "$alias_set" -eq 1 ] && [ "$canonical_value" != "$alias_value" ]; then
          _nixfied_contract_err "$label has conflicting values between $name and $alias_name; set one variable or use matching values"
          return 1
        fi
      fi

      if [ "$canonical_set" -eq 1 ] && [ -n "$canonical_value" ]; then
        NIXFIED_CONTRACT_RESOLVED_VALUE="$canonical_value"
      elif [ "$alias_set" -eq 1 ] && [ -n "$alias_value" ]; then
        NIXFIED_CONTRACT_RESOLVED_VALUE="$alias_value"
      elif [ "$default" != "$NIXFIED_CONTRACT_NONE" ]; then
        NIXFIED_CONTRACT_RESOLVED_VALUE="$default"
      else
        NIXFIED_CONTRACT_RESOLVED_VALUE=""
      fi

      return 0
    }

    nixfied_contract_resolve_runtime_primitives() {
      local log_level_default="''${1:-info}"
      local output_mode_default="''${2:-stdout}"
      local log_level="" output_mode=""

      case "$log_level_default" in
        error|warn|info|debug|trace)
          ;;
        *)
          _nixfied_contract_err "invalid runtime default LOG_LEVEL value=$log_level_default allowed=error,warn,info,debug,trace"
          return 2
          ;;
      esac

      case "$output_mode_default" in
        stdout|logs|both)
          ;;
        *)
          _nixfied_contract_err "invalid runtime default OUTPUT_MODE value=$output_mode_default allowed=stdout,logs,both"
          return 2
          ;;
      esac

      if ! _nixfied_contract_resolve_env_with_aliases \
        "LOG_LEVEL" \
        "$NIXFIED_CONTRACT_RUNTIME_LOG_LEVEL_ALIASES" \
        "$log_level_default" \
        "env:LOG_LEVEL" \
        "1"
      then
        return 2
      fi
      log_level="$NIXFIED_CONTRACT_RESOLVED_VALUE"
      if [ -z "$log_level" ]; then
        log_level="$log_level_default"
      fi

      case "$log_level" in
        error|warn|info|debug|trace)
          ;;
        *)
          _nixfied_contract_err "invalid LOG_LEVEL value=$log_level allowed=error,warn,info,debug,trace"
          return 2
          ;;
      esac

      if ! _nixfied_contract_resolve_env_with_aliases \
        "OUTPUT_MODE" \
        "$NIXFIED_CONTRACT_RUNTIME_OUTPUT_MODE_ALIASES" \
        "$NIXFIED_CONTRACT_NONE" \
        "env:OUTPUT_MODE" \
        "1"
      then
        return 2
      fi
      output_mode="$NIXFIED_CONTRACT_RESOLVED_VALUE"
      if [ -z "$output_mode" ]; then
        if [ "$log_level" = "debug" ] && [ "$output_mode_default" = "stdout" ]; then
          output_mode="both"
        else
          output_mode="$output_mode_default"
        fi
      fi

      case "$output_mode" in
        stdout|logs|both)
          ;;
        *)
          _nixfied_contract_err "invalid OUTPUT_MODE value=$output_mode allowed=stdout,logs,both"
          return 2
          ;;
      esac

      export LOG_LEVEL="$log_level"
      export OUTPUT_MODE="$output_mode"
      export NIXFIED_LOG_LEVEL="$log_level"
      export NIXFIED_OUTPUT_MODE="$output_mode"
      return 0
    }

    _nixfied_contract_validate_scalar() {
      local type="$1"
      local value="$2"
      local values_block="$3"
      local values_label="$4"
      local min="$5"
      local max="$6"
      local label="$7"
      local num=0
      local enum_value=""
      local enum_match=0

      case "$type" in
        string)
          return 0
          ;;
        bool)
          case "$value" in
            1|0|true|false|TRUE|FALSE|yes|YES|no|NO|on|ON)
              return 0
              ;;
            *)
              _nixfied_contract_err "$label must be bool (got '$value')"
              return 1
              ;;
          esac
          ;;
        int)
          if ! _nixfied_contract_is_int "$value"; then
            _nixfied_contract_err "$label must be int (got '$value')"
            return 1
          fi
          num="$value"
          ;;
        durationSec)
          if ! _nixfied_contract_is_int "$value"; then
            _nixfied_contract_err "$label must be durationSec int (got '$value')"
            return 1
          fi
          num="$value"
          if [ "$num" -lt 0 ]; then
            _nixfied_contract_err "$label must be >= 0 seconds (got '$value')"
            return 1
          fi
          ;;
        enum)
          while IFS= read -r enum_value || [ -n "$enum_value" ]; do
            [ -z "$enum_value" ] && continue
            if [ "$enum_value" = "$value" ]; then
              enum_match=1
              break
            fi
          done <<< "$values_block"
          if [ "$enum_match" != "1" ]; then
            _nixfied_contract_err "$label must be one of $values_label (got '$value')"
            return 1
          fi
          return 0
          ;;
        pathAbs)
          case "$value" in
            /*) return 0 ;;
            *)
              _nixfied_contract_err "$label must be an absolute path (got '$value')"
              return 1
              ;;
          esac
          ;;
        pathRel)
          case "$value" in
            ""|/*)
              _nixfied_contract_err "$label must be a relative path (got '$value')"
              return 1
              ;;
            *)
              return 0
              ;;
          esac
          ;;
        port)
          if ! _nixfied_contract_is_int "$value"; then
            _nixfied_contract_err "$label must be port int (got '$value')"
            return 1
          fi
          num="$value"
          if [ "$num" -lt 1 ] || [ "$num" -gt 65535 ]; then
            _nixfied_contract_err "$label must be port 1-65535 (got '$value')"
            return 1
          fi
          ;;
        json)
          if ! printf '%s' "$value" | "$NIXFIED_CONTRACT_JQ" -e . >/dev/null 2>&1; then
            _nixfied_contract_err "$label must be valid json"
            return 1
          fi
          return 0
          ;;
        *)
          _nixfied_contract_err "unsupported type=$type for $label"
          return 1
          ;;
      esac

      if [ "$min" != "$NIXFIED_CONTRACT_NONE" ] && [ "$num" -lt "$min" ]; then
        _nixfied_contract_err "$label must be >= $min (got '$value')"
        return 1
      fi
      if [ "$max" != "$NIXFIED_CONTRACT_NONE" ] && [ "$num" -gt "$max" ]; then
        _nixfied_contract_err "$label must be <= $max (got '$value')"
        return 1
      fi
      return 0
    }

    nixfied_contract_validate_env() {
      local contract_file="''${1:-}"
      local failures=0
      local name="" type="" required="" default="" min="" max="" values_block="" values_label="" aliases_block=""
      local strict_runtime_env=0
      local value=""

      _nixfied_contract_load_runtime_plan "$contract_file" || return 2

      for name in "''${NIXFIED_CONTRACT_ENV_NAMES[@]}"; do
        type="''${NIXFIED_CONTRACT_ENV_TYPE[$name]}"
        required="''${NIXFIED_CONTRACT_ENV_REQUIRED[$name]}"
        default="''${NIXFIED_CONTRACT_ENV_DEFAULT[$name]}"
        min="''${NIXFIED_CONTRACT_ENV_MIN[$name]}"
        max="''${NIXFIED_CONTRACT_ENV_MAX[$name]}"
        values_block="''${NIXFIED_CONTRACT_ENV_VALUES[$name]}"
        values_label="''${NIXFIED_CONTRACT_ENV_VALUES_LABEL[$name]}"
        aliases_block="''${NIXFIED_CONTRACT_ENV_ALIASES[$name]}"
        strict_runtime_env=0
        if [ "$name" = "LOG_LEVEL" ] || [ "$name" = "OUTPUT_MODE" ]; then
          strict_runtime_env=1
        fi
        if ! _nixfied_contract_resolve_env_with_aliases "$name" "$aliases_block" "$default" "env:$name" "$strict_runtime_env"; then
          failures=1
          continue
        fi
        value="$NIXFIED_CONTRACT_RESOLVED_VALUE"

        if [ -z "$value" ] && [ "$required" = "true" ]; then
          _nixfied_contract_err "required env var missing name=$name"
          failures=1
          continue
        fi

        if [ -n "$value" ]; then
          if ! _nixfied_contract_validate_scalar \
            "$type" \
            "$value" \
            "$values_block" \
            "$values_label" \
            "$min" \
            "$max" \
            "env:$name"
          then
            failures=1
            continue
          fi
          export "$name=$value"
        fi
      done

      if [ "$failures" -ne 0 ]; then
        return 2
      fi
      return 0
    }

    nixfied_contract_validate_args() {
      local contract_file="''${1:-}"
      shift || true

      _nixfied_contract_load_runtime_plan "$contract_file" || return 2

      local allow_unknown="$NIXFIED_CONTRACT_ALLOW_UNKNOWN"

      local failures=0
      local name="" long="" lookup_name="" token="" value=""
      local -a positional_values=()
      local -a short_cluster=()
      declare -A seen=()

      while [ "$#" -gt 0 ]; do
        token="$1"
        shift

        case "$token" in
          --)
            while [ "$#" -gt 0 ]; do
              positional_values+=("$1")
              shift
            done
            break
            ;;
          --*=*)
            long="''${token%%=*}"
            value="''${token#*=}"
            lookup_name="''${NIXFIED_CONTRACT_ARG_BY_LONG[$long]:-}"
            if [ -z "$lookup_name" ]; then
              if [ "$allow_unknown" = "true" ]; then
                continue
              fi
              _nixfied_contract_err "unknown option token=$long"
              failures=1
              continue
            fi
            if [ "''${NIXFIED_CONTRACT_ARG_KIND[$lookup_name]}" = "flag" ]; then
              _nixfied_contract_err "flag does not accept a value token=$long"
              failures=1
              continue
            fi
            if ! _nixfied_contract_validate_scalar \
              "''${NIXFIED_CONTRACT_ARG_TYPE[$lookup_name]}" \
              "$value" \
              "''${NIXFIED_CONTRACT_ARG_VALUES[$lookup_name]}" \
              "''${NIXFIED_CONTRACT_ARG_VALUES_LABEL[$lookup_name]}" \
              "''${NIXFIED_CONTRACT_ARG_MIN[$lookup_name]}" \
              "''${NIXFIED_CONTRACT_ARG_MAX[$lookup_name]}" \
              "arg:$lookup_name"
            then
              failures=1
              continue
            fi
            seen["$lookup_name"]="1"
            export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=$value"
            ;;
          --*)
            lookup_name="''${NIXFIED_CONTRACT_ARG_BY_LONG[$token]:-}"
            if [ -z "$lookup_name" ]; then
              if [ "$allow_unknown" = "true" ]; then
                continue
              fi
              _nixfied_contract_err "unknown option token=$token"
              failures=1
              continue
            fi

            if [ "''${NIXFIED_CONTRACT_ARG_KIND[$lookup_name]}" = "flag" ]; then
              seen["$lookup_name"]="1"
              export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=true"
              continue
            fi

            if [ "$#" -eq 0 ]; then
              _nixfied_contract_err "option requires value token=$token"
              failures=1
              continue
            fi
            value="$1"
            shift
            if ! _nixfied_contract_validate_scalar \
              "''${NIXFIED_CONTRACT_ARG_TYPE[$lookup_name]}" \
              "$value" \
              "''${NIXFIED_CONTRACT_ARG_VALUES[$lookup_name]}" \
              "''${NIXFIED_CONTRACT_ARG_VALUES_LABEL[$lookup_name]}" \
              "''${NIXFIED_CONTRACT_ARG_MIN[$lookup_name]}" \
              "''${NIXFIED_CONTRACT_ARG_MAX[$lookup_name]}" \
              "arg:$lookup_name"
            then
              failures=1
              continue
            fi
            seen["$lookup_name"]="1"
            export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=$value"
            ;;
          -*)
            lookup_name="''${NIXFIED_CONTRACT_ARG_BY_SHORT[$token]:-}"
            if [ -n "$lookup_name" ]; then
              if [ "''${NIXFIED_CONTRACT_ARG_KIND[$lookup_name]}" = "flag" ]; then
                seen["$lookup_name"]="1"
                export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=true"
              else
                if [ "$#" -eq 0 ]; then
                  _nixfied_contract_err "option requires value token=$token"
                  failures=1
                  continue
                fi
                value="$1"
                shift
                if ! _nixfied_contract_validate_scalar \
                  "''${NIXFIED_CONTRACT_ARG_TYPE[$lookup_name]}" \
                  "$value" \
                  "''${NIXFIED_CONTRACT_ARG_VALUES[$lookup_name]}" \
                  "''${NIXFIED_CONTRACT_ARG_VALUES_LABEL[$lookup_name]}" \
                  "''${NIXFIED_CONTRACT_ARG_MIN[$lookup_name]}" \
                  "''${NIXFIED_CONTRACT_ARG_MAX[$lookup_name]}" \
                  "arg:$lookup_name"
                then
                  failures=1
                  continue
                fi
                seen["$lookup_name"]="1"
                export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=$value"
              fi
              continue
            fi

            short_cluster=()
            if [ "''${#token}" -gt 2 ]; then
              local idx=1
              while [ "$idx" -lt "''${#token}" ]; do
                short_cluster+=("-''${token:$idx:1}")
                idx=$((idx + 1))
              done
            fi
            if [ "''${#short_cluster[@]}" -gt 0 ]; then
              local cluster_ok=1
              local short_tok=""
              for short_tok in "''${short_cluster[@]}"; do
                lookup_name="''${NIXFIED_CONTRACT_ARG_BY_SHORT[$short_tok]:-}"
                if [ -z "$lookup_name" ] || [ "''${NIXFIED_CONTRACT_ARG_KIND[$lookup_name]}" != "flag" ]; then
                  cluster_ok=0
                  break
                fi
              done
              if [ "$cluster_ok" -eq 1 ]; then
                for short_tok in "''${short_cluster[@]}"; do
                  lookup_name="''${NIXFIED_CONTRACT_ARG_BY_SHORT[$short_tok]}"
                  seen["$lookup_name"]="1"
                  export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=true"
                done
                continue
              fi
            fi

            if [ "$allow_unknown" = "true" ]; then
              continue
            fi
            _nixfied_contract_err "unknown option token=$token"
            failures=1
            ;;
          *)
            positional_values+=("$token")
            ;;
        esac
      done

      local pos_index=0
      local pos_name=""
      for pos_name in "''${NIXFIED_CONTRACT_POSITIONAL_SPECS[@]}"; do
        if [ "$pos_index" -lt "''${#positional_values[@]}" ]; then
          value="''${positional_values[$pos_index]}"
          if ! _nixfied_contract_validate_scalar \
            "''${NIXFIED_CONTRACT_ARG_TYPE[$pos_name]}" \
            "$value" \
            "''${NIXFIED_CONTRACT_ARG_VALUES[$pos_name]}" \
            "''${NIXFIED_CONTRACT_ARG_VALUES_LABEL[$pos_name]}" \
            "''${NIXFIED_CONTRACT_ARG_MIN[$pos_name]}" \
            "''${NIXFIED_CONTRACT_ARG_MAX[$pos_name]}" \
            "arg:$pos_name"
          then
            failures=1
          else
            seen["$pos_name"]="1"
            export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$pos_name")=$value"
          fi
          pos_index=$((pos_index + 1))
        elif [ "''${NIXFIED_CONTRACT_ARG_REQUIRED[$pos_name]}" = "true" ]; then
          _nixfied_contract_err "missing required positional arg name=$pos_name"
          failures=1
        fi
      done

      if [ "$pos_index" -lt "''${#positional_values[@]}" ] && [ "$allow_unknown" != "true" ]; then
        _nixfied_contract_err "unexpected positional args count=$(("''${#positional_values[@]}" - pos_index))"
        failures=1
      fi

      for name in "''${NIXFIED_CONTRACT_ARG_NAMES[@]}"; do
        if [ "''${NIXFIED_CONTRACT_ARG_REQUIRED[$name]}" = "true" ] && [ -z "''${seen[$name]:-}" ]; then
          _nixfied_contract_err "missing required arg name=$name"
          failures=1
        fi
      done

      if [ "$failures" -ne 0 ]; then
        return 2
      fi
      return 0
    }

    nixfied_contract_validate_exit() {
      local contract_file="''${1:-}"
      local exit_code="$2"

      if [ "$exit_code" -eq 0 ]; then
        return 0
      fi

      _nixfied_contract_load_runtime_plan "$contract_file" || return 2

      if [ -n "''${NIXFIED_CONTRACT_FAILURE_CODE[$exit_code]+x}" ]; then
        return 0
      fi

      _nixfied_contract_err "undeclared exit code code=$exit_code"
      return 2
    }

    nixfied_contract_emit_kv() {
      local key="$1"
      local value="$2"
      if [ -z "$key" ]; then
        _nixfied_contract_err "nixfied_contract_emit_kv requires key"
        return 1
      fi
      printf '%s=%s\n' "$key" "$value"
    }

    nixfied_contract_emit_json() {
      local payload="$1"
      if ! printf '%s' "$payload" | "$NIXFIED_CONTRACT_JQ" -e . >/dev/null 2>&1; then
        _nixfied_contract_err "nixfied_contract_emit_json payload must be valid json"
        return 1
      fi
      printf '%s\n' "$payload"
    }
  '';
in
{
  inherit
    supportedTypes
    supportedArgKinds
    supportedOutputModes
    supportedCommandClasses
    runtimeLogLevels
    runtimeOutputModes
    runtimeLogLevelEnvName
    runtimeLogLevelAliases
    runtimeLogLevelDefault
    runtimeOutputModeEnvName
    runtimeOutputModeAliases
    runtimeOutputModeDefault
    mkRuntimePrimitiveEnvSpecs
    mkServiceRuntimePrimitivesV1
    defaultFailureCodes
    validateAppContractErrors
    validateAppContract
    mkDefaultAppContract
    mkContractRuntime
    runtime
    ;
}
