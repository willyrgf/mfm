{ lib, pkgs, ... }:
let
  conf = import ./conf.nix { inherit pkgs; };
  project = conf.project;

  envNames = builtins.attrNames conf.envs;
  envOffsets = lib.mapAttrs (_: value: value.offset or 0) conf.envs;

  thinWrapperFlake = import ../install/wrapper-flake.nix {
    frameworkInput = "github:willyrgf/nixfied";
  };

  vendoredWrapperFlake = import ../install/wrapper-flake.nix {
    vendorPath = "./nixfied";
  };

  commonRuntimeInputs = [
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnused
    pkgs.gnugrep
  ];

  mkCommandTask =
    {
      id,
      appName,
      summary,
      description ? "",
      command ? "",
      kind ? "command",
      tags ? [ ],
      usage ? [ ],
      examples ? [ ],
      runtimeInputs ? commonRuntimeInputs,
      workflowId ? null,
      contractArgs ? [ ],
    }:
    {
      inherit
        id
        kind
        summary
        description
        tags
        ;

      runner =
        if workflowId == null then
          {
            type = "shell";
            command = command;
          }
        else
          {
            type = "workflowRef";
            workflowId = workflowId;
          };

      contract = {
        version = 1;
        input = {
          args = {
            parser = "typed";
            allowUnknown = true;
            spec = contractArgs;
          };
          env = {
            schemaRef = "runtimePrimitives";
            extra = [ ];
          };
        };
        output = {
          format = "text";
          channels = "stdout";
          keys = [ ];
        };
        behavior = {
          idempotent = false;
          effects = [ "writes-state" ];
          timeoutSec = 0;
        };
        errors.codes = {
          generic = 1;
          usage = 2;
          precondition = 3;
        };
      };

      runtime = {
        slotEnv = "optional";
        workdir = "projectRoot";
        hermetic = true;
        runtimeInputs = runtimeInputs;
        passThroughEnv = [
          "HOME"
          project.envVar
          project.slotVar
          "CI_ARTIFACTS_DIR"
          "CI_MAX_WORKERS"
          "NIXFIED_CI_MAX_WORKERS"
          "API_KEY"
        ];
        env = { };
        umask = "022";
        locale = "C.UTF-8";
        timezone = "UTC";
      };

      scheduling = {
        locks = [ ];
        maxAttempts = 1;
        retryBackoffSec = [ ];
        priority = 100;
      };

      deps = {
        needs = [ ];
        softNeeds = [ ];
      };

      produces = {
        artifacts = [ ];
        stateKeys = [ ];
      };

      ui.app = {
        expose = true;
        name = appName;
        category = "core";
        usage = usage;
        examples = examples;
      };
    };
