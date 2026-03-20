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

  frameworkRoot = ../../.;

  compiled = import ./mkNixfied.nix {
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

  serviceNames = builtins.sort builtins.lessThan (
    lib.unique (
      map (
        serviceId:
        let
          service = compiled.services.${serviceId};
        in
        service.name or serviceId
      ) (builtins.attrNames (compiled.services or { }))
    )
  );
  viewAppNames = builtins.sort builtins.lessThan (
    builtins.attrNames (compiled.model.views.apps or { })
  );
  serviceAppNames = builtins.filter (name: lib.hasPrefix "svc::" name) (
    builtins.attrNames (compiled.apps or { })
  );
  dispatcherAppNames = builtins.filter (name: builtins.hasAttr name compiled.apps) [
    "run-task"
    "run-workflow"
    "run-workflow-parallel"
  ];
  nonSelectorAppNames = [
    "framework::install"
    "framework::upgrade"
  ];
  wrappedAppNames = builtins.sort builtins.lessThan (
    builtins.filter (appName: !(builtins.elem appName nonSelectorAppNames)) (
      lib.unique (viewAppNames ++ serviceAppNames ++ dispatcherAppNames)
    )
  );
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

  taskIds = builtins.sort builtins.lessThan (builtins.attrNames (compiled.model.tasks or { }));

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
      task = compiled.model.tasks.${taskId};
      app = task.ui.app or { };
      argsContract = (((task.contract or { }).input or { }).args or { });
      specs = map normalizeTaskArgSpec (argsContract.spec or [ ]);
      displayName = if (app.expose or false) && (app.name or "") != "" then app.name else taskId;
      usageLines =
        let
          configuredUsage = app.usage or [ ];
        in
        if configuredUsage != [ ] then configuredUsage else [ "nix run .#run-task -- ${taskId} [-- ...]" ];
      exampleLines = app.examples or [ ];
      optionLines = map formatTaskArgHelpLine specs ++ [
        "  -h, --help: Show this help."
      ];
      summary = task.summary or "";
      description = task.description or "";
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

  renderTaskHelpCases = builtins.concatStringsSep "\n" (
    map (taskId: ''
      ${lib.escapeShellArg taskId})
        cat ${lib.escapeShellArg (builtins.toString taskHelpFiles.${taskId})}
        return 0
        ;;
    '') taskIds
  );

  workflowIds = builtins.sort builtins.lessThan (
    builtins.attrNames (compiled.model.workflows or { })
  );

  workflowFamilyFromId =
    workflowId:
    let
      match = builtins.match "^workflow\\.([^.]+)\\..+$" workflowId;
    in
    if match == null then null else builtins.elemAt match 0;

  workflowModesByFamily = builtins.foldl' (
    acc: workflowId:
    let
      family = workflowFamilyFromId workflowId;
      modeMatch = builtins.match "^workflow\\.[^.]+\\.(.+)$" workflowId;
      mode = if modeMatch == null then null else builtins.elemAt modeMatch 0;
      existing = acc.${family} or [ ];
    in
    if family == null || mode == null then
      acc
    else
      acc
      // {
        ${family} = builtins.sort builtins.lessThan (lib.unique (existing ++ [ mode ]));
      }
  ) { } workflowIds;

  workflowFamilies = builtins.sort builtins.lessThan (builtins.attrNames workflowModesByFamily);

  taskBaseClosureCsvById = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = compiled.serviceSelection.servicesToCsv (
        compiled.serviceSelection.taskBaseClosureServicesById.${taskId} or [ ]
      );
    }) taskIds
  );

  taskRunnerWorkflowIdById = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = compiled.model.tasks.${taskId}.runner.workflowId or "";
    }) taskIds
  );

  workflowClosureCsvById = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = compiled.serviceSelection.servicesToCsv (
        compiled.serviceSelection.workflowClosureServicesById.${workflowId} or [ ]
      );
    }) workflowIds
  );

  mkSelectorAwareLauncher =
    appName:
    let
      launcherTaskId =
        if builtins.hasAttr appName (compiled.model.views.apps or { }) then
          compiled.model.views.apps.${appName}.taskId
        else
          "";
      serviceAppMatch = builtins.match "^svc::([^:]+)::.+$" appName;
      launcherServiceName = if serviceAppMatch == null then "" else builtins.elemAt serviceAppMatch 0;
      viewHelpFile =
        if builtins.hasAttr appName (compiled.model.views.apps or { }) then
          builtins.toString taskHelpFiles.${compiled.model.views.apps.${appName}.taskId}
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

  launcherApps =
    if !launchersSupported then
      { }
    else
      builtins.listToAttrs (
        map (appName: {
          name = appName;
          value = mkSelectorAwareLauncher appName;
        }) wrappedAppNames
      );
  internalBasePackages =
    if !launchersSupported then
      { }
    else
      builtins.listToAttrs (
        map (appName: {
          name = internalBaseTargetName appName;
          value = pkgs.writeShellScriptBin (internalBaseTargetName appName) ''
            set -euo pipefail
            exec ${compiled.apps.${appName}.program} "$@"
          '';
        }) wrappedAppNames
      );
in
compiled
// {
  apps = compiled.apps // launcherApps;
  legacyPackages = {
    _nixfied = {
      baseApps = internalBasePackages;
    };
  };
}
