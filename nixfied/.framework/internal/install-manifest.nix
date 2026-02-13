# Installer metadata for project template filters and framework helper apps.
{ pkgs }:

let
  lib = pkgs.lib;

  assertUnique =
    label: values:
    let
      uniq = lib.unique values;
    in
    if (builtins.length values) == (builtins.length uniq) then
      null
    else
      throw "Install manifest invalid: duplicate ${label}: ${builtins.concatStringsSep ", " values}";

  mkTemplate =
    {
      key,
      file,
      order,
      required ? false,
      aliases ? [ ],
      filterDisplay ? null,
    }:
    {
      inherit
        key
        file
        order
        required
        aliases
        filterDisplay
        ;
    };

  mkFrameworkHelper =
    {
      name,
      runner,
      summary,
      details,
      usage,
      env ? { },
    }:
    {
      inherit name runner env;
      api = {
        version = 1;
        inherit
          summary
          details
          usage
          ;
        category = "framework";
      };
    };

  templateSpecs = [
    {
      key = "conf";
      file = "conf.nix";
      order = 10;
      required = true;
    }
    {
      key = "dev";
      file = "dev.nix";
      order = 20;
    }
    {
      key = "test";
      file = "test.nix";
      order = 30;
    }
    {
      key = "prod";
      file = "prod.nix";
      order = 40;
      aliases = [ "build" ];
      # Keep help output stable while still accepting the canonical token ("prod").
      filterDisplay = [ "build" ];
    }
    {
      key = "quality";
      file = "quality.nix";
      order = 50;
    }
    {
      key = "ci";
      file = "ci.nix";
      order = 60;
    }
  ];
  projectTemplates = lib.sort (a: b: a.order < b.order) (map mkTemplate templateSpecs);

  templateKeys = map (t: t.key) projectTemplates;
  templateFiles = map (t: t.file) projectTemplates;
  templateFilterTokens = builtins.concatLists (
    map (t: [ t.key ] ++ (t.aliases or [ ])) projectTemplates
  );
  templateFilterDisplayTokens = builtins.concatLists (
    map (t: if t.filterDisplay == null then [ t.key ] else t.filterDisplay) projectTemplates
  );

  _templateKeysUnique = assertUnique "template key" templateKeys;
  _templateFilesUnique = assertUnique "template file" templateFiles;
  _templateFilterTokensUnique = assertUnique "template filter token" templateFilterTokens;

  _displayTokensValid =
    let
      unknown = builtins.filter (
        token: !(builtins.elem token templateFilterTokens)
      ) templateFilterDisplayTokens;
    in
    if unknown == [ ] then
      null
    else
      throw "Install manifest invalid: filterDisplay includes unknown tokens: ${builtins.concatStringsSep ", " unknown}";

  frameworkHelperSpecs = [
    {
      name = "install";
      runner = "install";
      summary = "Install Nixfied framework into a repository";
      details = "Installs the Nixfied framework into a target repository (writes flake.nix and nixfied/), optionally generating project scaffolding.";
      usage = [
        "nix run .#framework::install -- [--force] [--filter=...] [--reset-project] [--no-prompt-plan]"
      ];
    }
    {
      name = "upgrade";
      runner = "install";
      env = {
        NIXFIED_INSTALL_MODE = "upgrade";
      };
      summary = "Upgrade Nixfied framework in-place (preserving nixfied/project and nixfied/local by default)";
      details = "Upgrades the Nixfied framework in-place. By default it preserves nixfied/project and nixfied/local so project-specific configuration and extensions remain intact.";
      usage = [ "nix run .#framework::upgrade -- [--force] [--reset-project] [--no-prompt-plan]" ];
    }
    {
      name = "prompt-plan";
      runner = "prompt-plan";
      summary = "Generate Nixfied prompt plan from project docs";
      details = "Generates a prompt plan document (for agents) from the repository's project docs. This is framework-only and can be disabled via NIXFIED_PROMPT_PLAN=0.";
      usage = [ "nix run .#framework::prompt-plan -- [--force] [--output=PATH]" ];
    }
  ];
  frameworkHelpers = map mkFrameworkHelper frameworkHelperSpecs;

  helperNames = map (h: h.name) frameworkHelpers;
  helperRunners = map (h: h.runner) frameworkHelpers;
  allowedRunners = [
    "install"
    "prompt-plan"
  ];

  _helperNamesUnique = assertUnique "framework helper name" helperNames;

  _helperRunnersValid =
    let
      invalid = builtins.filter (runner: !(builtins.elem runner allowedRunners)) helperRunners;
    in
    if invalid == [ ] then
      null
    else
      throw "Install manifest invalid: unknown framework helper runners: ${builtins.concatStringsSep ", " invalid}";
in
{
  inherit
    projectTemplates
    frameworkHelpers
    templateFilterDisplayTokens
    ;
}
