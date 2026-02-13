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

  mkCommandApp =
    name: cfg:
    lib.mkApp {
      name = name;
      script = cfg.script or "";
      fixtures = cfg.fixtures or null;
      env = cfg.env or { };
      useDeps = cfg.useDeps or false;
      fixtureProfile = cfg.fixtureProfile or "default";
      description = if (cfg ? api) then (cfg.api.summary or null) else (cfg.description or null);
      meta = pkgs.lib.optionalAttrs (cfg ? api) { nixfied.api = cfg.api; };
    };

  commandApps = builtins.listToAttrs (
    map (name: {
      name = name;
      value = mkCommandApp name commands.${name};
    }) commandNames
  );

  helpApi = {
    version = 1;
    summary = "Show available commands";
    details = "Lists available commands, or shows detailed documentation for a single command.";
    usage = [
      "nix run .#help"
      "nix run .#help -- <command>"
    ];
    examples = [ "nix run .#help -- dev" ];
    category = "core";
  };

  moduleAppNames = builtins.attrNames moduleApps;
  sortNames = names: pkgs.lib.sort (a: b: a < b) names;
  getCommandCfg = name: if name == "help" && !(commands ? help) then { api = helpApi; } else commands.${name};
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