in
{
  imports = [
    ../modules/profiles/webapp.nix
  ];

  config = {
    nixfied = {
      identity = {
        projectId = project.id;
        projectName = project.name;
        description = project.description;
      };

      runtime = {
        slot = {
          var = project.slotVar;
          default = conf.slots.default;
          max = conf.slots.max;
          stride = conf.slots.stride;
        };

        env = {
          var = project.envVar;
          names = envNames;
          offsets = envOffsets;
          default = "dev";
        };

        logging = {
          levelDefault = conf.logging.level;
          outputDefault = conf.logging.output;
        };

        ports = conf.ports;
        directories.base = conf.directories.base;
      };

      state = {
        registryRoot = conf.process.registryRoot;
        artifactsRoot = "/tmp/ci-artifacts";
      };

      tooling = {
        runtimePackages = conf.tooling.runtimePackages;
        devShellPackages = conf.tooling.devShellPackages;
        devShellHook = conf.tooling.devShellHook;
      };

      services = {
        postgres = {
          enable = conf.modules.postgres.enable or false;
          database = conf.modules.postgres.database or "app";
          portKey = conf.modules.postgres.portKey or "postgres";
        };

        nginx = {
          enable = conf.modules.nginx.enable or false;
          portKeyHttp = conf.modules.nginx.portKeyHttp or "http";
          portKeyHttps = conf.modules.nginx.portKeyHttps or "https";
        };

        minio = {
          enable = conf.modules.minio.enable or false;
          portKeyApi = conf.modules.minio.portKeyApi or "minioApi";
          portKeyConsole = conf.modules.minio.portKeyConsole or "minioConsole";
        };

        reth = {
          enable = conf.modules.reth.enable or false;
          portKeyHttp = conf.modules.reth.portKeyHttp or "rethHttp";
          portKeyWs = conf.modules.reth.portKeyWs or "rethWs";
          portKeyAuth = conf.modules.reth.portKeyAuth or "rethAuth";
        };

        helios = {
          enable = conf.modules.helios.enable or false;
          portKeyRpc = conf.modules.helios.portKeyRpc or "heliosRpc";
          executionRpcPortKey = conf.modules.helios.executionRpcPortKey or "rethHttp";
        };
      };

      operations = {
        enable = true;
        validateEnv.enable = true;
        testIsolation.enable = true;
        ports.enable = true;
        checkPorts.enable = true;
      };

      tasks = {
        dev = mkCommandTask {
          id = "task.dev";
          appName = "dev";
          summary = "Start the dev workflow";
          description = ''
            Runs the project's dev workflow.

            Customize this command in nixfied/project/module.nix.
          '';
          tags = [
            "dev"
            "local"
          ];
          usage = [ "NIX_ENV=0 nix run .#dev" ];
          examples = [ "NIX_ENV=0 nix run .#dev" ];
          command = ''
            set -euo pipefail
            echo "INFO: starting dev workflow"
            echo "SKIP: dev command placeholder. Edit nixfied/project/module.nix to run your app."
          '';
        };

        build = mkCommandTask {
          id = "task.build";
          appName = "build";
          summary = "Build artifacts";
          description = ''
            Runs the project's build workflow.

            Customize this command in nixfied/project/module.nix.
          '';
          usage = [ "nix run .#build" ];
          command = ''
            set -euo pipefail
            echo "INFO: running build workflow"
            echo "SKIP: build command placeholder. Edit nixfied/project/module.nix."
          '';
        };

        check = mkCommandTask {
          id = "task.check";
          appName = "check";
          summary = "Run quality checks";
          description = ''
            Runs quality checks for the repository.

            Customize this command in nixfied/project/module.nix.
          '';
          usage = [ "nix run .#check" ];
          command = ''
            set -euo pipefail
            echo "INFO: running quality checks"
            echo "SKIP: quality checks placeholder. Edit nixfied/project/module.nix."
          '';
        };

        format = mkCommandTask {
          id = "task.format";
          appName = "format";
          summary = "Format Nix files";
          usage = [ "nix run .#format" ];
          command = ''
            set -euo pipefail
            find . -name '*.nix' -print0 | xargs -0 nixfmt --
            echo "OK: formatted nix files"
          '';
        };

        test = mkCommandTask {
          id = "task.test";
          appName = "test";
          kind = "workflow";
          summary = "Run tests";
          description = "Run tests through the deterministic workflow executor.";
          usage = [ "nix run .#test" ];
          workflowId = "workflow.ci.full";
          contractArgs = [
            {
              name = "summary";
              kind = "flag";
              long = "--summary";
              description = "Print compact summary output.";
            }
            {
              name = "mode";
              kind = "option";
              long = "--mode";
              type = "enum";
              values = [
                "basic"
                "app"
                "env"
                "full"
              ];
              description = "Select workflow mode.";
            }
          ];
        };

        ci = mkCommandTask {
          id = "task.ci";
          appName = "ci";
          kind = "workflow";
          summary = "Run the CI pipeline";
          description = "Runs CI through workflow.ci.<mode> plans.";
          usage = [
            "nix run .#ci"
            "nix run .#ci -- --summary"
          ];
          workflowId = "workflow.ci.full";
          contractArgs = [
            {
              name = "summary";
              kind = "flag";
              long = "--summary";
              description = "Print compact summary output.";
            }
            {
              name = "mode";
              kind = "option";
              long = "--mode";
              type = "enum";
              values = [
                "basic"
                "app"
                "env"
                "full"
              ];
              description = "Select workflow mode.";
            }
            {
              name = "basic";
              kind = "flag";
              long = "--basic";
              description = "Alias for --mode basic.";
            }
            {
              name = "app";
              kind = "flag";
              long = "--app";
              description = "Alias for --mode app.";
            }
            {
              name = "env";
              kind = "flag";
              long = "--env";
              description = "Alias for --mode env.";
            }
            {
              name = "full";
              kind = "flag";
              long = "--full";
              description = "Alias for --mode full.";
            }
          ];
        };

        ci-quality =
          mkCommandTask {
            id = "task.ci.quality";
            appName = "ci-quality";
            kind = "ci-step";
            summary = "Quality checks";
            description = "Quality CI step.";
            tags = [
              "ci"
              "quality"
            ];
            runtimeInputs = commonRuntimeInputs;
            command = ''
              set -euo pipefail
              artifacts_dir="''${CI_ARTIFACTS_DIR:-/tmp/ci-artifacts}"
              mkdir -p "$artifacts_dir"
              touch "$artifacts_dir/quality.log"
              echo "OK: quality step complete"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-tests =
          mkCommandTask {
            id = "task.ci.tests";
            appName = "ci-tests";
            kind = "ci-step";
            summary = "Tests";
            description = "Test CI step.";
            tags = [
              "ci"
              "tests"
            ];
            runtimeInputs = commonRuntimeInputs;
            command = ''
              set -euo pipefail
              artifacts_dir="''${CI_ARTIFACTS_DIR:-/tmp/ci-artifacts}"
              mkdir -p "$artifacts_dir"
              touch "$artifacts_dir/tests.log"
              echo "OK: tests step complete"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-system-quick =
          mkCommandTask {
            id = "task.ci.system-quick";
            appName = "ci-system-quick";
            kind = "ci-step";
            summary = "Quick system tests";
            description = "Optional system test gate.";
            tags = [
              "ci"
              "system"
            ];
            runtimeInputs = commonRuntimeInputs;
            command = ''
              set -euo pipefail
              if [ -z "''${API_KEY:-}" ]; then
                echo "SKIP: API_KEY not set"
                exit 0
              fi
              artifacts_dir="''${CI_ARTIFACTS_DIR:-/tmp/ci-artifacts}"
              mkdir -p "$artifacts_dir"
              touch "$artifacts_dir/system-quick.log"
              echo "OK: quick system step complete"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-nginx-proxy =
          mkCommandTask {
            id = "task.ci.nginx-proxy";
            appName = "ci-nginx-proxy";
            kind = "ci-step";
            summary = "Nginx proxy test";
            description = "Nginx proxy CI step.";
            tags = [
              "ci"
              "proxy"
            ];
            runtimeInputs = commonRuntimeInputs;
            command = ''
              set -euo pipefail
              artifacts_dir="''${CI_ARTIFACTS_DIR:-/tmp/ci-artifacts}"
              mkdir -p "$artifacts_dir"
              touch "$artifacts_dir/nginx-proxy.log"
              echo "OK: nginx proxy step complete"
            '';
          }
          // {
            ui.app.expose = false;
          };

        framework-test = mkCommandTask {
          id = "task.framework.test";
          appName = "framework::test";
          kind = "utility";
          summary = "Run framework validation in the model";
          description = ''
            Runs deterministic validation shards.
          '';
          runtimeInputs = [
            pkgs.bash
            pkgs.coreutils
            pkgs.findutils
            pkgs.gnugrep
            pkgs.gnused
            pkgs.nix
          ];
          usage = [
            "nix run .#framework::test"
            "nix run .#framework::test -- --summary"
            "nix run .#framework::test -- --mode env --summary-json /tmp/framework-summary.json"
          ];
          examples = [
            "nix run .#framework::test -- --list-shards"
            "nix run .#framework::test -- --shard flake-check"
            "FRAMEWORK_ISOLATION=1 nix run .#framework::test -- --summary"
          ];
          contractArgs = [
            {
              name = "summary";
              kind = "flag";
              long = "--summary";
              description = "Print compact summary output.";
            }
            {
              name = "summary-json";
              kind = "option";
              long = "--summary-json";
              type = "string";
              description = "Write summary JSON to a file.";
            }
            {
              name = "profile";
              kind = "option";
              long = "--profile";
              type = "enum";
              values = [ "ci" ];
              description = "Test profile to run (ci only).";
            }
            {
              name = "shard";
              kind = "option";
              long = "--shard";
              type = "string";
              values = [
                "flake-check"
                "help"
                "workflow-test"
                "workflow-ci"
                "isolation"
              ];
              description = "Run one shard only.";
            }
            {
              name = "list-shards";
              kind = "flag";
              long = "--list-shards";
              description = "List available shards and exit.";
            }
            {
              name = "mode";
              kind = "option";
              long = "--mode";
              type = "enum";
              values = [
                "basic"
                "app"
                "env"
                "full"
              ];
              description = "CI workflow mode used by the workflow-ci shard.";
            }
            {
              name = "basic";
              kind = "flag";
              long = "--basic";
              description = "Alias for --mode basic.";
            }
            {
              name = "app";
              kind = "flag";
              long = "--app";
              description = "Alias for --mode app.";
            }
            {
              name = "env";
              kind = "flag";
              long = "--env";
              description = "Alias for --mode env.";
            }
            {
              name = "full";
              kind = "flag";
              long = "--full";
              description = "Alias for --mode full.";
            }
          ];
          command = ''
            set -euo pipefail

            ROOT="$(pwd -P)"
            PROFILE="ci"
            MODE="full"
            SHARD=""
            LIST_SHARDS=0
            SUMMARY=0
            SUMMARY_JSON=""
            SHARDS=(
              "flake-check"
              "help"
              "workflow-test"
              "workflow-ci"
              "isolation"
            )
            EXECUTED=0
            STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
            START_EPOCH="$(date +%s)"

            log_info() {
              printf 'INFO: %s\n' "$*"
            }

            log_warn() {
              printf 'WARN: %s\n' "$*"
            }

            log_error() {
              printf 'ERROR: %s\n' "$*" >&2
            }

            log_ok() {
              printf 'OK: %s\n' "$*"
            }

            log_skip() {
              printf 'SKIP: %s\n' "$*"
            }

            usage() {
              cat <<'EOF'
            Usage: nix run .#framework::test [-- --profile ci] [--mode <basic|app|env|full>] [--summary] [--summary-json <path>] [--shard <name>] [--list-shards]

            Shards:
              flake-check   Run nix flake check for the current project root.
              help          Validate generated help output.
              workflow-test Run the test workflow surface.
              workflow-ci   Run the CI workflow surface in selected mode.
              isolation     Run isolation checks when FRAMEWORK_ISOLATION=1 or explicitly selected.
            EOF
            }

            print_shards() {
              local shard_name
              for shard_name in "''${SHARDS[@]}"; do
                printf '%s\n' "$shard_name"
              done
            }

            shard_exists() {
              local candidate="$1"
              local shard_name
              for shard_name in "''${SHARDS[@]}"; do
                if [ "$candidate" = "$shard_name" ]; then
                  return 0
                fi
              done
              return 1
            }

            write_summary_json() {
              local rc="$1"
              local finished_at duration
              finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
              duration="$(( $(date +%s) - START_EPOCH ))"
              mkdir -p "$(dirname "$SUMMARY_JSON")"
              cat > "$SUMMARY_JSON" <<JSON
            {
              "profile": "$PROFILE",
              "mode": "$MODE",
              "shard": $(if [ -n "$SHARD" ]; then printf '"%s"' "$SHARD"; else printf 'null'; fi),
              "executed_shards": $EXECUTED,
              "exit_code": $rc,
              "duration_seconds": $duration,
              "started_at": "$STARTED_AT",
              "finished_at": "$finished_at"
            }
            JSON
              log_info "wrote summary json path=$SUMMARY_JSON"
            }

            run_shard() {
              local shard_name="$1"
              shift
              log_info "running shard=$shard_name"
              if "$@"; then
                EXECUTED="$((EXECUTED + 1))"
                log_ok "shard passed name=$shard_name"
                return 0
              fi
              local rc=$?
              log_error "shard failed name=$shard_name rc=$rc"
              return "$rc"
            }

            shard_flake_check() {
              nix flake check "path:$ROOT"
            }

            shard_help() {
              nix run "path:$ROOT"#help >/dev/null
            }

            shard_workflow_test() {
              nix run "path:$ROOT"#test -- --summary
            }

            shard_workflow_ci() {
              nix run "path:$ROOT"#ci -- --mode "$MODE" --summary
            }

            shard_isolation() {
              if [ "''${FRAMEWORK_ISOLATION:-}" = "1" ] || [ "$SHARD" = "isolation" ]; then
                nix run "path:$ROOT"#test-isolation
                return 0
              fi
              log_skip "isolation shard disabled (set FRAMEWORK_ISOLATION=1 to enable)"
              return 0
            }

            run_named_shard() {
              local shard_name="$1"
              case "$shard_name" in
                flake-check)
                  run_shard "$shard_name" shard_flake_check
                  ;;
                help)
                  run_shard "$shard_name" shard_help
                  ;;
                workflow-test)
                  run_shard "$shard_name" shard_workflow_test
                  ;;
                workflow-ci)
                  run_shard "$shard_name" shard_workflow_ci
                  ;;
                isolation)
                  run_shard "$shard_name" shard_isolation
                  ;;
                *)
                  log_error "unknown shard '$shard_name'"
                  return 2
                  ;;
              esac
            }

            while [ "$#" -gt 0 ]; do
              case "$1" in
                --profile)
                  if [ "$#" -lt 2 ]; then
                    log_error "--profile requires a value"
                    exit 2
                  fi
                  PROFILE="$2"
                  shift 2
                  ;;
                --mode)
                  if [ "$#" -lt 2 ]; then
                    log_error "--mode requires a value"
                    exit 2
                  fi
                  MODE="$2"
                  shift 2
                  ;;
                --basic|--app|--env|--full)
                  MODE="''${1#--}"
                  shift
                  ;;
                --summary)
                  SUMMARY=1
                  shift
                  ;;
                --summary-json)
                  if [ "$#" -lt 2 ]; then
                    log_error "--summary-json requires a value"
                    exit 2
                  fi
                  SUMMARY_JSON="$2"
                  shift 2
                  ;;
                --shard)
                  if [ "$#" -lt 2 ]; then
                    log_error "--shard requires a value"
                    exit 2
                  fi
                  SHARD="$2"
                  shift 2
                  ;;
                --list-shards)
                  LIST_SHARDS=1
                  shift
                  ;;
                --help|-h)
                  usage
                  exit 0
                  ;;
                --)
                  shift
                  break
                  ;;
                *)
                  log_error "unknown option '$1'"
                  usage >&2
                  exit 2
                  ;;
              esac
            done

            if [ "$#" -gt 0 ]; then
              log_error "unexpected positional arguments: $*"
              exit 2
            fi

            case "$PROFILE" in
              ci)
                ;;
              full)
                log_error "profile 'full' is no longer supported; use --profile ci."
                exit 2
                ;;
              *)
                log_error "unknown profile '$PROFILE' (expected: ci)"
                exit 2
                ;;
            esac

            case "$MODE" in
              basic|app|env|full)
                ;;
              *)
                log_error "unknown mode '$MODE' (expected: basic|app|env|full)"
                exit 2
                ;;
            esac

            if [ "$LIST_SHARDS" -eq 1 ]; then
              print_shards
              exit 0
            fi

            if [ -n "$SHARD" ] && ! shard_exists "$SHARD"; then
              log_error "unknown shard '$SHARD'"
              log_info "valid shards: $(print_shards | tr '\n' ' ')"
              exit 2
            fi

            cleanup() {
              local rc=$?
              if [ -n "$SUMMARY_JSON" ]; then
                write_summary_json "$rc"
              fi
              return "$rc"
            }
            trap cleanup EXIT

            if [ -n "$SHARD" ]; then
              run_named_shard "$SHARD"
            else
              for shard_name in "''${SHARDS[@]}"; do
                run_named_shard "$shard_name"
              done
            fi

            if [ "$SUMMARY" -eq 1 ]; then
              log_info "summary profile=$PROFILE mode=$MODE executed_shards=$EXECUTED"
            fi

            log_ok "framework::test completed"
          '';
        };

        framework-install = mkCommandTask {
          id = "task.framework.install";
          appName = "framework::install";
          kind = "utility";
          summary = "Install thin wrapper flake";
          description = "Creates a thin wrapper flake by default, or vendored wrapper with --vendor.";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.findutils
            pkgs.gnused
          ];
          usage = [
            "nix run .#framework::install"
            "nix run .#framework::install -- --vendor"
          ];
          command = ''
                        set -euo pipefail

                        source_root="${builtins.toString ../.}"
                        repo_root="${builtins.toString ../../.}"
                        target="."
                        vendor=0

                        while [ "$#" -gt 0 ]; do
                          case "$1" in
                            --vendor)
                              vendor=1
                              shift
                              ;;
                            --target)
                              if [ "$#" -lt 2 ]; then
                                echo "ERROR: --target requires a value"
                                exit 2
                              fi
                              target="$2"
                              shift 2
                              ;;
                            *)
                              echo "ERROR: unknown argument '$1'"
                              exit 2
                              ;;
                          esac
                        done

                        mkdir -p "$target"

                        if [ "$vendor" -eq 1 ]; then
                          if [ -e "$target/nixfied" ]; then
                            chmod -R u+w "$target/nixfied" 2>/dev/null || true
                            rm -rf "$target/nixfied"
                          fi
                          mkdir -p "$target/nixfied"
                          cp -R "$source_root/." "$target/nixfied"
                          if [ -f "$repo_root/README.md" ]; then
                            cp "$repo_root/README.md" "$target/nixfied/README.md"
                          fi
                          chmod -R u+w "$target/nixfied" 2>/dev/null || true
                          rm -rf "$target/nixfied/.git"
                          rm -f "$target/nixfied/result"
                          cat > "$target/flake.nix" <<'NIXFIED_WRAPPER'
            ${vendoredWrapperFlake}
            NIXFIED_WRAPPER
                          echo "OK: vendored wrapper flake generated at $target/flake.nix"
                        else
                          cat > "$target/flake.nix" <<'NIXFIED_WRAPPER'
            ${thinWrapperFlake}
            NIXFIED_WRAPPER
                          echo "OK: thin wrapper flake generated at $target/flake.nix"
                        fi
          '';
        };
      };

      workflows = {
        ci-basic = {
          id = "workflow.ci.basic";
          summary = "Basic CI workflow";
          description = "Runs quality and tests.";
          mode = "ci";
          maxWorkers = 4;
          units = {
            quality = {
              taskId = "task.ci.quality";
              needs = [ ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ ];
            };
            tests = {
              taskId = "task.ci.tests";
              needs = [ ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ ];
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };

        ci-app = {
          id = "workflow.ci.app";
          summary = "App CI workflow";
          description = "Basic workflow plus quick system checks.";
          mode = "ci";
          maxWorkers = 4;
          units = {
            quality = {
              taskId = "task.ci.quality";
              needs = [ ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ ];
            };
            tests = {
              taskId = "task.ci.tests";
              needs = [ "quality" ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ ];
            };
            system-quick = {
              taskId = "task.ci.system-quick";
              needs = [ "tests" ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ "API_KEY" ];
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };

        ci-env = {
          id = "workflow.ci.env";
          summary = "Environment CI workflow";
          description = "App workflow plus nginx proxy checks.";
          mode = "ci";
          maxWorkers = 4;
          units = {
            quality = {
              taskId = "task.ci.quality";
              needs = [ ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ ];
            };
            tests = {
              taskId = "task.ci.tests";
              needs = [ "quality" ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ ];
            };
            system-quick = {
              taskId = "task.ci.system-quick";
              needs = [ "tests" ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ "API_KEY" ];
            };
            nginx-proxy = {
              taskId = "task.ci.nginx-proxy";
              needs = [ "system-quick" ];
              locks = [ ];
              when = {
                envEquals = { };
                envPresent = [ ];
              };
              skipIfMissingEnv = [ ];
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };

        ci-full = {
          id = "workflow.ci.full";
          summary = "Full CI workflow";
          description = "Runs quality/tests then system checks as deterministic stages.";
          mode = "ci";
          maxWorkers = 2;
          units = { };
          stages = [
            [
              "task.ci.quality"
              "task.ci.tests"
            ]
            [
              "task.ci.system-quick"
              "task.ci.nginx-proxy"
            ]
          ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };
      };
    };
  };
}
