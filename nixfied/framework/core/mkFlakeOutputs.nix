{
  pkgs,
  system,
  projectRoot,
  projectModules,
  extraModules ? [ ],
  localOverrides ? [ ],
  frameworkSourceRevision ? import ./framework-revision.nix {
    sourcePath = ../../.;
    metadataPath = ../../VENDORED.txt;
  },
}:
let
  lib = pkgs.lib;
  mkShellApp = import ./mk-shell-app.nix { inherit pkgs; };
  skipPolicy = import ../runtime/helpers/skip-policy.nix { inherit pkgs; };
  canonical = import ./canonical.nix { inherit lib; };
  workspaceMarker = import ../workspace-marker.nix;
  registry = import ../runtime/registry { inherit pkgs; };

  frameworkRoot = ../../.;
  frameworkRepoRoot = ../../../.;
  frameworkUtilityCommand = import ../install/wrapper-command.nix {
    inherit
      pkgs
      frameworkSourceRevision
      ;
    sourceRoot = frameworkRoot;
    repoRoot = frameworkRepoRoot;
  };

  compiledCore = import ./mkCompiledCore.nix {
    inherit
      pkgs
      system
      projectRoot
      projectModules
      extraModules
      localOverrides
      frameworkSourceRevision
      ;
  };

  coreSurfaces = import ./mkCoreSurfaces.nix {
    inherit
      pkgs
      canonical
      ;
    compiledCore = compiledCore;
  };

  toAbsString =
    value:
    if builtins.isPath value then
      builtins.toString value
    else if builtins.isString value && lib.hasPrefix "/" value then
      value
    else
      throw "nixfied.mkFlakeOutputs requires path values or absolute path strings for project modules and overrides";

  isPathLike = value: builtins.isPath value || (builtins.isString value && lib.hasPrefix "/" value);

  pathWithin = root: path: path == root || lib.hasPrefix "${root}/" path;

  relativeTo = root: path: if path == root then "" else lib.removePrefix "${root}/" path;

  projectRootAbs = toAbsString projectRoot;
  frameworkRootAbs = builtins.toString frameworkRoot;
  nixpkgsPathAbs = builtins.toString pkgs.path;

  encodeModuleSpec =
    value:
    let
      absPath = toAbsString value;
    in
    if pathWithin projectRootAbs absPath then
      {
        scope = "project";
        path = relativeTo projectRootAbs absPath;
      }
    else if pathWithin frameworkRootAbs absPath then
      {
        scope = "framework";
        path = relativeTo frameworkRootAbs absPath;
      }
    else
      {
        scope = "absolute";
        path = absPath;
      };

  launchersSupported = builtins.all isPathLike (projectModules ++ extraModules ++ localOverrides);

  projectModuleSpecsJson = builtins.toJSON (map encodeModuleSpec projectModules);
  extraModuleSpecsJson = builtins.toJSON (map encodeModuleSpec extraModules);
  localOverrideSpecsJson = builtins.toJSON (map encodeModuleSpec localOverrides);

  workspaceMarkerPresent = workspaceMarker.isPresent projectRoot;
  launcherMetadata = import ./mkLauncherMetadata.nix {
    inherit
      lib
      workspaceMarkerPresent
      ;
    model = compiledCore.model;
    selectionIndex = compiledCore.selectionIndex;
    serviceSurfaceCatalog = compiledCore.serviceSurfaceCatalog;
  };
  serviceNames = launcherMetadata.enabledServices;
  runtimeControlAppNames = launcherMetadata.runtimeControlAppNames;
  viewWrappedAppNames = launcherMetadata.viewWrappedAppNames;
  serviceWrappedAppNames = launcherMetadata.serviceWrappedAppNames;
  runtimeAppNames = launcherMetadata.runtimeAppNames;
  internalBaseTargetName =
    appName: "base${builtins.substring 0 10 (builtins.hashString "sha256" appName)}";

  renderKnownServices =
    if serviceNames == [ ] then
      "  (none)"
    else
      builtins.concatStringsSep "\n" (map (serviceName: "  - ${serviceName}") serviceNames);

  renderKnownServicesArray = builtins.concatStringsSep "\n" (
    map (serviceName: "    ${lib.escapeShellArg serviceName}") serviceNames
  );

  taskIds = launcherMetadata.taskIds;
  appModels = compiledCore.model.apps or { };

  taskAppIds =
    taskId:
    builtins.sort builtins.lessThan (
      builtins.filter (
        appId:
        let
          app = appModels.${appId};
        in
        (app.kind or "") == "taskRef" && (app.taskId or "") == taskId
      ) (builtins.attrNames appModels)
    );

  preferredTaskApp =
    taskId:
    let
      appIds = taskAppIds taskId;
    in
    if appIds == [ ] then null else appModels.${builtins.head appIds};

  normalizeTaskArgSpec =
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
    in
    {
      inherit kind;
      long = if hasLong then spec.long else "";
      short = if hasShort then spec.short else "";
      type = if (spec ? type) && spec.type != null && spec.type != "" then toString spec.type else "";
      values = if (spec ? values) && spec.values != null then map toString spec.values else [ ];
      description = if (spec ? description) && spec.description != null then spec.description else "";
    };

  formatTaskArgHelpLine =
    spec:
    let
      tokens =
        (lib.optionals (spec.short != "") [ spec.short ])
        ++ (lib.optionals (spec.long != "") [ spec.long ]);
      valueLabel =
        if spec.kind != "option" then
          ""
        else if spec.values != [ ] then
          "<${builtins.concatStringsSep "|" spec.values}>"
        else if spec.type != "" then
          "<${spec.type}>"
        else
          "<value>";
      descriptionSuffix = if spec.description != "" then ": ${spec.description}" else "";
    in
    "  ${builtins.concatStringsSep ", " tokens}${
        lib.optionalString (valueLabel != "") " ${valueLabel}"
      }${descriptionSuffix}";

  renderTaskHelpText =
    taskId:
    let
      task = compiledCore.model.tasks.${taskId};
      app = preferredTaskApp taskId;
      argsContract = (((task.contract or { }).input or { }).args or { });
      specs = map normalizeTaskArgSpec (argsContract.spec or [ ]);
      displayName = if app == null then taskId else app.id or taskId;
      usageLines =
        let
          configuredUsage = if app == null then [ ] else app.usage or [ ];
        in
        if configuredUsage != [ ] then configuredUsage else [ "nix run .#run-task -- ${taskId} [-- ...]" ];
      exampleLines = if app == null then [ ] else app.examples or [ ];
      optionLines = map formatTaskArgHelpLine specs ++ [
        "  -h, --help: Show this help."
      ];
      summary = if app == null then task.summary or "" else app.summary or task.summary or "";
      description =
        if app == null then task.description or "" else app.description or task.description or "";
    in
    builtins.concatStringsSep "\n" (
      [ "${displayName} - ${summary}" ]
      ++ lib.optionals (description != "") [
        ""
        description
      ]
      ++ [
        ""
        "Usage:"
      ]
      ++ map (line: "  ${line}") usageLines
      ++ [
        ""
        "Options:"
      ]
      ++ optionLines
      ++ lib.optionals (exampleLines != [ ]) (
        [
          ""
          "Examples:"
        ]
        ++ map (line: "  ${line}") exampleLines
      )
    );

  taskHelpFiles = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = pkgs.writeText "nixfied-task-help-${builtins.substring 0 10 (builtins.hashString "sha256" taskId)}.txt" ''
        ${renderTaskHelpText taskId}
      '';
    }) taskIds
  );

  mkStaticHelpFile =
    name: text:
    pkgs.writeText "nixfied-help-${builtins.substring 0 10 (builtins.hashString "sha256" name)}.txt" ''
      ${text}
    '';

  dispatcherHelpFiles = {
    "run-task" = mkStaticHelpFile "run-task" ''
      run-task - Run a compiled task by id

      Usage:
        nix run .#run-task -- <task-id> [-- ...]

      Options:
        -h, --help: Show this help.
    '';

    "run-workflow" = mkStaticHelpFile "run-workflow" ''
      run-workflow - Run a compiled workflow by id

      Usage:
        nix run .#run-workflow -- <workflow-id> [-- ...]

      Options:
        -h, --help: Show this help.
    '';

    "run-workflow-parallel" = mkStaticHelpFile "run-workflow-parallel" ''
      run-workflow-parallel - Run a compiled workflow by id with parallel execution enabled

      Usage:
        nix run .#run-workflow-parallel -- <workflow-id> [-- ...]

      Options:
        -h, --help: Show this help.
    '';
  };

  runtimeControlHelpFiles = {
    "runs" = mkStaticHelpFile "runs" ''
      runs - List runs or show one run by id

      Usage:
        nix run .#runs
        nix run .#runs -- <run-id>

      Options:
        -h, --help: Show this help.
    '';

    "stop-run" = mkStaticHelpFile "stop-run" ''
      stop-run - Stop one running or queued run

      Usage:
        nix run .#stop-run -- <run-id>

      Options:
        -h, --help: Show this help.
    '';

    "stop-all-runs" = mkStaticHelpFile "stop-all-runs" ''
      stop-all-runs - Stop all running or queued runs

      Usage:
        nix run .#stop-all-runs

      Options:
        -h, --help: Show this help.
    '';
  };

  renderTaskHelpCases = builtins.concatStringsSep "\n" (
    map (taskId: ''
      ${lib.escapeShellArg taskId})
        cat ${lib.escapeShellArg (builtins.toString taskHelpFiles.${taskId})}
        return 0
        ;;
    '') taskIds
  );

  workflowIds = launcherMetadata.workflowIds;
  workflowModesByFamily = launcherMetadata.workflowModesByFamily;
  workflowFamilies = launcherMetadata.workflowFamilies;
  taskBaseClosureCsvById = launcherMetadata.taskBaseClosureCsvById;
  taskRunnerWorkflowIdById = launcherMetadata.taskRunnerWorkflowIdById;
  workflowClosureCsvById = launcherMetadata.workflowClosureCsvById;
  serviceSetServicesCsvById = builtins.mapAttrs (
    _: serviceSet: builtins.concatStringsSep "," (serviceSet.services.all or [ ])
  ) (compiledCore.serviceSets or { });

  mkSelectorAwareLauncher =
    appName:
    let
      launcherViewApp =
        if builtins.hasAttr appName (compiledCore.model.views.apps or { }) then
          compiledCore.model.views.apps.${appName}
        else
          null;
      launcherTaskId =
        if launcherViewApp != null then
          if (launcherViewApp.taskId or null) == null then "" else launcherViewApp.taskId
        else
          "";
      serviceAppMatch = builtins.match "^svc::([^:]+)::.+$" appName;
      launcherServiceName = if serviceAppMatch == null then "" else builtins.elemAt serviceAppMatch 0;
      launcherServiceSetId =
        if launcherViewApp != null then
          if (launcherViewApp.serviceSetId or null) == null then "" else launcherViewApp.serviceSetId
        else
          "";
      viewHelpFile =
        if
          launcherViewApp != null && (launcherViewApp.taskId or null) != null && launcherViewApp.taskId != ""
        then
          builtins.toString taskHelpFiles.${launcherViewApp.taskId}
        else
          "";
      dispatcherHelpFile =
        if builtins.hasAttr appName dispatcherHelpFiles then
          builtins.toString dispatcherHelpFiles.${appName}
        else
          "";
    in
    mkShellApp {
      inherit appName;
      binPrefix = "nixfied-launch";
      body = ''
                ${skipPolicy.skipPolicyFunctions}

                known_services=(
                ${renderKnownServicesArray}
                )

                render_launcher_help() {
                  cat <<'NIXFIED_LAUNCHER_HELP'
        ${appName} accepts framework launcher selectors before normal app arguments.

        Launcher options:
          --exclude-services <csv>: Exclude services from the compiled graph.
          --launcher-help: Show this help.

        Selector parsing stops at the first non-launcher argument or `--`, and the
        remaining arguments are forwarded unchanged to the selected app.

        Compatibility:
          Truthy SKIP_<SERVICE> env vars are folded into the excluded set as
          launcher sugar. The explicit --exclude-services selector is the
          canonical compile-time interface.

        Known services:
        ${renderKnownServices}
        NIXFIED_LAUNCHER_HELP
                }

                service_is_known() {
                  local target="$1"
                  local service=""
                  for service in "''${known_services[@]}"; do
                    if [ "$service" = "$target" ]; then
                      return 0
                    fi
                  done
                  return 1
                }

                append_excluded_service() {
                  local service="$1"
                  if [ -z "$service" ]; then
                    return 0
                  fi

                  if ! service_is_known "$service"; then
                    echo "ERROR: unknown service '$service' in --exclude-services" >&2
                    exit 2
                  fi

                  requested_excluded_services+=("$service")
                }

                parse_excluded_services_csv() {
                  local csv="$1"
                  local token=""
                  local old_ifs="$IFS"
                  local csv_parts=()

                  IFS=','
                  read -r -a csv_parts <<< "$csv"
                  IFS="$old_ifs"

                  for token in "''${csv_parts[@]}"; do
                    token="$(printf '%s' "$token" | ${pkgs.coreutils}/bin/tr -d '[:space:]')"
                    [ -n "$token" ] || continue
                    append_excluded_service "$token"
                  done
                }

                build_excluded_services_csv() {
                  if [ "''${#requested_excluded_services[@]}" -eq 0 ]; then
                    printf '%s' ""
                    return 0
                  fi

                  printf '%s\n' "''${requested_excluded_services[@]}" \
                    | ${pkgs.coreutils}/bin/sort -u \
                    | ${pkgs.coreutils}/bin/paste -sd, -
                }

                find_flake_root() {
                  local dir="''${NIXFIED_FLAKE_ROOT:-''${NIXFIED_CALLER_PWD:-$PWD}}"
                  while [ "$dir" != "/" ]; do
                    if [ -f "$dir/flake.nix" ]; then
                      printf '%s' "$dir"
                      return 0
                    fi
                    dir="$(${pkgs.coreutils}/bin/dirname "$dir")"
                  done

                  echo "ERROR: unable to locate flake root from ''${NIXFIED_FLAKE_ROOT:-''${NIXFIED_CALLER_PWD:-$PWD}}" >&2
                  exit 3
                }

                forwarded_args_request_help() {
                  local arg=""
                  while [ "$#" -gt 0 ]; do
                    arg="$1"
                    shift

                    case "$arg" in
                      --help|-h)
                        return 0
                        ;;
                      --)
                        return 1
                        ;;
                    esac
                  done

                  return 1
                }

                forwarded_args_only_help_flag() {
                  if [ "$#" -ne 1 ]; then
                    return 1
                  fi

                  case "$1" in
                    --help|-h)
                      return 0
                      ;;
                    *)
                      return 1
                      ;;
                  esac
                }

                print_fast_task_help() {
                  local task_id="$1"
                  case "$task_id" in
        ${renderTaskHelpCases}
                    *)
                      return 1
                      ;;
                  esac
                }

                merge_services_csv() {
                  local csv=""
                  local token=""
                  local old_ifs="$IFS"
                  local csv_parts=()

                  (
                    for csv in "$@"; do
                      [ -n "$csv" ] || continue
                      IFS=','
                      read -r -a csv_parts <<< "$csv"
                      IFS="$old_ifs"

                      for token in "''${csv_parts[@]}"; do
                        token="$(printf '%s' "$token" | ${pkgs.coreutils}/bin/tr -d '[:space:]')"
                        [ -n "$token" ] || continue
                        printf '%s\n' "$token"
                      done
                    done
                  ) | ${pkgs.coreutils}/bin/sort -u | ${pkgs.coreutils}/bin/paste -sd, -
                }

                workflow_family_from_id() {
                  local workflow_id="$1"
                  if [[ "$workflow_id" =~ ^workflow\.([^.]+)\..+$ ]]; then
                    printf '%s' "''${BASH_REMATCH[1]}"
                    return 0
                  fi
                  return 1
                }

                workflow_id_exists() {
                  local workflow_id="$1"
                  case "$workflow_id" in
        ${builtins.concatStringsSep "\n" (
          map (workflowId: ''
            ${lib.escapeShellArg workflowId})
              return 0
              ;;
          '') workflowIds
        )}
                    *)
                      return 1
                      ;;
                  esac
                }

                workflow_modes_for_family() {
                  local family="$1"
                  case "$family" in
        ${builtins.concatStringsSep "\n" (
          map (family: ''
                ${lib.escapeShellArg family})
            ${builtins.concatStringsSep "\n" (
              map (mode: "              printf '%s\\n' ${lib.escapeShellArg mode}") (
                workflowModesByFamily.${family} or [ ]
              )
            )}
                  return 0
                  ;;
          '') workflowFamilies
        )}
                    *)
                      return 0
                      ;;
                  esac
                }

                workflow_mode_is_simple_shorthand() {
                  local mode="$1"
                  [[ "$mode" =~ ^[a-z0-9-]+$ ]]
                }

                workflow_simple_shorthand_exists_for_family() {
                  local workflow_id="$1"
                  local candidate="$2"
                  local family=""
                  local mode=""

                  family="$(workflow_family_from_id "$workflow_id" || true)"
                  [ -n "$family" ] || return 1

                  while IFS= read -r mode; do
                    if [ "$mode" = "$candidate" ] && workflow_mode_is_simple_shorthand "$mode"; then
                      return 0
                    fi
                  done < <(workflow_modes_for_family "$family")

                  return 1
                }

                workflow_resolve_mode_id() {
                  local workflow_id="$1"
                  local mode_override="$2"
                  local family=""
                  local candidate=""

                  if [ -z "$mode_override" ]; then
                    printf '%s' "$workflow_id"
                    return 0
                  fi

                  family="$(workflow_family_from_id "$workflow_id" || true)"
                  [ -n "$family" ] || return 1

                  candidate="workflow.$family.$mode_override"
                  if workflow_id_exists "$candidate"; then
                    printf '%s' "$candidate"
                    return 0
                  fi

                  return 1
                }

                workflow_mode_override_from_args() {
                  local workflow_id="$1"
                  shift

                  local parse_options=1
                  local arg=""
                  local shorthand_mode=""
                  local mode_override=""

                  while [ "$#" -gt 0 ]; do
                    arg="$1"
                    shift

                    if [ "$parse_options" -eq 0 ]; then
                      continue
                    fi

                    case "$arg" in
                      --mode)
                        if [ "$#" -lt 1 ]; then
                          break
                        fi
                        mode_override="$1"
                        shift
                        ;;
                      --mode=*)
                        mode_override="''${arg#--mode=}"
                        ;;
                      --summary)
                        ;;
                      --)
                        parse_options=0
                        ;;
                      --*)
                        shorthand_mode="''${arg#--}"
                        if workflow_simple_shorthand_exists_for_family "$workflow_id" "$shorthand_mode"; then
                          mode_override="$shorthand_mode"
                        fi
                        ;;
                    esac
                  done

                  printf '%s' "$mode_override"
                }

                dispatcher_task_base_closure_services_csv() {
                  local task_id="$1"
                  case "$task_id" in
        ${builtins.concatStringsSep "\n" (
          map (taskId: ''
            ${lib.escapeShellArg taskId})
              printf '%s' ${lib.escapeShellArg (taskBaseClosureCsvById.${taskId} or "")}
              return 0
              ;;
          '') taskIds
        )}
                    *)
                      printf '%s' ""
                      return 0
                      ;;
                  esac
                }

                dispatcher_task_runner_workflow_id() {
                  local task_id="$1"
                  case "$task_id" in
        ${builtins.concatStringsSep "\n" (
          map (taskId: ''
            ${lib.escapeShellArg taskId})
              printf '%s' ${lib.escapeShellArg (taskRunnerWorkflowIdById.${taskId} or "")}
              return 0
              ;;
          '') taskIds
        )}
                    *)
                      printf '%s' ""
                      return 0
                      ;;
                  esac
                }

                workflow_closure_services_csv() {
                  local workflow_id="$1"
                  case "$workflow_id" in
        ${builtins.concatStringsSep "\n" (
          map (workflowId: ''
            ${lib.escapeShellArg workflowId})
              printf '%s' ${lib.escapeShellArg (workflowClosureCsvById.${workflowId} or "")}
              return 0
              ;;
          '') workflowIds
        )}
                    *)
                      printf '%s' ""
                      return 0
                      ;;
                  esac
                }

                task_selected_services_csv_for_launcher() {
                  local task_id="$1"
                  shift

                  local task_base_csv=""
                  local task_workflow_id=""
                  local mode_override=""
                  local resolved_workflow_id=""
                  local workflow_csv=""

                  task_base_csv="$(dispatcher_task_base_closure_services_csv "$task_id")"
                  task_workflow_id="$(dispatcher_task_runner_workflow_id "$task_id")"

                  if [ -z "$task_workflow_id" ]; then
                    printf '%s' "$task_base_csv"
                    return 0
                  fi

                  mode_override="$(workflow_mode_override_from_args "$task_workflow_id" "$@")"
                  resolved_workflow_id="$task_workflow_id"
                  if resolved_workflow_id_candidate="$(workflow_resolve_mode_id "$task_workflow_id" "$mode_override" 2>/dev/null)"; then
                    resolved_workflow_id="$resolved_workflow_id_candidate"
                  fi
                  workflow_csv="$(workflow_closure_services_csv "$resolved_workflow_id")"
                  merge_services_csv "$task_base_csv" "$workflow_csv"
                }

                workflow_selected_services_csv_for_launcher() {
                  local workflow_id="$1"
                  shift

                  local mode_override=""
                  local resolved_workflow_id="$workflow_id"

                  mode_override="$(workflow_mode_override_from_args "$workflow_id" "$@")"
                  if resolved_workflow_id_candidate="$(workflow_resolve_mode_id "$workflow_id" "$mode_override" 2>/dev/null)"; then
                    resolved_workflow_id="$resolved_workflow_id_candidate"
                  fi

                  workflow_closure_services_csv "$resolved_workflow_id"
                }

                launcher_selected_services_csv() {
                  local task_id=""
                  local workflow_id=""

                  if [ -n ${lib.escapeShellArg launcherTaskId} ]; then
                    task_selected_services_csv_for_launcher ${lib.escapeShellArg launcherTaskId} "$@"
                    return 0
                  fi

                  if [ -n ${lib.escapeShellArg launcherServiceName} ]; then
                    printf '%s' ${lib.escapeShellArg launcherServiceName}
                    return 0
                  fi

                  if [ -n ${lib.escapeShellArg launcherServiceSetId} ]; then
                    printf '%s' ${
                      lib.escapeShellArg (serviceSetServicesCsvById.${launcherServiceSetId} or "")
                    }
                    return 0
                  fi

                  case ${lib.escapeShellArg appName} in
                    run-task)
                      task_id="''${1:-}"
                      if [ -z "$task_id" ]; then
                        printf '%s' ""
                        return 0
                      fi
                      shift
                      task_selected_services_csv_for_launcher "$task_id" "$@"
                      ;;
                    run-workflow|run-workflow-parallel)
                      workflow_id="''${1:-}"
                      if [ -z "$workflow_id" ]; then
                        printf '%s' ""
                        return 0
                      fi
                      shift
                      workflow_selected_services_csv_for_launcher "$workflow_id" "$@"
                      ;;
                    *)
                      printf '%s' ""
                      ;;
                  esac
                }

                requested_excluded_services=()
                forwarded_args=()

                while [ "$#" -gt 0 ]; do
                  case "$1" in
                    --launcher-help)
                      render_launcher_help
                      exit 0
                      ;;
                    --exclude-services)
                      if [ "$#" -lt 2 ]; then
                        echo "ERROR: option '--exclude-services' requires a value" >&2
                        exit 2
                      fi
                      parse_excluded_services_csv "$2"
                      shift 2
                      ;;
                    --exclude-services=*)
                      parse_excluded_services_csv "''${1#--exclude-services=}"
                      shift
                      ;;
                    --)
                      shift
                      while [ "$#" -gt 0 ]; do
                        forwarded_args+=("$1")
                        shift
                      done
                      break
                      ;;
                    *)
                      while [ "$#" -gt 0 ]; do
                        forwarded_args+=("$1")
                        shift
                      done
                      break
                      ;;
                  esac
                done

                for service in "''${known_services[@]}"; do
                  if is_service_skipped "$service"; then
                    requested_excluded_services+=("$service")
                  fi
                done

                excluded_services_csv="$(build_excluded_services_csv)"
                selected_services_csv="$(launcher_selected_services_csv "''${forwarded_args[@]}")"

                if [ -n ${lib.escapeShellArg dispatcherHelpFile} ] \
                  && forwarded_args_only_help_flag "''${forwarded_args[@]}"; then
                  cat ${lib.escapeShellArg dispatcherHelpFile}
                  exit 0
                fi

                if forwarded_args_request_help "''${forwarded_args[@]}"; then
                  if [ -n ${lib.escapeShellArg viewHelpFile} ]; then
                    cat ${lib.escapeShellArg viewHelpFile}
                    exit 0
                  fi

                  if [ ${lib.escapeShellArg appName} = 'run-task' ] \
                    && print_fast_task_help "''${forwarded_args[0]:-}"; then
                    exit 0
                  fi
                fi

                flake_root="$(find_flake_root)"

        selection_cmd=(
          "${pkgs.nix}/bin/nix-build"
          "--no-out-link"
          "${frameworkRoot}/framework/launch/run-selected-app.nix"
          "--argstr"
          "system"
                  ${lib.escapeShellArg system}
                  "--argstr"
                  "projectRoot"
                  "$flake_root"
                  "--argstr"
                  "frameworkRoot"
                  ${lib.escapeShellArg frameworkRootAbs}
                  "--argstr"
                  "nixpkgsPath"
                  ${lib.escapeShellArg nixpkgsPathAbs}
                  "--argstr"
                  "frameworkSourceRevision"
                  ${lib.escapeShellArg frameworkSourceRevision}
                  "--argstr"
                  "appName"
                  ${lib.escapeShellArg appName}
                  "--argstr"
                  "excludedServicesCsv"
                  "$excluded_services_csv"
                  "--argstr"
                  "selectedServicesCsv"
                  "$selected_services_csv"
                  "--argstr"
                  "projectModuleSpecsJson"
                  ${lib.escapeShellArg projectModuleSpecsJson}
                  "--argstr"
                  "extraModuleSpecsJson"
                  ${lib.escapeShellArg extraModuleSpecsJson}
          "--argstr"
          "localOverrideSpecsJson"
          ${lib.escapeShellArg localOverrideSpecsJson}
        )

        selected_launcher="$("''${selection_cmd[@]}")"
        shopt -s nullglob
        selected_programs=("$selected_launcher"/bin/*)
        shopt -u nullglob

        if [ "''${#selected_programs[@]}" -ne 1 ]; then
          echo "ERROR: expected exactly one selected launcher binary for app ${appName}" >&2
          exit 3
        fi

        exec "''${selected_programs[0]}" "''${forwarded_args[@]}"
      '';
    };

  mkRuntimeAppLauncher =
    appName:
    mkShellApp {
      inherit appName;
      binPrefix = "nixfied-runtime-launch";
      body = ''
        find_flake_root() {
          local dir="''${NIXFIED_FLAKE_ROOT:-''${NIXFIED_CALLER_PWD:-$PWD}}"
          while [ "$dir" != "/" ]; do
            if [ -f "$dir/flake.nix" ]; then
              printf '%s' "$dir"
              return 0
            fi
            dir="$(${pkgs.coreutils}/bin/dirname "$dir")"
          done

          echo "ERROR: unable to locate flake root from ''${NIXFIED_FLAKE_ROOT:-''${NIXFIED_CALLER_PWD:-$PWD}}" >&2
          exit 3
        }

        flake_root="$(find_flake_root)"

        runtime_cmd=(
          "${pkgs.nix}/bin/nix-build"
          "--no-out-link"
          "${frameworkRoot}/framework/launch/run-runtime-app.nix"
          "--argstr"
          "system"
          ${lib.escapeShellArg system}
          "--argstr"
          "projectRoot"
          "$flake_root"
          "--argstr"
          "frameworkRoot"
          ${lib.escapeShellArg frameworkRootAbs}
          "--argstr"
          "nixpkgsPath"
          ${lib.escapeShellArg nixpkgsPathAbs}
          "--argstr"
          "frameworkSourceRevision"
          ${lib.escapeShellArg frameworkSourceRevision}
          "--argstr"
          "appName"
          ${lib.escapeShellArg appName}
          "--argstr"
          "projectModuleSpecsJson"
          ${lib.escapeShellArg projectModuleSpecsJson}
          "--argstr"
          "extraModuleSpecsJson"
          ${lib.escapeShellArg extraModuleSpecsJson}
          "--argstr"
          "localOverrideSpecsJson"
          ${lib.escapeShellArg localOverrideSpecsJson}
        )

        selected_launcher="$("''${runtime_cmd[@]}")"
        shopt -s nullglob
        selected_programs=("$selected_launcher"/bin/*)
        shopt -u nullglob

        if [ "''${#selected_programs[@]}" -ne 1 ]; then
          echo "ERROR: expected exactly one selected launcher binary for app ${appName}" >&2
          exit 3
        fi

        exec "''${selected_programs[0]}" "$@"
      '';
    };

  viewSelectorLauncherApps =
    if !launchersSupported then
      { }
    else
      builtins.listToAttrs (
        map (appName: {
          name = appName;
          value = mkSelectorAwareLauncher appName;
        }) viewWrappedAppNames
      );
  serviceSelectorLauncherApps =
    if !launchersSupported then
      { }
    else
      builtins.listToAttrs (
        map (appName: {
          name = appName;
          value = mkSelectorAwareLauncher appName;
        }) serviceWrappedAppNames
      );
  runtimeLauncherApps =
    if !launchersSupported then
      { }
    else
      builtins.listToAttrs (
        map (appName: {
          name = appName;
          value = mkRuntimeAppLauncher appName;
        }) runtimeAppNames
      );
  frameworkUtilityApps = {
    "framework::install" = mkShellApp {
      appName = "framework::install";
      binPrefix = "nixfied-framework";
      body = ''
        if [ "$#" -gt 0 ]; then
          case "$1" in
            --help|-h)
              cat ${lib.escapeShellArg (builtins.toString taskHelpFiles."task.framework.install")}
              exit 0
              ;;
          esac
        fi

        ${frameworkUtilityCommand { }}
      '';
    };

    "framework::upgrade" = mkShellApp {
      appName = "framework::upgrade";
      binPrefix = "nixfied-framework";
      body = ''
        if [ "$#" -gt 0 ]; then
          case "$1" in
            --help|-h)
              cat ${lib.escapeShellArg (builtins.toString taskHelpFiles."task.framework.upgrade")}
              exit 0
              ;;
          esac
        fi

        ${frameworkUtilityCommand {
          upgradeDefault = true;
        }}
      '';
    };
  };
  orchestratorControl = import ../runtime/orchestrator-control.nix {
    inherit
      pkgs
      registry
      ;
    model = compiledCore.model;
  };
  runtimeControlProgram = "${orchestratorControl}/bin/nixfied-orchestrator-control";
  runtimeControlApps = builtins.listToAttrs (
    map (appName: {
      name = appName;
      value = mkShellApp {
        inherit appName;
        binPrefix = "nixfied-control";
        body = ''
          if [ "$#" -eq 1 ]; then
            case "$1" in
              --help|-h)
                cat ${lib.escapeShellArg (builtins.toString runtimeControlHelpFiles.${appName})}
                exit 0
                ;;
            esac
          fi

          exec ${lib.escapeShellArg runtimeControlProgram} ${lib.escapeShellArg appName} "$@"
        '';
      };
    }) runtimeControlAppNames
  );
  heavyOutputs =
    let
      materializedExecution = import ./materializeExecution.nix {
        inherit
          pkgs
          projectRoot
          ;
        compiledCore = compiledCore;
        frameworkSourceFlakeRef = null;
      };
    in
    {
      runtimeHash = materializedExecution.runtimeHash or compiledCore.model.identity.evalHash;
      services = materializedExecution.services;
      serviceApis = materializedExecution.serviceApis;
      serviceHookEnv = materializedExecution.serviceHookEnv;
      directApps =
        materializedExecution.baseApps
        // coreSurfaces.apps
        // {
          default =
            if builtins.hasAttr "help" coreSurfaces.apps then
              coreSurfaces.apps.help
            else if builtins.hasAttr "default" materializedExecution.baseApps then
              materializedExecution.baseApps.default
            else
              materializedExecution.baseApps.help;
        };
      frameworkWorkspaceApps =
        if
          workspaceMarkerPresent && builtins.hasAttr "framework::test" (compiledCore.model.views.apps or { })
        then
          {
            "framework::test" = mkShellApp {
              appName = "framework::test";
              binPrefix = "nixfied-framework";
              body = ''
                if [ "$#" -gt 0 ]; then
                  case "$1" in
                    --help|-h)
                      cat ${lib.escapeShellArg (builtins.toString taskHelpFiles."task.framework.test")}
                      exit 0
                      ;;
                  esac
                fi

                cd ${lib.escapeShellArg projectRootAbs}
                exec ${lib.escapeShellArg materializedExecution.baseApps."framework::test".program} "$@"
              '';
            };
          }
        else
          { };
    };
  internalBasePackages =
    if !launchersSupported then
      { }
    else
      builtins.listToAttrs (
        map (appName: {
          name = internalBaseTargetName appName;
          value = pkgs.writeShellScriptBin (internalBaseTargetName appName) ''
            set -euo pipefail
              exec ${
                if builtins.elem appName viewWrappedAppNames then
                  viewSelectorLauncherApps.${appName}.program
                else
                  serviceSelectorLauncherApps.${appName}.program
              } "$@"
          '';
        }) (viewWrappedAppNames ++ serviceWrappedAppNames)
      );
in
{
  model = compiledCore.model;
  stateHash = compiledCore.stateHash;
  runtimeHash = heavyOutputs.runtimeHash;
  tasks = compiledCore.model.tasks;
  services = heavyOutputs.services;
  serviceCatalog = compiledCore.model.serviceCatalog;
  workflows = compiledCore.model.workflows;
  features = compiledCore.model.features;
  selectionIndex = compiledCore.selectionIndex;
  serviceSurfaceCatalog = compiledCore.serviceSurfaceCatalog;
  serviceApis = heavyOutputs.serviceApis;
  serviceHookEnv = heavyOutputs.serviceHookEnv;
  packages = coreSurfaces.packages // {
    default = pkgs.runCommand "nixfied-default" { } ''
      mkdir -p "$out/bin"
      ln -s ${
        if launchersSupported then
          coreSurfaces.apps.help.program
        else
          heavyOutputs.directApps.default.program
      } "$out/bin/default"
    '';
  };
  checks = coreSurfaces.checks;
  devShells = coreSurfaces.devShells;
  schema = coreSurfaces.schema;
  apps =
    if launchersSupported then
      serviceSelectorLauncherApps
      // coreSurfaces.apps
      // viewSelectorLauncherApps
      // runtimeLauncherApps
      // runtimeControlApps
      // frameworkUtilityApps
      // heavyOutputs.frameworkWorkspaceApps
      // {
        default = coreSurfaces.apps.help;
      }
    else
      heavyOutputs.directApps
      // runtimeControlApps
      // frameworkUtilityApps
      // heavyOutputs.frameworkWorkspaceApps;
  legacyPackages = {
    _nixfied = {
      baseApps = internalBasePackages;
    };
  };
}
