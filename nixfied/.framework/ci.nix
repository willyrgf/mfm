# CI pipeline generator (steps DSL)
{
  pkgs,
  project,
  lib,
  ephemeral ? null,
}:

let
  ci = project.ci or null;
  enabled = ci != null && (ci.enable or false);
  useEphemeral = ephemeral != null && (ci.useEphemeral or true);

  steps = if enabled then (ci.steps or { }) else { };
  modes = if enabled then (ci.modes or { }) else { };
  modeNames = pkgs.lib.sort (a: b: a < b) (builtins.attrNames modes);
  modeTokenName = mode: pkgs.lib.replaceStrings [ "-" "." ":" " " "/" ] [ "_" "_" "_" "_" "_" ] mode;
  modeArgDocs = map (mode: {
    name = "--${mode}";
    description = "Select CI mode ${mode}.";
  }) modeNames;
  modeFlagSpecs = map (
    mode:
    lib.appApi.arg.flag {
      name = "mode_${modeTokenName mode}";
      long = "--${mode}";
    }
  ) modeNames;
  modeOptionSpec =
    if modeNames == [ ] then
      lib.appApi.arg.option {
        name = "mode";
        long = "--mode";
        type = "string";
      }
    else
      lib.appApi.arg.option {
        name = "mode";
        long = "--mode";
        type = "enum";
        values = modeNames;
      };
  defaultMode =
    if enabled then
      (ci.defaultMode or (if modeNames != [ ] then builtins.head modeNames else ""))
    else
      "";
  resolvedDefaultMode =
    if defaultMode != "" && builtins.hasAttr defaultMode modes then
      defaultMode
    else if modeNames != [ ] then
      builtins.head modeNames
    else
      "";

  ciEnv = ci.env or { };
  ciEnvExports = pkgs.lib.concatMapStringsSep "\n" (key: "export ${key}=${toString ciEnv.${key}}") (
    builtins.attrNames ciEnv
  );

  artifacts = ci.artifacts or { };
  artifactsRoot = artifacts.dir or "/tmp/ci-artifacts";
  keepOnFailure = artifacts.keepOnFailure or true;
  keepOnSuccess = artifacts.keepOnSuccess or false;

  normalizeName =
    name:
    let
      replaced = pkgs.lib.replaceStrings [ "-" "." " " "/" ] [ "_" "_" "_" "_" ] name;
    in
    pkgs.lib.strings.toLower replaced;

  stepNames = pkgs.lib.sort (a: b: a < b) (builtins.attrNames steps);

  mkStepTemplate =
    name:
    let
      step = steps.${name};
      _ =
        if step ? requires then
          throw "ci.steps.${name}.requires has been removed. Use ci.steps.${name}.fixtures.services instead."
        else
          null;
      fixtures = step.fixtures or null;
      fixturePrelude = lib.fixtures.renderPrelude {
        inherit fixtures;
        contextName = "ci-step-${name}";
        defaultProfile = "test";
        defaultLogs = true;
      };
      run = step.run or "";
    in
    {
      inherit name;
      description = step.description or name;
      env = step.env or { };
      when = step.when or "";
      cleanup = step.cleanup or "";
      skip_if_missing = step.skipIfMissing or [ ];
      depends_on = step.dependsOn or [ ];
      missing = false;
      run = pkgs.lib.optionalString (fixturePrelude != "") (fixturePrelude + "\n") + run;
    };

  stepCatalog = builtins.listToAttrs (
    map (name: {
      inherit name;
      value = mkStepTemplate name;
    }) stepNames
  );

  mkModePlan =
    mode:
    let
      modeSteps = modes.${mode}.steps or [ ];
      mkUnit =
        index: stepName:
        let
          baseUnit =
            if builtins.hasAttr stepName stepCatalog then
              stepCatalog.${stepName}
            else
              {
                name = stepName;
                description = stepName;
                env = { };
                when = "";
                cleanup = "";
                skip_if_missing = [ ];
                depends_on = [ ];
                missing = true;
                run = "";
              };
          sequentialDep = if index == 0 then [ ] else [ (builtins.elemAt modeSteps (index - 1)) ];
        in
        baseUnit
        // {
          id = "unit.${normalizeName mode}.${toString (index + 1)}.${normalizeName stepName}";
          depends_on = pkgs.lib.unique (sequentialDep ++ (baseUnit.depends_on or [ ]));
        };
      units = pkgs.lib.imap0 mkUnit modeSteps;
    in
    {
      schema_version = 2;
      mode = mode;
      units = units;
    };

  modePlans = builtins.listToAttrs (
    map (mode: {
      name = mode;
      value = mkModePlan mode;
    }) modeNames
  );
  modePlansJson = builtins.toJSON modePlans;

  setupScript = ci.setup or "";
  teardownScript = ci.teardown or "";

  # Run registry script path
  runRegistryScript = toString lib.runRegistryStart;

  script =
    if !enabled then
      ''
        echo "CI DSL is disabled. Enable it in nixfied/project/ci.nix (ci.enable = true)."
        exit 1
      ''
    else if modeNames == [ ] then
      ''
        echo "No CI modes configured (ci.modes is empty)."
        exit 1
      ''
    else
      ''
                        # Parse args
                        CI_MODE="${resolvedDefaultMode}"
                        CI_SUMMARY=false
                        CI_BACKGROUND=false
                        CI_STEP_ARGS=()

                        while [ "''$#" -gt 0 ]; do
                          case "''$1" in
                            --summary)
                              CI_SUMMARY=true
                              shift
                              ;;
                            --bg)
                              CI_BACKGROUND=true
                              shift
                              ;;
                            --mode)
                              if [ -z "''${2:-}" ]; then
                                echo "Missing value for --mode" >&2
                                exit 1
                              fi
                              CI_MODE="''${2:-}"
                              shift 2
                              ;;
                            --mode=*)
                              CI_MODE="''${1#--mode=}"
                              if [ -z "$CI_MODE" ]; then
                                echo "Missing value for --mode" >&2
                                exit 1
                              fi
                              shift
                              ;;
                            --)
                              shift
                              CI_STEP_ARGS=("$@")
                              break
                              ;;
                            --*)
                              MODE_FLAG="''${1#--}"
                              case "$MODE_FLAG" in
                ${pkgs.lib.concatMapStringsSep "\n" (
                  m: "                ${m}) CI_MODE=\"${m}\" ;;"
                ) modeNames}
                                *)
                                  echo "Unknown option: ''$1" >&2
                                  exit 1
                                  ;;
                              esac
                              shift
                              ;;
                            *)
                              echo "Unknown option: ''$1" >&2
                              exit 1
                              ;;
                          esac
                        done

                        export CI_MODE
                        export CI_SUMMARY
                        export CI_STEP_ARGS
                        if [ -n "''${CI_ARTIFACTS_DIR:-}" ]; then
                          export CI_ARTIFACTS_DIR="''${CI_ARTIFACTS_DIR}"
                          case "$CI_ARTIFACTS_DIR" in
                            /*) ;;
                            *) CI_ARTIFACTS_DIR="$(pwd)/$CI_ARTIFACTS_DIR" ;;
                          esac
                          export CI_ARTIFACTS_BASE="$(dirname "$CI_ARTIFACTS_DIR")"
                          export CI_ARTIFACTS_LATEST_LINK=""
                        else
                          export CI_ARTIFACTS_BASE="${artifactsRoot}"
                          case "$CI_ARTIFACTS_BASE" in
                            /*) ;;
                            *) CI_ARTIFACTS_BASE="$(pwd)/$CI_ARTIFACTS_BASE" ;;
                          esac
                          CI_RUN_ID="$(${toString lib.resolveId})"
                          export CI_ARTIFACTS_DIR="$CI_ARTIFACTS_BASE/$CI_RUN_ID"
                          export CI_ARTIFACTS_LATEST_LINK="$CI_ARTIFACTS_BASE/latest"
                        fi
                        case "$CI_ARTIFACTS_BASE" in
                          /*) ;;
                          *)
                            echo "ERROR: CI_ARTIFACTS_BASE must resolve to an absolute path (got '$CI_ARTIFACTS_BASE')" >&2
                            exit 1
                            ;;
                        esac
                        export CI_KEEP_ARTIFACTS_ON_FAILURE="${if keepOnFailure then "1" else "0"}"
                        export CI_KEEP_ARTIFACTS_ON_SUCCESS="${if keepOnSuccess then "1" else "0"}"
                ${pkgs.lib.optionalString (ciEnvExports != "") ciEnvExports}

                        export RUN_ID="$(${toString lib.resolveId} "''${RUN_ID:-}")"

                        _emit_process_event() {
                          ${toString lib.emitEvent} "$@" >/dev/null 2>&1 || true
                        }

                        init_ci_artifacts() {
                          mkdir -p "$CI_ARTIFACTS_DIR"
                          if [ -n "$CI_ARTIFACTS_LATEST_LINK" ]; then
                            mkdir -p "$CI_ARTIFACTS_BASE"
                            ln -sfn "$CI_ARTIFACTS_DIR" "$CI_ARTIFACTS_LATEST_LINK" 2>/dev/null || true
                          fi
                        }

                        cleanup_ci_artifacts() {
                          local ec="$1"
                          local keep=0

                          if [ "$ec" -ne 0 ] && [ "$CI_KEEP_ARTIFACTS_ON_FAILURE" = "1" ]; then
                            keep=1
                          fi
                          if [ "$ec" -eq 0 ] && [ "$CI_KEEP_ARTIFACTS_ON_SUCCESS" = "1" ]; then
                            keep=1
                          fi

                          if [ "$keep" -eq 1 ]; then
                            echo "INFO: CI artifacts kept at: $CI_ARTIFACTS_DIR"
                            return 0
                          fi

                          rm -rf "$CI_ARTIFACTS_DIR" 2>/dev/null || true
                          if [ -n "$CI_ARTIFACTS_LATEST_LINK" ] && [ -L "$CI_ARTIFACTS_LATEST_LINK" ]; then
                            local link_target=""
                            link_target=$(readlink "$CI_ARTIFACTS_LATEST_LINK" 2>/dev/null || true)
                            if [ "$link_target" = "$CI_ARTIFACTS_DIR" ]; then
                              rm -f "$CI_ARTIFACTS_LATEST_LINK" 2>/dev/null || true
                            fi
                          fi
                        }

                        # Background mode: delegate to run registry and exit
                        if [ "$CI_BACKGROUND" = "true" ]; then
                          REEXEC_ARGS="--mode $CI_MODE --summary"
                          ${runRegistryScript} \
                            --name "ci-$CI_MODE" \
                            --bg \
                            --script "$0 $REEXEC_ARGS"
                          exit $?
                        fi

                        MODE_PLANS_JSON=$(cat <<'NIXFIED_MODE_PLANS_JSON'
                ${modePlansJson}
        NIXFIED_MODE_PLANS_JSON
                        )

                        _write_summary_json() {
                          local ec="$1"
                          local total_duration="$2"
                          local setup_duration="$3"
                          local steps_duration="$4"
                          local teardown_duration="$5"
                          local plan_result_file="$6"
                          local accounted_duration=0
                          local untracked_duration=0
                          local json_file="$CI_ARTIFACTS_DIR/summary.json"
                          local steps_json="[]"
                          local plan_id=""
                          local run_id=""

                          accounted_duration=$((setup_duration + steps_duration + teardown_duration))
                          untracked_duration=$((total_duration - accounted_duration))
                          if [ "$untracked_duration" -lt 0 ]; then
                            untracked_duration=0
                          fi

                          if [ -f "$plan_result_file" ]; then
                            steps_json="$(${pkgs.jq}/bin/jq -c '.steps // []' "$plan_result_file" 2>/dev/null || echo "[]")"
                            plan_id="$(${pkgs.jq}/bin/jq -r '.plan_id // ""' "$plan_result_file" 2>/dev/null || true)"
                            run_id="$(${pkgs.jq}/bin/jq -r '.run_id // ""' "$plan_result_file" 2>/dev/null || true)"
                          fi

                          mkdir -p "$CI_ARTIFACTS_DIR"
                          ${pkgs.jq}/bin/jq -n \
                            --arg mode "$CI_MODE" \
                            --arg exit_code "$ec" \
                            --argjson steps "$steps_json" \
                            --arg plan_id "$plan_id" \
                            --arg run_id "$run_id" \
                            --arg total_duration "$total_duration" \
                            --arg setup_duration "$setup_duration" \
                            --arg steps_duration "$steps_duration" \
                            --arg teardown_duration "$teardown_duration" \
                            --arg accounted_duration "$accounted_duration" \
                            --arg untracked_duration "$untracked_duration" \
                            '
                            {
                              mode: $mode,
                              exit_code: ($exit_code | tonumber),
                              plan_id: (if $plan_id == "" then null else $plan_id end),
                              run_id: (if $run_id == "" then null else $run_id end),
                              steps: $steps,
                              timing: {
                                total_duration: ($total_duration | tonumber),
                                setup_duration: ($setup_duration | tonumber),
                                steps_duration: ($steps_duration | tonumber),
                                teardown_duration: ($teardown_duration | tonumber),
                                accounted_duration: ($accounted_duration | tonumber),
                                untracked_duration: ($untracked_duration | tonumber)
                              }
                            }
                            ' > "$json_file"
                        }

                        write_ci_plan() {
                          local plan_file="$1"
                          local mode_plan_json=""
                          local unit_count=0
                          local plan_canonical=""
                          local plan_id=""
                          local tmp_file=""

                          mode_plan_json="$(printf '%s\n' "$MODE_PLANS_JSON" | ${pkgs.jq}/bin/jq -c --arg mode "$CI_MODE" '.[$mode] // null')"
                          if [ "$mode_plan_json" = "null" ] || [ -z "$mode_plan_json" ]; then
                            echo "Unknown CI mode: $CI_MODE" >&2
                            return 1
                          fi

                          printf '%s\n' "$mode_plan_json" > "$plan_file"
                          unit_count="$(${pkgs.jq}/bin/jq -r '(.units // []) | length' "$plan_file")"
                          if [ "$unit_count" -eq 0 ]; then
                            echo "No steps configured for mode: $CI_MODE"
                            return 1
                          fi

                          plan_canonical="$(${pkgs.jq}/bin/jq -cS 'del(.plan_id)' "$plan_file")"
                          plan_id="$(printf '%s' "$plan_canonical" | ${toString lib.mkPlanId} --from-stdin)"
                          tmp_file="$plan_file.tmp.$$"
                          ${pkgs.jq}/bin/jq --arg plan_id "$plan_id" '.plan_id = $plan_id' "$plan_file" > "$tmp_file"
                          mv "$tmp_file" "$plan_file"
                          export NIXFIED_PLAN_ID="$plan_id"
                          return 0
                        }

                        run_pipeline() {
                          local exit_code=0
                          local setup_rc=0
                          local teardown_rc=0
                          local step_rc=0
                          local plan_write_rc=0
                          local setup_start_time=0
                          local setup_end_time=0
                          local setup_duration=0
                          local teardown_start_time=0
                          local teardown_end_time=0
                          local teardown_duration=0
                          local steps_duration=0
                          local pipeline_start_time=0
                          local pipeline_end_time=0
                          local pipeline_duration=0
                          local plan_file="$CI_ARTIFACTS_DIR/execution-plan.json"
                          local plan_result_file="$CI_ARTIFACTS_DIR/execution-result.json"

                          pipeline_start_time=$(date +%s)
                          init_ci_artifacts

                          setup_start_time=$(date +%s)
                          set +e
                          (
                            set -euo pipefail
                ${setupScript}
                          )
                          setup_rc=$?
                          set -e
                          setup_end_time=$(date +%s)
                          setup_duration=$((setup_end_time - setup_start_time))

                          if [ "$setup_rc" -ne 0 ]; then
                            echo "ERROR: CI setup failed rc=$setup_rc" >&2
                            exit_code=$setup_rc
                          fi

                          if [ "$exit_code" -eq 0 ]; then
                            set +e
                            write_ci_plan "$plan_file"
                            plan_write_rc=$?
                            set -e
                            if [ "$plan_write_rc" -ne 0 ]; then
                              exit_code="$plan_write_rc"
                            fi
                          fi

                          if [ "$exit_code" -eq 0 ]; then
                            set +e
                            ${toString lib.runPlan} \
                              --plan-file "$plan_file" \
                              --result-file "$plan_result_file" \
                              --emit-event "${toString lib.emitEvent}" \
                              --context-script "${toString lib.helpersScript}"
                            step_rc=$?
                            set -e

                            if [ -f "$plan_result_file" ]; then
                              steps_duration="$(${pkgs.jq}/bin/jq -r '.steps_duration // 0' "$plan_result_file" 2>/dev/null || echo 0)"
                            fi
                            if [ "$step_rc" -ne 0 ]; then
                              exit_code="$step_rc"
                            fi
                          fi

                          teardown_start_time=$(date +%s)
                          set +e
                          (
                            set -euo pipefail
                ${teardownScript}
                          )
                          teardown_rc=$?
                          set -e
                          teardown_end_time=$(date +%s)
                          teardown_duration=$((teardown_end_time - teardown_start_time))
                          if [ "$teardown_rc" -ne 0 ]; then
                            echo "ERROR: CI teardown failed rc=$teardown_rc" >&2
                            if [ "$exit_code" -eq 0 ]; then
                              exit_code=$teardown_rc
                            fi
                          fi

                          pipeline_end_time=$(date +%s)
                          pipeline_duration=$((pipeline_end_time - pipeline_start_time))

                          # Write structured summary
                          _write_summary_json "$exit_code" "$pipeline_duration" "$setup_duration" "$steps_duration" "$teardown_duration" "$plan_result_file"

                          if [ "$exit_code" -eq 0 ]; then
                            return 0
                          fi
                          return "$exit_code"
                        }

                        if [ "$CI_SUMMARY" = "true" ]; then
                          init_ci_artifacts
                          _emit_process_event --event-type run_started --state running --wait-reason "ci_mode=$CI_MODE summary=true"
                          LOGFILE=$(artifact_path "ci-output.log")
                          START_TIME=$(date +%s)
                          set +e
                          ( run_pipeline ) 2>&1 | tee "$LOGFILE"
                          EXIT_CODE=$?
                          set -e
                          END_TIME=$(date +%s)
                          DURATION=$((END_TIME - START_TIME))
                          if [ "$EXIT_CODE" -eq 0 ]; then
                            _emit_process_event --event-type run_finished --state passed --wait-reason "exit_code=0 duration=$DURATION"
                          else
                            _emit_process_event --event-type run_finished --state failed --wait-reason "exit_code=$EXIT_CODE duration=$DURATION"
                          fi
                          summary_parse "$LOGFILE" "$DURATION" "$EXIT_CODE"
                          cleanup_ci_artifacts "$EXIT_CODE"
                          exit $EXIT_CODE
                        else
                          _emit_process_event --event-type run_started --state running --wait-reason "ci_mode=$CI_MODE summary=false"
                          set +e
                          run_pipeline
                          EXIT_CODE=$?
                          set -e
                          if [ "$EXIT_CODE" -eq 0 ]; then
                            _emit_process_event --event-type run_finished --state passed --wait-reason "exit_code=0"
                          else
                            _emit_process_event --event-type run_finished --state failed --wait-reason "exit_code=$EXIT_CODE"
                          fi
                          cleanup_ci_artifacts "$EXIT_CODE"
                          exit $EXIT_CODE
                        fi
      '';

  ciApi = lib.appApi.mkBatchRunnerCommandApi {
    name = "ci";
    summary = "Run the CI pipeline";
    details = "Runs the CI pipeline defined in nixfied/project/ci.nix (modes + steps).";
    usage = [
      "nix run .#ci"
      "nix run .#ci -- --summary"
    ];
    examples = [ "nix run .#ci -- --summary" ];
    category = "core";
    idempotent = false;
    args = [
      {
        name = "--summary";
        description = "Print compact CI summary output.";
      }
      {
        name = "--bg";
        description = "Run CI in background mode via the run registry.";
      }
      {
        name = "--mode";
        description = "Select configured CI mode.";
      }
    ]
    ++ modeArgDocs;
    contractArgs = [
      (lib.appApi.arg.flag {
        name = "summary";
        long = "--summary";
      })
      (lib.appApi.arg.flag {
        name = "bg";
        long = "--bg";
      })
      modeOptionSpec
    ]
    ++ modeFlagSpecs;
    env = [
      {
        name = "CI_ARTIFACTS_DIR";
        description = "Override the output artifact directory.";
      }
      {
        name = "CI_ARTIFACTS_BASE";
        description = "Override artifacts root; must be absolute path.";
      }
    ];
    contractEnv = [
      (lib.appApi.env.string { name = "CI_ARTIFACTS_DIR"; })
      (lib.appApi.env.typed {
        name = "CI_ARTIFACTS_BASE";
        type = "pathAbs";
      })
    ];
    failureCodes = lib.appApi.failureProfiles.script;
  };

  scriptDrv =
    if enabled then
      if useEphemeral then
        ephemeral.mkEphemeralWrapper {
          name = "ci";
          installDeps = ci.useDeps or true;
          appContract = ciApi.appContract;
          extraEnv = ''
            export COMMAND_NAME="ci"
            source ${toString lib.loadEnv}
            source ${toString lib.helpersScript}
            ${lib.hookExports}
            ${ciEnvExports}
          '';
          inherit script;
        }
      else
        lib.mkAppScript {
          name = "ci";
          env = ci.env or { };
          useDeps = ci.useDeps or true;
          appContract = ciApi.appContract;
          script = script;
        }
    else
      null;

  app =
    if enabled then
      {
        type = "app";
        meta = {
          description = ciApi.summary;
          nixfied = {
            api = ciApi;
          };
        };
        program = toString scriptDrv;
      }
    else
      null;
in
if enabled then
  {
    inherit
      app
      script
      scriptDrv
      ;
  }
else
  null
