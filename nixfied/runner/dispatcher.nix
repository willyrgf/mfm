{
  pkgs,
  model,
  projectRoot,
  registry,
}:
let
  lib = pkgs.lib;
  safeProjectRoot = builtins.unsafeDiscardStringContext (builtins.toString projectRoot);
  workspaceMarkerPresent = builtins.pathExists "${safeProjectRoot}/nixfied/.framework/.workspace";

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
  taskModels = model.tasks or { };

  renderTaskHelpText =
    taskId:
    let
      task = taskModels.${taskId};
      app = task.ui.app or { };
      argsContract = (((task.contract or { }).input or { }).args or { });
      usageLines = app.usage or [ ];
      exampleLines = app.examples or [ ];
      argSpecs = argsContract.spec or [ ];
      renderOptionLine =
        spec:
        let
          hasLong = (spec ? long) && spec.long != null && spec.long != "";
          hasShort = (spec ? short) && spec.short != null && spec.short != "";
          kind =
            if (spec ? kind) && spec.kind != null then
              spec.kind
            else if hasLong || hasShort then
              "option"
            else
              "positional";
          tokens = lib.filter (token: token != "") [
            (if hasLong then spec.long else "")
            (if hasShort then spec.short else "")
          ];
          valueType = if (spec ? type) && spec.type != null && spec.type != "" then spec.type else "value";
          valueSuffix = if kind == "option" then " <${valueType}>" else "";
          label = "${builtins.concatStringsSep ", " tokens}${valueSuffix}";
          description = spec.description or "";
        in
        if kind == "positional" || tokens == [ ] then
          null
        else if description == "" then
          "  ${label}"
        else
          "  ${label}: ${description}";
      optionLines = builtins.filter (line: line != null) (map renderOptionLine argSpecs) ++ [
        "  -h, --help: Show this help."
      ];
      appName = app.name or taskId;
      summary = task.summary or "";
      description = task.description or "";
    in
    builtins.concatStringsSep "\n" (
      [ "${appName} - ${summary}" ]
      ++ lib.optionals (description != "") [
        ""
        description
      ]
      ++ lib.optionals (usageLines != [ ]) (
        [
          ""
          "Usage:"
        ]
        ++ map (line: "  ${line}") usageLines
      )
      ++ lib.optionals (optionLines != [ ]) (
        [
          ""
          "Options:"
        ]
        ++ optionLines
      )
      ++ lib.optionals (exampleLines != [ ]) (
        [
          ""
          "Examples:"
        ]
        ++ map (line: "  ${line}") exampleLines
      )
    );

  mkTaskHelpFile =
    taskId:
    pkgs.writeText "nixfied-task-help-${builtins.substring 0 10 (builtins.hashString "sha256" taskId)}.txt" ''
      ${renderTaskHelpText taskId}
    '';

  frameworkInstallHelpFile = mkTaskHelpFile "task.framework.install";
  frameworkUpgradeHelpFile = mkTaskHelpFile "task.framework.upgrade";

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
  featuresText = builtins.concatStringsSep "\n" model.views.features.lines;

  helpFile = pkgs.writeText "nixfied-help.txt" "${helpText}\n";
  docsFile = pkgs.writeText "nixfied-docs.md" "${docsText}\n";
  featuresFile = pkgs.writeText "nixfied-features.txt" "${featuresText}\n";

  frameworkProxyApps =
    if workspaceMarkerPresent then
      { }
    else
      {
        "framework::install" = mkApp "framework::install" ''
          if [ "$#" -gt 0 ]; then
            case "$1" in
              --help|-h)
                cat ${frameworkInstallHelpFile}
                exit 0
                ;;
            esac
          fi
          NIXFIED_CALLER_PWD="$PWD" exec ${pkgs.nix}/bin/nix run github:willyrgf/nixfied/dev#framework::install --refresh -- "$@"
        '';

        "framework::upgrade" = mkApp "framework::upgrade" ''
          if [ "$#" -gt 0 ]; then
            case "$1" in
              --help|-h)
                cat ${frameworkUpgradeHelpFile}
                exit 0
                ;;
            esac
          fi
          NIXFIED_CALLER_PWD="$PWD" exec ${pkgs.nix}/bin/nix run github:willyrgf/nixfied/dev#framework::upgrade --refresh -- "$@"
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

  "features" = mkApp "features" ''
    cat ${featuresFile}
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
