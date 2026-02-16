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

  hasPrefix =
    prefix: value:
    builtins.isString value
    && (builtins.stringLength value) >= (builtins.stringLength prefix)
    && (builtins.substring 0 (builtins.stringLength prefix) value) == prefix;

  isShortOpt =
    value: builtins.isString value && (builtins.match "^-.[^[:space:]]*$" value) != null;

  isLongOpt =
    value: builtins.isString value && (builtins.match "^--[^[:space:]]+$" value) != null;

  isSupportedType = value: builtins.elem value supportedTypes;
  isSupportedKind = value: builtins.elem value supportedArgKinds;
  isSupportedOutputMode = value: builtins.elem value supportedOutputModes;
  isSupportedCommandClass = value: builtins.elem value supportedCommandClasses;
  isPositiveExitCode = value: builtins.isInt value && value > 0 && value < 256;
  isEnvVarName = value: builtins.isString value && (builtins.match "^[A-Z_][A-Z0-9_]*$" value) != null;

  duplicatesOf =
    values:
    let
      uniq = lib.unique values;
    in
    builtins.filter (
      value: (builtins.length (builtins.filter (candidate: candidate == value) values)) > 1
    ) uniq;

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
      ++ expect (
        kind != "flag" || type == "bool"
      ) "${prefix}: flag args must use type=bool"
      ++ expect (
        !hasDefault || (builtins.isString argSpec.default || builtins.isInt argSpec.default || builtins.isBool argSpec.default)
      ) "${prefix}: default must be string/int/bool when set"
      ++ expect (
        type != "enum" || (isNonEmptyList values && isListOfNonEmptyStrings values)
      ) "${prefix}: enum args must define values=[\"...\"]"
      ++ expect (
        type != "enum" || !(argSpec ? pattern)
      ) "${prefix}: enum args cannot also define pattern"
      ++ expect (
        min == null || builtins.isInt min
      ) "${prefix}: min must be an integer when set"
      ++ expect (
        max == null || builtins.isInt max
      ) "${prefix}: max must be an integer when set"
      ++ expect (
        (min == null || max == null) || min <= max
      ) "${prefix}: min must be <= max";

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
        !hasDefault || (builtins.isString envSpec.default || builtins.isInt envSpec.default || builtins.isBool envSpec.default)
      ) "${prefix}: default must be string/int/bool when set"
      ++ expect (
        !(envSpec ? sensitive) || builtins.isBool (envSpec.sensitive or null)
      ) "${prefix}: sensitive must be a boolean when set"
      ++ expect (
        builtins.isList aliases && builtins.all isEnvVarName aliases
      ) "${prefix}: aliases must be env var tokens"
      ++ expect (
        min == null || builtins.isInt min
      ) "${prefix}: min must be an integer when set"
      ++ expect (
        max == null || builtins.isInt max
      ) "${prefix}: max must be an integer when set"
      ++ expect (
        (min == null || max == null) || min <= max
      ) "${prefix}: min must be <= max";

  validateFailureCodesErrors =
    {
      appName,
      failureCodes,
    }:
    let
      prefix = "${appName}.appContract.failureCodes";
      names =
        if builtins.isAttrs failureCodes then builtins.attrNames failureCodes else [ ];
      badKeys = builtins.filter (key: !isNonEmptyString key) names;
      badValues =
        builtins.filter
          (key: !(isPositiveExitCode failureCodes.${key}))
          names;
    in
    expect (builtins.isAttrs failureCodes) "${prefix}: failureCodes must be an attribute set"
    ++ expect (badKeys == [ ]) "${prefix}: failure code names must be non-empty strings"
    ++ expect (
      badValues == [ ]
    ) "${prefix}: failure code values must be integers in 1..255 (invalid: ${builtins.concatStringsSep ", " badValues})";

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
        lib.imap0 (index: argSpec: validateArgSpecErrors {
          appName = name;
          inherit argSpec index;
        }) args
      );
      envErrs = builtins.concatLists (
        lib.imap0 (index: envSpec: validateEnvSpecErrors {
          appName = name;
          inherit envSpec index;
        }) env
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
    in
    if contract == null then
      [ "${name}: missing appContract" ]
    else if !builtins.isAttrs contract then
      [ "${name}: appContract must be an attribute set" ]
    else
      expect (contract ? version) "${name}: appContract.version is required"
      ++ expect (
        builtins.isInt (contract.version or null)
      ) "${name}: appContract.version must be an integer"
      ++ expect ((contract.version or null) == 2) "${name}: appContract.version must be 2"
      ++ expect (
        isNonEmptyString (contract.name or "")
      ) "${name}: appContract.name must be a non-empty string"
      ++ expect (
        !(contract ? allowUnknownArgs) || builtins.isBool (contract.allowUnknownArgs or null)
      ) "${name}: appContract.allowUnknownArgs must be a boolean when set"
      ++ expect (
        contract ? commandClass
      ) "${name}: appContract.commandClass is required"
      ++ expect (
        isNonEmptyString commandClass
      ) "${name}: appContract.commandClass must be a non-empty string"
      ++ expect (
        isSupportedCommandClass commandClass
      ) "${name}: appContract.commandClass must be one of ${builtins.concatStringsSep "|" supportedCommandClasses}"
      ++ expect (
        builtins.isList args
      ) "${name}: appContract.args must be a list"
      ++ expect (
        builtins.isList env
      ) "${name}: appContract.env must be a list"
      ++ expect (
        builtins.isAttrs outputs
      ) "${name}: appContract.outputs must be an attribute set"
      ++ expect (
        isSupportedOutputMode mode
      ) "${name}: appContract.outputs.mode must be one of ${builtins.concatStringsSep "|" supportedOutputModes}"
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
      ++ expect (
        duplicateArgNames == [ ]
      ) "${name}: appContract.args has duplicate names: ${builtins.concatStringsSep ", " duplicateArgNames}"
      ++ expect (
        duplicateLongNames == [ ]
      ) "${name}: appContract.args has duplicate long options: ${builtins.concatStringsSep ", " duplicateLongNames}"
      ++ expect (
        duplicateShortNames == [ ]
      ) "${name}: appContract.args has duplicate short options: ${builtins.concatStringsSep ", " duplicateShortNames}"
      ++ expect (
        duplicateEnvNames == [ ]
      ) "${name}: appContract.env has duplicate names: ${builtins.concatStringsSep ", " duplicateEnvNames}"
      ++ expect (
        duplicateEnvAliases == [ ]
      ) "${name}: appContract.env has duplicate aliases: ${builtins.concatStringsSep ", " duplicateEnvAliases}"
      ++ argErrs
      ++ envErrs;

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
      base0 = if hasPrefix "--" token then builtins.substring 2 ((builtins.stringLength token) - 2) token else token;
      base1 = if hasPrefix "-" base0 then builtins.substring 1 ((builtins.stringLength base0) - 1) base0 else base0;
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
      type =
        if kind == "flag" then "bool" else "string";
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
      env = map mkDefaultEnvFromDoc env;
      outputs = {
        mode = outputsMode;
      };
    };

  runtime = pkgs.writeShellScript "nixfied-shell-contract-runtime" ''
    NIXFIED_CONTRACT_JQ="${pkgs.jq}/bin/jq"
    NIXFIED_CONTRACT_NONE="__NIXFIED_NONE__"

    _nixfied_contract_err() {
      echo "ERROR: $*" >&2
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

    _nixfied_contract_validate_scalar() {
      local type="$1"
      local value="$2"
      local values_json="$3"
      local min="$4"
      local max="$5"
      local label="$6"
      local num=0

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
          if ! printf '%s' "$values_json" | "$NIXFIED_CONTRACT_JQ" -e --arg value "$value" 'index($value) != null' >/dev/null 2>&1; then
            _nixfied_contract_err "$label must be one of $(printf '%s' "$values_json" | "$NIXFIED_CONTRACT_JQ" -r 'join(",")') (got '$value')"
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
      local contract_file="$1"
      local failures=0
      local name="" type="" required="" default="" min="" max="" values_json="" aliases_json="" env_spec_json=""
      local value="" alias=""

      while IFS= read -r env_spec_json; do
        [ -z "$env_spec_json" ] && continue
        name="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '.name // ""')"
        type="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '.type // "string"')"
        required="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '(.required // false) | tostring')"
        default="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r 'if has("default") then (.default | tostring) else "__NIXFIED_NONE__" end')"
        min="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r 'if has("min") then (.min | tostring) else "__NIXFIED_NONE__" end')"
        max="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r 'if has("max") then (.max | tostring) else "__NIXFIED_NONE__" end')"
        values_json="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '(.values // []) | @json')"
        aliases_json="$(printf '%s' "$env_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '(.aliases // []) | @json')"
        [ -z "$name" ] && continue
        value="''${!name:-}"
        if [ -z "$value" ] && [ -n "$aliases_json" ] && [ "$aliases_json" != "[]" ]; then
          while IFS= read -r alias; do
            if [ -n "$alias" ] && [ -n "''${!alias:-}" ]; then
              value="''${!alias}"
              break
            fi
          done < <(printf '%s' "$aliases_json" | "$NIXFIED_CONTRACT_JQ" -r '.[]')
        fi

        if [ -z "$value" ] && [ "$default" != "$NIXFIED_CONTRACT_NONE" ]; then
          value="$default"
        fi

        if [ -z "$value" ] && [ "$required" = "true" ]; then
          _nixfied_contract_err "required env var missing name=$name"
          failures=1
          continue
        fi

        if [ -n "$value" ]; then
          if ! _nixfied_contract_validate_scalar "$type" "$value" "$values_json" "$min" "$max" "env:$name"; then
            failures=1
            continue
          fi
          export "$name=$value"
        fi
      done < <("$NIXFIED_CONTRACT_JQ" -c '(.env // [])[]' "$contract_file")

      if [ "$failures" -ne 0 ]; then
        return 2
      fi
      return 0
    }

    nixfied_contract_validate_args() {
      local contract_file="$1"
      shift || true

      local allow_unknown=""
      allow_unknown="$("$NIXFIED_CONTRACT_JQ" -r '.allowUnknownArgs // false' "$contract_file")"

      local failures=0
      local name="" kind="" long="" short="" type="" required="" values_json="" min="" max="" arg_spec_json=""
      local token="" value="" lookup_name=""
      local -a arg_names=()
      local -a positional_specs=()
      local -a positional_values=()
      local -a short_cluster=()
      declare -A spec_kind=()
      declare -A spec_type=()
      declare -A spec_required=()
      declare -A spec_values=()
      declare -A spec_min=()
      declare -A spec_max=()
      declare -A by_long=()
      declare -A by_short=()
      declare -A seen=()

      while IFS= read -r arg_spec_json; do
        [ -z "$arg_spec_json" ] && continue
        name="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '.name // ""')"
        kind="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '.kind // ""')"
        long="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '.long // ""')"
        short="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '.short // ""')"
        type="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '.type // ""')"
        required="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '(.required // false) | tostring')"
        values_json="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r '(.values // []) | @json')"
        min="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r 'if has("min") then (.min | tostring) else "__NIXFIED_NONE__" end')"
        max="$(printf '%s' "$arg_spec_json" | "$NIXFIED_CONTRACT_JQ" -r 'if has("max") then (.max | tostring) else "__NIXFIED_NONE__" end')"
        [ -z "$name" ] && continue
        if [ -z "$kind" ]; then
          if [ -n "$long" ] || [ -n "$short" ]; then
            kind="option"
          else
            kind="positional"
          fi
        fi
        if [ -z "$type" ]; then
          if [ "$kind" = "flag" ]; then
            type="bool"
          else
            type="string"
          fi
        fi
        arg_names+=("$name")
        spec_kind["$name"]="$kind"
        spec_type["$name"]="$type"
        spec_required["$name"]="$required"
        spec_values["$name"]="$values_json"
        spec_min["$name"]="$min"
        spec_max["$name"]="$max"
        if [ "$kind" = "positional" ]; then
          positional_specs+=("$name")
        fi
        if [ -n "$long" ]; then
          by_long["$long"]="$name"
        fi
        if [ -n "$short" ]; then
          by_short["$short"]="$name"
        fi
      done < <("$NIXFIED_CONTRACT_JQ" -c '(.args // [])[]' "$contract_file")

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
            lookup_name="''${by_long[$long]:-}"
            if [ -z "$lookup_name" ]; then
              if [ "$allow_unknown" = "true" ]; then
                continue
              fi
              _nixfied_contract_err "unknown option token=$long"
              failures=1
              continue
            fi
            if [ "''${spec_kind[$lookup_name]}" = "flag" ]; then
              _nixfied_contract_err "flag does not accept a value token=$long"
              failures=1
              continue
            fi
            if ! _nixfied_contract_validate_scalar \
              "''${spec_type[$lookup_name]}" \
              "$value" \
              "''${spec_values[$lookup_name]}" \
              "''${spec_min[$lookup_name]}" \
              "''${spec_max[$lookup_name]}" \
              "arg:$lookup_name"
            then
              failures=1
              continue
            fi
            seen["$lookup_name"]="1"
            export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=$value"
            ;;
          --*)
            lookup_name="''${by_long[$token]:-}"
            if [ -z "$lookup_name" ]; then
              if [ "$allow_unknown" = "true" ]; then
                continue
              fi
              _nixfied_contract_err "unknown option token=$token"
              failures=1
              continue
            fi

            if [ "''${spec_kind[$lookup_name]}" = "flag" ]; then
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
              "''${spec_type[$lookup_name]}" \
              "$value" \
              "''${spec_values[$lookup_name]}" \
              "''${spec_min[$lookup_name]}" \
              "''${spec_max[$lookup_name]}" \
              "arg:$lookup_name"
            then
              failures=1
              continue
            fi
            seen["$lookup_name"]="1"
            export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$lookup_name")=$value"
            ;;
          -*)
            lookup_name="''${by_short[$token]:-}"
            if [ -n "$lookup_name" ]; then
              if [ "''${spec_kind[$lookup_name]}" = "flag" ]; then
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
                  "''${spec_type[$lookup_name]}" \
                  "$value" \
                  "''${spec_values[$lookup_name]}" \
                  "''${spec_min[$lookup_name]}" \
                  "''${spec_max[$lookup_name]}" \
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
                lookup_name="''${by_short[$short_tok]:-}"
                if [ -z "$lookup_name" ] || [ "''${spec_kind[$lookup_name]}" != "flag" ]; then
                  cluster_ok=0
                  break
                fi
              done
              if [ "$cluster_ok" -eq 1 ]; then
                for short_tok in "''${short_cluster[@]}"; do
                  lookup_name="''${by_short[$short_tok]}"
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
      for pos_name in "''${positional_specs[@]}"; do
        if [ "$pos_index" -lt "''${#positional_values[@]}" ]; then
          value="''${positional_values[$pos_index]}"
          if ! _nixfied_contract_validate_scalar \
            "''${spec_type[$pos_name]}" \
            "$value" \
            "''${spec_values[$pos_name]}" \
            "''${spec_min[$pos_name]}" \
            "''${spec_max[$pos_name]}" \
            "arg:$pos_name"
          then
            failures=1
          else
            seen["$pos_name"]="1"
            export "NIXFIED_ARG_$(_nixfied_contract_sanitize_name "$pos_name")=$value"
          fi
          pos_index=$((pos_index + 1))
        elif [ "''${spec_required[$pos_name]}" = "true" ]; then
          _nixfied_contract_err "missing required positional arg name=$pos_name"
          failures=1
        fi
      done

      if [ "$pos_index" -lt "''${#positional_values[@]}" ] && [ "$allow_unknown" != "true" ]; then
        _nixfied_contract_err "unexpected positional args count=$(("''${#positional_values[@]}" - pos_index))"
        failures=1
      fi

      for name in "''${arg_names[@]}"; do
        if [ "''${spec_required[$name]}" = "true" ] && [ -z "''${seen[$name]:-}" ]; then
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
      local contract_file="$1"
      local exit_code="$2"

      if [ "$exit_code" -eq 0 ]; then
        return 0
      fi

      if "$NIXFIED_CONTRACT_JQ" -e --argjson code "$exit_code" '
        (.failureCodes // {})
        | to_entries
        | any(.value == $code)
      ' "$contract_file" >/dev/null 2>&1; then
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
    defaultFailureCodes
    validateAppContractErrors
    validateAppContract
    mkDefaultAppContract
    runtime
    ;
}
