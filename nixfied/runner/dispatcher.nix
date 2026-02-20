{
  pkgs,
  model,
  projectRoot,
  registry,
}:
let
  lib = pkgs.lib;
  workspaceMarkerPresent = builtins.pathExists "${projectRoot}/nixfied/.framework/.workspace";

  orchestrator = import ./orchestrator.nix {
    inherit
      pkgs
      model
      registry
      projectRoot
      ;
  };

  replayTool = registry.mkReplayApp;

  orchestratorProgram = "${orchestrator}/bin/nixfied-orchestrator";

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
          NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-task ${lib.escapeShellArg taskId} "$@"
        '';
      }
    ) viewAppNames
  );

  helpText = builtins.concatStringsSep "\n" model.views.help.lines;
  docsText = builtins.concatStringsSep "\n" model.views.docs.lines;

  helpFile = pkgs.writeText "nixfied-help.txt" "${helpText}\n";
  docsFile = pkgs.writeText "nixfied-docs.md" "${docsText}\n";

  frameworkProxyApps =
    if workspaceMarkerPresent then
      { }
    else
      {
        "framework::install" = mkApp "framework::install" ''
          NIXFIED_CALLER_PWD="$PWD" exec ${pkgs.nix}/bin/nix run github:willyrgf/nixfied#run-task --refresh -- task.framework.install "$@"
        '';

        "framework::upgrade" = mkApp "framework::upgrade" ''
          NIXFIED_CALLER_PWD="$PWD" exec ${pkgs.nix}/bin/nix run github:willyrgf/nixfied#run-task --refresh -- task.framework.upgrade "$@"
        '';
      };
in
{
  "run-task" = mkApp "run-task" ''
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-task <task-id> [-- ...]"
      exit 2
    fi
    NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-task "$@"
  '';

  "run-workflow" = mkApp "run-workflow" ''
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-workflow <workflow-id> [-- ...]"
      exit 2
    fi
    NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-workflow "$@"
  '';

  "run-workflow-parallel" = mkApp "run-workflow-parallel" ''
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-workflow-parallel <workflow-id> [-- ...]"
      exit 2
    fi
    NIXFIED_WORKFLOW_PARALLEL=1 NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-workflow "$@"
  '';

  "runs" = mkApp "runs" ''
    NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} runs "$@"
  '';

  "stop-run" = mkApp "stop-run" ''
    if [ "$#" -ne 1 ]; then
      echo "ERROR: usage: stop-run <run-id>"
      exit 2
    fi
    NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} stop-run "$@"
  '';

  "stop-all-runs" = mkApp "stop-all-runs" ''
    if [ "$#" -ne 0 ]; then
      echo "ERROR: usage: stop-all-runs"
      exit 2
    fi
    NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} stop-all-runs
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
// frameworkProxyApps
// {
  default =
    if builtins.hasAttr "help" taskApps then
      taskApps.help
    else
      mkApp "default-help" ''
        cat ${helpFile}
      '';
}
