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
      env = cfg.env or { };
      useDeps = cfg.useDeps or false;
      description = cfg.description or null;
    };

  commandApps = builtins.listToAttrs (
    map (name: {
      name = name;
      value = mkCommandApp name commands.${name};
    }) commandNames
  );

  helpLines = pkgs.lib.concatMapStringsSep "\n" (
    name:
    let
      desc = (commands.${name}.description or "");
    in
    "  ${name}  ${desc}"
  ) (pkgs.lib.sort (a: b: a < b) commandNames);

  moduleAppNames = builtins.attrNames moduleApps;
  moduleHelpLines =
    if moduleAppNames == [ ] then
      ""
    else
      "\n"
      + pkgs.lib.concatMapStringsSep "\n" (
        name:
        let
          app = moduleApps.${name};
          desc = (app.meta.description or "");
        in
        "  ${name}  ${desc}"
      ) (pkgs.lib.sort (a: b: a < b) moduleAppNames);

  helpScript = ''
        cat <<'EOF'
    ${project.project.name}

    Commands:
    ${helpLines}
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

    Edit nixfied/project/ to customize commands, ports, and modules.
    EOF
  '';

  autoHelp = lib.mkApp {
    name = "help";
    script = helpScript;
    env = { };
    useDeps = false;
    description = "Show available commands";
  };

in
commandApps // (if commands ? help then { } else { help = autoHelp; })
