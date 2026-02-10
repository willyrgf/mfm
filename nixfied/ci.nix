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
  modeNames = builtins.attrNames modes;
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
  artifactsDir = artifacts.dir or "/tmp/ci-artifacts";
  keepOnFailure = artifacts.keepOnFailure or true;
  keepOnSuccess = artifacts.keepOnSuccess or false;

  stepDescCase = pkgs.lib.concatMapStringsSep "\n" (
    name:
    let
      desc = steps.${name}.description or name;
    in
    "  ${name}) echo \"${desc}\" ;;"
  ) (builtins.attrNames steps);

  normalizeName =
    name:
    let
      replaced = pkgs.lib.replaceStrings [ "-" "." " " "/" ] [ "_" "_" "_" "_" ] name;
    in
    pkgs.lib.strings.toLower replaced;

  stepFuncCase = pkgs.lib.concatMapStringsSep "\n" (
    name:
    let
      slug = normalizeName name;
    in
    "  ${name}) echo \"run_step_${slug}\" ;;"
  ) (builtins.attrNames steps);

  modeCase = pkgs.lib.concatMapStringsSep "\n" (
    mode:
    let
      modeSteps = modes.${mode}.steps or [ ];
      stepsList = pkgs.lib.concatMapStringsSep " " (s: "\"${s}\"") modeSteps;
    in
    "  ${mode}) STEPS=(${stepsList}) ;;"
  ) modeNames;

  mkStepFunction =
    name: step:
    let
      desc = step.description or name;
      run = step.run or "";
      when = step.when or "";
      cleanup = step.cleanup or "";
      env = step.env or { };
      slug = normalizeName name;
      envExports = pkgs.lib.concatMapStringsSep "\n" (key: "export ${key}=${toString env.${key}}") (
        builtins.attrNames env
      );
      skipVars = step.skipIfMissing or [ ];
      skipList = pkgs.lib.concatMapStringsSep " " (v: "\"${v}\"") skipVars;
      requires = step.requires or [ ];
      missingModules = builtins.filter (
        req:
        let
          modCfg = project.modules.${req} or null;
          enabledMod = if modCfg == null then false else (modCfg.enable or false);
        in
        !enabledMod
      ) requires;
      missingReason =
        if missingModules == [ ] then
          ""
        else
          "requires module(s): ${pkgs.lib.concatStringsSep ", " missingModules}";
    in
    ''
            run_step_${slug}() {
              local step_name="${name}"
              local step_desc="${desc}"

      ${pkgs.lib.optionalString (missingReason != "") ''
        echo "↷ Skipping ''${step_desc}: ${missingReason}"
        return 42
      ''}

      ${pkgs.lib.optionalString (skipVars != [ ]) ''
        local missing_reason=""
        for var in ${skipList}; do
          if [ -z "''${!var:-}" ]; then
            missing_reason="missing $var"
            break
          fi
        done
        if [ -n "$missing_reason" ]; then
          echo "↷ Skipping ''${step_desc}: $missing_reason"
          return 42
        fi
      ''}

      ${pkgs.lib.optionalString (when != "") ''
        if ! ( ${when} ); then
          echo "↷ Skipping ''${step_desc}: condition not met"
          return 42
        fi
      ''}

              local rc=0
              set +e
              (
                set -euo pipefail
      ${pkgs.lib.optionalString (envExports != "") envExports}
      ${run}
              )
              rc=$?
              set -e
      ${pkgs.lib.optionalString (cleanup != "") cleanup}
              if [ $rc -ne 0 ]; then
                return $rc
              fi
              return 0
            }
    '';

  stepFunctions = pkgs.lib.concatMapStringsSep "\n" (name: mkStepFunction name steps.${name}) (
    builtins.attrNames steps
  );

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
                      CI_MODE="''${2:-}"
                      shift 2
                      ;;
                    --)
                      shift
                      CI_STEP_ARGS=("$@")
                      break
                      ;;
                    --*)
                      MODE_FLAG="''${1#--}"
                      case "$MODE_FLAG" in
        ${pkgs.lib.concatMapStringsSep "\n" (m: "                ${m}) CI_MODE=\"${m}\" ;;") modeNames}
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
                else
                  export CI_ARTIFACTS_DIR="${artifactsDir}"
                fi
                export CI_KEEP_ARTIFACTS_ON_FAILURE="${if keepOnFailure then "1" else "0"}"
                export CI_KEEP_ARTIFACTS_ON_SUCCESS="${if keepOnSuccess then "1" else "0"}"

                # Background mode: delegate to run registry and exit
                if [ "$CI_BACKGROUND" = "true" ]; then
                  REEXEC_ARGS="--mode $CI_MODE --summary"
                  ${runRegistryScript} \
                    --name "ci-$CI_MODE" \
                    --bg \
                    --script "$0 $REEXEC_ARGS"
                  exit $?
                fi

        ${stepFunctions}

                step_desc() {
                  case "''$1" in
        ${stepDescCase}
                    *) echo "''$1" ;;
                  esac
                }

                step_func() {
                  case "''$1" in
        ${stepFuncCase}
                    *) echo "" ;;
                  esac
                }

                # Step result tracking for summary.json
                declare -a _CI_STEP_RESULTS=()

                _ci_record_step() {
                  local name="$1" status="$2" duration="$3"
                  _CI_STEP_RESULTS+=("$name|$status|$duration")
                }

                _write_summary_json() {
                  local ec="$1"
                  local json_file="$CI_ARTIFACTS_DIR/summary.json"
                  mkdir -p "$CI_ARTIFACTS_DIR"
                  {
                    echo "{"
                    echo "  \"mode\": \"$CI_MODE\","
                    echo "  \"exit_code\": $ec,"
                    echo "  \"steps\": ["
                    local first=true
                    for entry in "''${_CI_STEP_RESULTS[@]}"; do
                      IFS='|' read -r s_name s_status s_dur <<< "$entry"
                      if [ "$first" = true ]; then first=false; else echo ","; fi
                      printf "    {\"name\": \"%s\", \"status\": \"%s\", \"duration\": %s}" "$s_name" "$s_status" "$s_dur"
                    done
                    echo ""
                    echo "  ]"
                    echo "}"
                  } > "$json_file"
                }

                run_pipeline() {
                  set -euo pipefail
        ${pkgs.lib.optionalString (ciEnvExports != "") ciEnvExports}
        ${setupScript}

                  mkdir -p "$CI_ARTIFACTS_DIR"

                  local exit_code=0
                  local step_rc=0
                  STEPS=()

                  case "$CI_MODE" in
        ${modeCase}
                    *)
                      echo "Unknown CI mode: $CI_MODE" >&2
                      exit_code=1
                      ;;
                  esac

                  if [ "$exit_code" -eq 0 ] && [ "''${#STEPS[@]}" -eq 0 ]; then
                    echo "No steps configured for mode: $CI_MODE"
                    exit_code=1
                  fi

                  if [ "$exit_code" -eq 0 ]; then
                    TOTAL_STEPS="''${#STEPS[@]}"
                    STEP_INDEX=1
                    for step in "''${STEPS[@]}"; do
                      STEP_FUNC=$(step_func "$step")
                      if [ -z "$STEP_FUNC" ]; then
                        echo "Unknown step: $step" >&2
                        exit_code=1
                        break
                      fi
                      STEP_DESC=$(step_desc "$step")
                      echo ""
                      echo "Step ''${STEP_INDEX}/''${TOTAL_STEPS}: ''${STEP_DESC}"
                      local STEP_START_TIME=$(date +%s)
                      set +e
                      ( "$STEP_FUNC" )
                      step_rc=$?
                      set -e
                      local STEP_END_TIME=$(date +%s)
                      local STEP_DUR=$((STEP_END_TIME - STEP_START_TIME))
                      if [ "$step_rc" -eq 42 ]; then
                        _ci_record_step "$step" "skipped" "$STEP_DUR"
                      elif [ "$step_rc" -eq 0 ]; then
                        _ci_record_step "$step" "passed" "$STEP_DUR"
                      else
                        _ci_record_step "$step" "failed" "$STEP_DUR"
                        exit_code="$step_rc"
                        break
                      fi
                      STEP_INDEX=$((STEP_INDEX + 1))
                    done
                  fi

        ${teardownScript}

                  # Write structured summary
                  _write_summary_json "$exit_code"

                  return "$exit_code"
                }

                if [ "$CI_SUMMARY" = "true" ]; then
                  LOGFILE=$(mktemp)
                  START_TIME=$(date +%s)
                  set +e
                  ( run_pipeline ) 2>&1 | tee "$LOGFILE"
                  EXIT_CODE=$?
                  set -e
                  END_TIME=$(date +%s)
                  DURATION=$((END_TIME - START_TIME))
                  summary_parse "$LOGFILE" "$DURATION" "$EXIT_CODE"
                  rm -f "$LOGFILE" 2>/dev/null || true
                  if [ "$EXIT_CODE" -ne 0 ]; then
                    if [ "$CI_KEEP_ARTIFACTS_ON_FAILURE" = "1" ]; then
                      echo "🧾 CI artifacts kept at: $CI_ARTIFACTS_DIR"
                    else
                      rm -rf "$CI_ARTIFACTS_DIR" 2>/dev/null || true
                    fi
                  else
                    if [ "$CI_KEEP_ARTIFACTS_ON_SUCCESS" = "1" ]; then
                      echo "🧾 CI artifacts kept at: $CI_ARTIFACTS_DIR"
                    else
                      rm -rf "$CI_ARTIFACTS_DIR" 2>/dev/null || true
                    fi
                  fi
                  exit $EXIT_CODE
                else
                  set +e
                  run_pipeline
                  EXIT_CODE=$?
                  set -e
                  if [ "$EXIT_CODE" -ne 0 ]; then
                    if [ "$CI_KEEP_ARTIFACTS_ON_FAILURE" = "1" ]; then
                      echo "🧾 CI artifacts kept at: $CI_ARTIFACTS_DIR"
                    else
                      rm -rf "$CI_ARTIFACTS_DIR" 2>/dev/null || true
                    fi
                  else
                    if [ "$CI_KEEP_ARTIFACTS_ON_SUCCESS" = "1" ]; then
                      echo "🧾 CI artifacts kept at: $CI_ARTIFACTS_DIR"
                    else
                      rm -rf "$CI_ARTIFACTS_DIR" 2>/dev/null || true
                    fi
                  fi
                  exit $EXIT_CODE
                fi
      '';

  scriptDrv =
    if enabled then
      if useEphemeral then
        ephemeral.mkEphemeralWrapper {
          name = "ci";
          installDeps = ci.useDeps or true;
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
          script = script;
        }
    else
      null;

  ciApi = {
    version = 1;
    summary = "Run the CI pipeline";
    details = "Runs the CI pipeline defined in nixfied/project/ci.nix (modes + steps).";
    usage = [
      "nix run .#ci"
      "nix run .#ci -- --summary"
    ];
    examples = [ "nix run .#ci -- --summary" ];
    category = "core";
  };
  _ = lib.appApi.validateApi {
    name = "ci";
    api = ciApi;
  };

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
