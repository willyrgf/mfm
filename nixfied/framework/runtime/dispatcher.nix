{
  pkgs,
  model,
  selectionIndex,
  services,
  runtimeHash ? model.identity.evalHash,
  projectRoot,
  registry,
  frameworkSourceFlakeRef ? null,
  serviceApps ? { },
  serviceHookEnv ? { },
}:
let
  lib = pkgs.lib;
  mkShellApp = import ../core/mk-shell-app.nix { inherit pkgs; };
  shellCommon = import ../core/shell-common.nix { inherit pkgs; };
  workspaceMarker = import ../workspace-marker.nix;
  workspaceMarkerPresent = workspaceMarker.isPresent projectRoot;

  orchestrator = import ./orchestrator.nix {
    inherit
      pkgs
      model
      selectionIndex
      services
      runtimeHash
      registry
      projectRoot
      serviceHookEnv
      ;
  };

  orchestratorProgram = "${orchestrator}/bin/nixfied-orchestrator";

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
  frameworkSourceFlakeRefValue =
    if frameworkSourceFlakeRef != null && frameworkSourceFlakeRef != "" then
      frameworkSourceFlakeRef
    else
      "github:willyrgf/nixfied/dev";
  frameworkSourceFlakeRefShell = lib.escapeShellArg frameworkSourceFlakeRefValue;
  proxyFrameworkCommand = taskId: helpFile: ''
    if [ "$#" -gt 0 ]; then
      case "$1" in
        --help|-h)
          cat ${helpFile}
          exit 0
          ;;
      esac
    fi

    framework_source_flake_ref="''${NIXFIED_FRAMEWORK_SOURCE_FLAKE:-${frameworkSourceFlakeRefShell}}"
    NIXFIED_CALLER_PWD="$PWD" exec ${pkgs.nix}/bin/nix run "''${framework_source_flake_ref}#run-task" --refresh -- ${lib.escapeShellArg taskId} "$@"
  '';

  taskApps = builtins.listToAttrs (
    map (
      appName:
      let
        taskId = viewApps.${appName}.taskId;
      in
      {
        name = appName;
        value = mkShellApp {
          inherit appName;
          body = ''
            NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-task ${lib.escapeShellArg taskId} "$@"
          '';
        };
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
        "framework::install" = mkShellApp {
          appName = "framework::install";
          body = proxyFrameworkCommand "task.framework.install" frameworkInstallHelpFile;
        };

        "framework::upgrade" = mkShellApp {
          appName = "framework::upgrade";
          body = proxyFrameworkCommand "task.framework.upgrade" frameworkUpgradeHelpFile;
        };
      };
in
{
  "run-task" = mkShellApp {
    appName = "run-task";
    body = ''
      ${shellCommon}
      if [ "$#" -lt 1 ]; then
        nixfied_exit_usage "usage: run-task <task-id> [-- ...]"
      fi
      NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-task "$@"
    '';
  };

  "run-workflow" = mkShellApp {
    appName = "run-workflow";
    body = ''
      ${shellCommon}
      if [ "$#" -lt 1 ]; then
        nixfied_exit_usage "usage: run-workflow <workflow-id> [-- ...]"
      fi
      NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-workflow "$@"
    '';
  };

  "run-workflow-parallel" = mkShellApp {
    appName = "run-workflow-parallel";
    body = ''
      ${shellCommon}
      if [ "$#" -lt 1 ]; then
        nixfied_exit_usage "usage: run-workflow-parallel <workflow-id> [-- ...]"
      fi
      NIXFIED_WORKFLOW_PARALLEL=1 NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} run-workflow "$@"
    '';
  };

  "runs" = mkShellApp {
    appName = "runs";
    body = ''
      NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} runs "$@"
    '';
  };

  "stop-run" = mkShellApp {
    appName = "stop-run";
    body = ''
      ${shellCommon}
      if [ "$#" -ne 1 ]; then
        nixfied_exit_usage "usage: stop-run <run-id>"
      fi
      NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} stop-run "$@"
    '';
  };

  "stop-all-runs" = mkShellApp {
    appName = "stop-all-runs";
    body = ''
      ${shellCommon}
      if [ "$#" -ne 0 ]; then
        nixfied_exit_usage "usage: stop-all-runs"
      fi
      NIXFIED_CALLER_PWD="$PWD" exec ${orchestratorProgram} stop-all-runs
    '';
  };

  "help" = mkShellApp {
    appName = "help";
    body = ''
      cat ${helpFile}
    '';
  };

  "docs" = mkShellApp {
    appName = "docs";
    body = ''
      cat ${docsFile}
    '';
  };

  "features" = mkShellApp {
    appName = "features";
    body = ''
      cat ${featuresFile}
    '';
  };
}
// taskApps
// serviceApps
// frameworkProxyApps
// {
  default =
    if builtins.hasAttr "help" taskApps then
      taskApps.help
    else
      mkShellApp {
        appName = "default-help";
        body = ''
          cat ${helpFile}
        '';
      };
}
