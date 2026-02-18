# Core framework apps (dev/test/build/ci/check/help)
{
  pkgs,
  project,
  lib,
  moduleApps ? { },
}:

let
  commands = project.commands or { };
  commandNames = builtins.attrNames commands;
  discovery = lib.discovery;
  discoveryEnabled = discovery.enabled or true;
  discoveryStrict = discovery.strict or true;
  checkRefreshArg = discovery.refreshArg or "--refresh-discovery";
  checkRefreshArgSpec = {
    name = checkRefreshArg;
    description = "Regenerate docs/repo-index.json and docs/repo-map.md before running checks.";
  };
  checkRefreshArgContractSpec = {
    name = "refresh_discovery";
    kind = "flag";
    long = checkRefreshArg;
    type = "bool";
    required = false;
  };

  appendApiArg =
    args: arg:
    if builtins.any (item: (item.name or "") == (arg.name or "")) args then args else args ++ [ arg ];

  augmentCheckApi =
    api:
    if api == null then
      null
    else
      let
        args0 = api.args or [ ];
        contract0 = api.appContract or null;
        contractArgs0 = if contract0 == null then [ ] else (contract0.args or [ ]);
        hasContractArg = builtins.any (
          item: (item.long or "") == checkRefreshArg || (item.name or "") == "refresh_discovery"
        ) contractArgs0;
        contractArgs1 =
          if hasContractArg then contractArgs0 else contractArgs0 ++ [ checkRefreshArgContractSpec ];
        contract1 = if contract0 == null then null else contract0 // { args = contractArgs1; };
      in
      api
      // {
        args = appendApiArg args0 checkRefreshArgSpec;
      }
      // (pkgs.lib.optionalAttrs (contract1 != null) { appContract = contract1; });

  wrapCheckScript = baseScript: ''
    DISCOVERY_REFRESH=0
    PASS_ARGS=()

    while [ "$#" -gt 0 ]; do
      if [ "$1" = "${checkRefreshArg}" ]; then
        DISCOVERY_REFRESH=1
        shift
        continue
      fi

      PASS_ARGS+=("$1")
      shift
    done

    if [ "$DISCOVERY_REFRESH" -eq 1 ]; then
      ${discovery.tool}/bin/nixfied-discovery-index --refresh
    else
      if [ "${if discoveryStrict then "1" else "0"}" = "1" ]; then
        if ! ${discovery.tool}/bin/nixfied-discovery-index --verify; then
          exit 1
        fi
      else
        if ! ${discovery.tool}/bin/nixfied-discovery-index --verify; then
          log_warn "discovery drift detected but strict mode is disabled by project config."
        fi
      fi
    fi

    if [ "''${#PASS_ARGS[@]}" -gt 0 ]; then
      set -- "''${PASS_ARGS[@]}"
    else
      set --
    fi

    ${baseScript}
  '';

  normalizeCommandCfg =
    name: cfg:
    let
      api0 = cfg.api or null;
      api1 = if discoveryEnabled && name == "check" then augmentCheckApi api0 else api0;
      script0 = cfg.script or "";
      script1 = if discoveryEnabled && name == "check" then wrapCheckScript script0 else script0;
    in
    cfg
    // {
      script = script1;
    }
    // (pkgs.lib.optionalAttrs (api1 != null) { api = api1; });

  normalizedCommands = builtins.listToAttrs (
    map (name: {
      inherit name;
      value = normalizeCommandCfg name commands.${name};
    }) commandNames
  );

  mkCommandApp =
    name: cfg:
    lib.appApi.mkNixfiedApp {
      name = name;
      script = cfg.script or "";
      fixtures = cfg.fixtures or null;
      env = cfg.env or { };
      useDeps = cfg.useDeps or false;
      fixtureProfile = cfg.fixtureProfile or "default";
      api =
        if cfg ? api then
          cfg.api
        else
          throw "commands.${name}.api is required and must include appContract version=2";
    };

  commandApps = builtins.listToAttrs (
    map (name: {
      name = name;
      value = mkCommandApp name normalizedCommands.${name};
    }) commandNames
  );

  helpApi = lib.appApi.mkCommandApi {
    class = "passthrough";
    name = "help";
    summary = "Show available commands";
    details = "Lists available commands, or shows detailed documentation for a single command.";
    usage = [
      "nix run .#help"
      "nix run .#help -- <command>"
    ];
    examples = [ "nix run .#help -- dev" ];
    category = "core";
    args = [
      {
        name = "--help";
        description = "Show command list.";
      }
      {
        name = "command";
        description = "Optional command name to render full docs.";
      }
    ];
  };

  moduleAppNames = builtins.attrNames moduleApps;
  sortNames = names: pkgs.lib.sort (a: b: a < b) names;
  getCommandCfg =
    name:
    if name == "help" && !(commands ? help) then { api = helpApi; } else normalizedCommands.${name};
  getCommandApi = name: (getCommandCfg name).api or null;
  getModuleApi = name: (((moduleApps.${name}.meta or { }).nixfied or { }).api or null);
  getCommandSummary =
    name:
    let
      cfg = getCommandCfg name;
      api = getCommandApi name;
    in
    if api != null then (api.summary or "") else (cfg.description or "");
  getModuleSummary =
    name:
    let
      app = moduleApps.${name};
      api = getModuleApi name;
    in
    if api != null then (api.summary or "") else (app.meta.description or "");
  renderDetailCasesFor =
    names: apiFor:
    pkgs.lib.concatMapStringsSep "\n" (
      name:
      let
        api = apiFor name;
      in
      if api == null then "" else renderApiDoc name api
    ) (sortNames names);

  commandNamesForHelp = commandNames ++ (if commands ? help then [ ] else [ "help" ]);
  commandHelpLines = pkgs.lib.concatMapStringsSep "\n" (
    name:
    let
      summary = getCommandSummary name;
    in
    "  ${name}  ${summary}"
  ) (sortNames commandNamesForHelp);

  moduleHelpLines =
    if moduleAppNames == [ ] then
      ""
    else
      "\n"
      + pkgs.lib.concatMapStringsSep "\n" (
        name:
        let
          summary = getModuleSummary name;
        in
        "  ${name}  ${summary}"
      ) (sortNames moduleAppNames);

  renderApiDoc =
    name: api:
    let
      usageLines = pkgs.lib.concatMapStringsSep "\n" (u: "  " + u) (api.usage or [ ]);
      examplesBlock =
        if (api ? examples) && (api.examples or [ ]) != [ ] then
          "\n\nExamples:\n" + pkgs.lib.concatMapStringsSep "\n" (e: "  " + e) (api.examples or [ ])
        else
          "";
      argsBlock =
        if (api ? args) && (api.args or [ ]) != [ ] then
          "\n\nArgs:\n"
          + pkgs.lib.concatMapStringsSep "\n" (a: "  " + (a.name or "") + "  " + (a.description or "")) (
            api.args or [ ]
          )
        else
          "";
      envBlock =
        if (api ? env) && (api.env or [ ]) != [ ] then
          "\n\nEnvironment:\n"
          + pkgs.lib.concatMapStringsSep "\n" (e: "  " + (e.name or "") + "  " + (e.description or "")) (
            api.env or [ ]
          )
        else
          "";
      details = api.details or "";
    in
    ''
                ${name})
                  cat <<'EOF'
      ${name}  ${api.summary}

      Usage:
      ${usageLines}

      Details:
      ${details}${examplesBlock}${argsBlock}${envBlock}
      EOF
                  ;;
    '';

  commandDetailCases = renderDetailCasesFor commandNamesForHelp getCommandApi;
  moduleDetailCases = renderDetailCasesFor moduleAppNames getModuleApi;

  detailCases = commandDetailCases + "\n" + moduleDetailCases;

  helpScript = ''
    if [ "$#" -gt 0 ]; then
      case "''${1-}" in
        --help|-h)
          # Fall through to list output.
          ;;
        *)
          CMD="''${1-}"
          case "$CMD" in
    ${detailCases}
            *)
              echo "Unknown command: $CMD" >&2
              echo "" >&2
              echo "Run: nix run .#help" >&2
              exit 1
              ;;
          esac
          exit 0
          ;;
      esac
    fi

    cat <<'EOF'
    ${project.project.name}

    Commands:
    ${commandHelpLines}
    ${
      if moduleHelpLines != "" then
        ''

          Module Apps:${moduleHelpLines}''
      else
        ""
    }

    Environment:
      ${project.project.envVar}  Environment name (${pkgs.lib.concatStringsSep "|" (builtins.attrNames project.envs)})
      ${project.project.slotVar}  Slot number (0-9)
      ${pkgs.lib.optionalString (project.project.slotVar != "NIXFIED_ENV") ''
        NIXFIED_ENV  Slot number (alias for ${project.project.slotVar})
      ''}

    Edit nixfied/project/ to customize commands, ports, and modules.
    EOF
  '';

  autoHelp = lib.appApi.mkNixfiedApp {
    name = "help";
    script = helpScript;
    env = { };
    useDeps = false;
    api = helpApi;
  };

in
commandApps // (if commands ? help then { } else { help = autoHelp; })
