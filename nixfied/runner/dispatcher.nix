{
  pkgs,
  model,
  projectRoot,
  registry,
}:
let
  lib = pkgs.lib;

  executor = import ./executor.nix {
    inherit
      pkgs
      model
      registry
      projectRoot
      ;
  };

  replayTool = registry.mkReplayApp;

  executorProgram = "${executor}/bin/nixfied-executor";

  mkApp =
    appName: body:
    let
      suffix = builtins.substring 0 10 (builtins.hashString "sha256" appName);
      binName = "nixfied-${suffix}";
      script = pkgs.writeShellScriptBin binName ''
        set -euo pipefail
        ${body}
      '';
    in
    {
      type = "app";
      program = "${script}/bin/${binName}";
    };

  viewApps = model.views.apps;
  viewAppNames = builtins.sort builtins.lessThan (builtins.attrNames viewApps);

  taskApps = builtins.listToAttrs (
    map (
      appName:
      let
        taskId = viewApps.${appName}.taskId;
      in
      {
        name = appName;
        value = mkApp appName ''
          NIXFIED_CALLER_PWD="$PWD" exec ${executorProgram} run-task ${lib.escapeShellArg taskId} "$@"
        '';
      }
    ) viewAppNames
  );

  helpText = builtins.concatStringsSep "\n" model.views.help.lines;
  docsText = builtins.concatStringsSep "\n" model.views.docs.lines;

  helpFile = pkgs.writeText "nixfied-help.txt" "${helpText}\n";
  docsFile = pkgs.writeText "nixfied-docs.md" "${docsText}\n";
in
{
  "run-task" = mkApp "run-task" ''
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-task <task-id> [-- ...]"
      exit 2
    fi
    NIXFIED_CALLER_PWD="$PWD" exec ${executorProgram} run-task "$@"
  '';

  "run-workflow" = mkApp "run-workflow" ''
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-workflow <workflow-id> [-- ...]"
      exit 2
    fi
    NIXFIED_CALLER_PWD="$PWD" exec ${executorProgram} run-workflow "$@"
  '';

  "help" = mkApp "help" ''
    cat ${helpFile}
  '';

  "docs" = mkApp "docs" ''
    cat ${docsFile}
  '';

  "registry::replay" = {
    type = "app";
    program = "${replayTool}/bin/nixfied-registry-replay";
  };
}
// taskApps
// {
  default = if builtins.hasAttr "help" taskApps then taskApps.help else mkApp "default-help" ''
    cat ${helpFile}
  '';
}
