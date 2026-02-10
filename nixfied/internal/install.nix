# Installer app for the framework
{
  pkgs,
  lib,
  frameworkRoot,
}:

let
  inherit (lib.appApi) mkNixfiedApp;

  promptPlanScript = pkgs.writeShellScript "nixfied-prompt-plan" ''
                            set -euo pipefail

                            FORCE=false
                            OUT_PATH=""

                            while [ "$#" -gt 0 ]; do
                              case "$1" in
                                --force)
                                  FORCE=true
                                  shift
                                  ;;
                                --output=*)
                                  OUT_PATH="''${1#--output=}"
                                  shift
                                  ;;
                	                --output)
                	                  if [ "$#" -lt 2 ]; then
                	                    echo "ERROR: --output requires a path" >&2
                	                    exit 1
                	                  fi
                                  OUT_PATH="''${2-}"
                                  shift 2
                                  ;;
                                --help|-h)
                                  echo "Usage: nix run github:willyrgf/nixfied#framework::prompt-plan [--force] [--output=PATH]"
                                  exit 0
                                  ;;
                                --)
                                  # Accept an explicit "--" (some wrappers include it) and keep parsing.
                                  shift
                                  ;;
                                *)
                                  # Ignore unknown args for forward compatibility.
                                  shift
                                  ;;
                              esac
                            done

                	            if [ "''${NIXFIED_PROMPT_PLAN:-1}" = "0" ] || [ "''${NIXFIED_PROMPT_PLAN:-}" = "false" ] || [ "''${NIXFIED_INTEGRATION_PLAN:-}" = "0" ] || [ "''${NIXFIED_INTEGRATION_PLAN:-}" = "false" ]; then
                	              echo "INFO: Prompt plan disabled (NIXFIED_PROMPT_PLAN=0)."
                	              exit 0
                	            fi

                	            ROOT=$(git rev-parse --show-toplevel 2>/dev/null || true)
                	            if [ -z "$ROOT" ]; then
                	              echo "ERROR: Not inside a git repository." >&2
                	              exit 1
                	            fi

                            OUT_PATH="''${OUT_PATH:-$ROOT/NIXFIED_PROMPT_PLAN.md}"

                	            if [ -f "$OUT_PATH" ] && [ "$FORCE" = "false" ] && [ "''${NIXFIED_PROMPT_PLAN_OVERWRITE:-0}" != "1" ] && [ "''${NIXFIED_INTEGRATION_PLAN_OVERWRITE:-0}" != "1" ]; then
                	              echo "INFO: Prompt plan already exists: $OUT_PATH"
                	              echo "    Re-run with --force or NIXFIED_PROMPT_PLAN_OVERWRITE=1 to overwrite."
                	              exit 0
                	            fi

                	            if ! command -v nix >/dev/null 2>&1; then
                	              echo "ERROR: nix is required to run dump2llm." >&2
                	              exit 1
                	            fi

                        CONTEXT_FILE=$(mktemp)
                        PROMPT_FILE=$(mktemp)
                        TMPDIR=$(mktemp -d)
                        trap 'rm -rf "$TMPDIR" "$CONTEXT_FILE" "$PROMPT_FILE"' EXIT

                        INPUTS=()

                        if [ -f "$ROOT/README.md" ]; then
                          cp "$ROOT/README.md" "$TMPDIR/PROJECT_README.md"
                          INPUTS+=("PROJECT_README.md")
                        fi
                        if [ -f "$ROOT/CLAUDE.md" ]; then
                          cp "$ROOT/CLAUDE.md" "$TMPDIR/PROJECT_CLAUDE.md"
                          INPUTS+=("PROJECT_CLAUDE.md")
                        fi
                        if [ -f "$ROOT/AGENTS.md" ]; then
                          cp "$ROOT/AGENTS.md" "$TMPDIR/PROJECT_AGENTS.md"
                          INPUTS+=("PROJECT_AGENTS.md")
                        fi

                        FRAMEWORK_README="${frameworkRoot}/README.md"
                        if [ -f "$FRAMEWORK_README" ]; then
                          cp "$FRAMEWORK_README" "$TMPDIR/NIXFIED_FRAMEWORK_README.md"
                          INPUTS+=("NIXFIED_FRAMEWORK_README.md")
                        fi

                	        if [ "''${#INPUTS[@]}" -eq 0 ]; then
                	          echo "SKIP: Skipping prompt plan (no README/CLAUDE/AGENTS files found)." >&2
                	          exit 0
                	        fi

                        cat > "$PROMPT_FILE" <<'EOF'
                    Create a PROMPT PLAN in Markdown for integrating this project with the Nixfied framework.

                    Requirements:
                    - Be concise and actionable.
                    - Use headings: "PROMPT PLAN", "Project Snapshot", "Current Behavior", "Integration Steps",
                      "Key Files to Edit", "Validation Checklist", "Open Questions", and "Next Prompts".
                    - Ground every step in the provided context; do not guess missing details.
                    - Treat Nixfied as the single entrypoint for dev/test/build/check/ci and (optionally) db/nginx/supervisor:
                      nix run .#help, .#dev, .#test, .#build, .#check, .#ci
                    - Call out the key file-to-command mapping (do not assume "prod" is a command):
                      - nixfied/project/dev.nix -> commands.dev
                      - nixfied/project/test.nix -> commands.test
                      - nixfied/project/prod.nix -> commands.build (build/prod workflow)
                      - nixfied/project/quality.nix -> commands.check
                      - nixfied/project/ci.nix -> CI pipeline DSL config (ci.modes/ci.steps) + CI command metadata
                      - nixfied/project/conf.nix -> project identity, envs/ports, module toggles, ephemeral config
                      - nixfied/project/default.nix -> merges all project files; update if new files are added
                    - Mention the primary customization surface is nixfied/project/ (avoid editing flake.nix unless the plan proves it's necessary).
                    - Reference relevant framework features (only if applicable to this project):
                      - CI pipeline DSL (modes/steps, artifacts, summary.json; supports --summary, --mode/--<mode>, --bg)
                      - Ephemeral environments (slot locking, source copy, conditional cleanup; ci.useEphemeral)
                      - Module apps + hooks (db-*, nginx-*, supervisor apps; postgres backups/migrations)
                      - Run registry (used by CI --bg mode)
                    - In "Integration Steps", start with high-level goals (behavior parity with the current dev/test/build/check/ci workflows, avoid regressions), then list concrete wiring steps with exact file paths.
                    - In "Key Files to Edit", list each file and the specific changes needed.
                    - In "Validation Checklist", include concrete smoke checks (nix run .#help/.#dev/.#test/.#build/.#check/.#ci -- --summary) and any project-specific checks from the docs.
                    - Include documentation alignment goals (README.md plus any agent instruction docs like CLAUDE.md/AGENTS.md should make Nixfied the canonical entrypoint).
                    - If docs conflict on command names or behavior, call it out and ask which source is authoritative.
                    - If info is missing, list it in "Open Questions".

                    Sources:
                    - Project docs: PROJECT_README.md, PROJECT_CLAUDE.md, PROJECT_AGENTS.md
                    - Nixfied docs: NIXFIED_FRAMEWORK_README.md

                    Context (project docs + framework README) follows:
    EOF

                        CONTEXT_STATUS=0
                        if ! (cd "$TMPDIR" && nix run github:willyrgf/dump2llm -- "''${INPUTS[@]}") > "$CONTEXT_FILE"; then
                          CONTEXT_STATUS=1
                        fi

                            {
                              echo "# NIXFIED PROMPT PLAN"
                              echo ""
                              cat "$PROMPT_FILE"
                              echo ""
                          if [ "$CONTEXT_STATUS" -eq 0 ]; then
                            cat "$CONTEXT_FILE"
                	          else
                	            echo ""
                	            echo "WARN: Context generation failed. Re-run the prompt plan:"
                	            echo ""
                	            echo "  nix run github:willyrgf/nixfied#framework::prompt-plan -- --force"
                	          fi
                	        } > "$OUT_PATH"

                	            echo "OK: Prompt plan written to $OUT_PATH"
                	  '';

  installScript = ''
                        set -euo pipefail

                        ORIG_ARGS=("$@")
                        FORCE=false
                        FILTERS_RAW=""
                        TARGET_PATH=""
                        USE_WORKTREE=false
                        SYNC_TARGET=false
                        PROMPT_PLAN=true
                        PROMPT_PLAN_FORCE=false
                        RESET_PROJECT=false
                        MODE="''${NIXFIED_INSTALL_MODE:-install}"

                        while [ "$#" -gt 0 ]; do
                          case "$1" in
                            --force)
                              FORCE=true
                              shift
                              ;;
                            --upgrade)
                              MODE="upgrade"
                              shift
                              ;;
                            --filter=*)
                              FILTERS_RAW="''${1#--filter=}"
                              shift
                              ;;
                	            --filter)
                	              if [ "$#" -lt 2 ]; then
                	                echo "ERROR: --filter requires a value (example: --filter=conf,ci)" >&2
                	                exit 1
                	              fi
                              FILTERS_RAW="''${2-}"
                              shift 2
                              ;;
                            --target=*)
                              TARGET_PATH="''${1#--target=}"
                              shift
                              ;;
                	            --target)
                	              if [ "$#" -lt 2 ]; then
                	                echo "ERROR: --target requires a path" >&2
                	                exit 1
                	              fi
                              TARGET_PATH="''${2-}"
                              shift 2
                              ;;
                            --worktree)
                              USE_WORKTREE=true
                              shift
                              ;;
                            --sync)
                              SYNC_TARGET=true
                              shift
                              ;;
                            --prompt-plan)
                              PROMPT_PLAN=true
                              shift
                              ;;
                            --prompt-plan-force)
                              PROMPT_PLAN=true
                              PROMPT_PLAN_FORCE=true
                              shift
                              ;;
                            --no-prompt-plan|--skip-prompt-plan)
                              PROMPT_PLAN=false
                              shift
                              ;;
                            --reset-project)
                              RESET_PROJECT=true
                              shift
                              ;;
                	        --help|-h)
                	          cat <<'EOF'
                Usage:
                  nix run github:willyrgf/nixfied#framework::install [options]
                  nix run github:willyrgf/nixfied#framework::upgrade [options]

                Install options:
                  --force                Overwrite existing nix files (flake.nix/flake.lock/nixfied)
                  --filter=LIST          Fresh install only: install subset of project templates
                                         (values: conf,dev,test,build,quality,ci; build is an alias of prod.nix)
                  --worktree             Install into a git worktree for the nixfied branch (keeps current checkout unchanged)
                  --target=PATH          With --worktree: worktree directory path (default: <repo>_nixfied)
                  --sync                 Deprecated (no-op); kept for backward compatibility

                Upgrade options:
                  --upgrade              Treat as an upgrade (preserves nixfied/project unless --reset-project)
                  --reset-project        Overwrite nixfied/project templates during upgrade

                Prompt plan:
                  --no-prompt-plan       Skip generating NIXFIED_PROMPT_PLAN.md (default is to generate)
                  --prompt-plan          Generate NIXFIED_PROMPT_PLAN.md after install/upgrade (best effort; default)
              --prompt-plan-force    Overwrite existing NIXFIED_PROMPT_PLAN.md
    EOF
                	          exit 0
                	          ;;
                            --)
                              # Accept an explicit "--" (some wrappers include it) and keep parsing.
                              shift
                              ;;
                            *)
                              # Ignore unknown args for forward compatibility.
                              shift
                              ;;
                          esac
                        done

                	        if [ "$FORCE" = "true" ]; then
                	          export NIXFIED_INSTALL_FORCE=1
                	        fi

                		        GIT="${pkgs.git}/bin/git"
                		        if [ ! -x "$GIT" ]; then
                		          echo "ERROR: git is required to install/upgrade." >&2
                		          exit 1
                		        fi

                		        ROOT=$("$GIT" rev-parse --show-toplevel 2>/dev/null || true)
                		        if [ -z "$ROOT" ]; then
                		          echo "ERROR: Not inside a git repository." >&2
                		          exit 1
                		        fi
                	        ROOT=$(cd "$ROOT" && pwd -P)

                	        INSTALL_BRANCH="''${NIXFIED_INSTALL_BRANCH:-nixfied}"

                		        if [ "$SYNC_TARGET" = "true" ]; then
                		          echo "INFO: --sync is deprecated in the branch-based installer (no-op)."
                		        fi

                	        if [ -z "''${NIXFIED_INSTALL_REENTRY:-}" ] && [ "$USE_WORKTREE" = "true" ]; then
                		          TARGET="''${TARGET_PATH:-''${ROOT}_''${INSTALL_BRANCH}}"
                		          case "$TARGET" in
                		            "$ROOT"/*)
                		              echo "ERROR: Target must not be inside the source repo (got: $TARGET)" >&2
                		              exit 1
                		              ;;
                		          esac

                		          if [ -e "$TARGET" ]; then
                		            if [ "$FORCE" = "true" ]; then
                		              echo "WARN: Target already exists: $TARGET"
                		              echo "    Reusing existing target (no new worktree created)."
                		            else
                		              echo "ERROR: Target already exists: $TARGET" >&2
                		              echo "   Remove it or pass --force to reuse." >&2
                		              exit 1
                		            fi
                		          else
                		            DIRTY=$("$GIT" -C "$ROOT" status --porcelain 2>/dev/null || true)
                		            if [ -n "$DIRTY" ] && [ "$FORCE" != "true" ]; then
                		              echo "ERROR: Working tree is dirty; refusing to create a worktree without --force" >&2
                		              echo "   (uncommitted changes would not be present in the worktree)" >&2
                		              exit 1
                		            fi
                		            echo "INFO: Creating git worktree at $TARGET (branch: $INSTALL_BRANCH)..."
                		            if "$GIT" -C "$ROOT" show-ref --verify --quiet "refs/heads/$INSTALL_BRANCH"; then
                		              "$GIT" -C "$ROOT" worktree add "$TARGET" "$INSTALL_BRANCH" >/dev/null
                		            else
                		              "$GIT" -C "$ROOT" worktree add -b "$INSTALL_BRANCH" "$TARGET" >/dev/null
                		            fi
                		          fi

                		          if ! "$GIT" -C "$TARGET" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
                		            echo "ERROR: Target exists but is not a git worktree: $TARGET" >&2
                		            exit 1
                		          fi

                		          echo "OK: Worktree ready. Re-running installer in $TARGET"
                		          (cd "$TARGET" && NIXFIED_INSTALL_REENTRY=1 "$0" "''${ORIG_ARGS[@]}")
                		          exit 0
                		        fi

                		        if [ -n "$TARGET_PATH" ] && [ "$USE_WORKTREE" != "true" ] && [ -z "''${NIXFIED_INSTALL_REENTRY:-}" ]; then
                		          echo "INFO: --target is only used with --worktree; ignoring."
                		        fi

                	        HEAD_REF=$("$GIT" -C "$ROOT" symbolic-ref -q HEAD 2>/dev/null || true)
                	        if [[ "$HEAD_REF" == refs/heads/* ]]; then
                	          CURRENT_BRANCH="''${HEAD_REF#refs/heads/}"
                	        else
                	          CURRENT_BRANCH=$("$GIT" -C "$ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
                	        fi

                		        if [ "$MODE" = "upgrade" ]; then
                		          if [ ! -d "$ROOT/nixfied" ]; then
                		            echo "ERROR: No nixfied/ directory found in $ROOT" >&2
                		            echo "   Run install first: nix run github:willyrgf/nixfied#framework::install" >&2
                		            exit 1
                		          fi
                		          if [ "$HEAD_REF" != "refs/heads/$INSTALL_BRANCH" ] && ! "$GIT" -C "$ROOT" show-ref --verify --quiet "refs/heads/$INSTALL_BRANCH"; then
                		            echo "ERROR: Upgrade requires an existing branch: $INSTALL_BRANCH" >&2
                		            exit 1
                		          fi
                		        fi

                		        if [ "$CURRENT_BRANCH" != "$INSTALL_BRANCH" ]; then
                		          if "$GIT" -C "$ROOT" show-ref --verify --quiet "refs/heads/$INSTALL_BRANCH"; then
                		            DIRTY=$("$GIT" -C "$ROOT" status --porcelain 2>/dev/null || true)
                		            if [ -n "$DIRTY" ] && [ "$FORCE" != "true" ]; then
                		              echo "ERROR: Working tree is dirty; refusing to switch to '$INSTALL_BRANCH' without --force" >&2
                		              echo "   (commit/stash your changes, or pass --worktree)" >&2
                		              exit 1
                		            fi
                		            echo "INFO: Switching to $INSTALL_BRANCH branch..."
                		            "$GIT" -C "$ROOT" switch "$INSTALL_BRANCH" >/dev/null
                		          else
                		            if [ "$MODE" = "upgrade" ]; then
                		              echo "ERROR: Upgrade requires an existing branch: $INSTALL_BRANCH" >&2
                		              exit 1
                		            fi
                		            echo "INFO: Creating and switching to $INSTALL_BRANCH branch..."
                		            "$GIT" -C "$ROOT" switch -c "$INSTALL_BRANCH" >/dev/null
                		          fi
                		        fi

                	        SRC="${frameworkRoot}"

                	        if [ ! -f "$SRC/flake.nix" ] || [ ! -d "$SRC/nixfied" ]; then
                	          echo "ERROR: Framework source is missing required files." >&2
                	          exit 1
                	        fi

                        NEEDS_OVERWRITE=false
                        if [ -e "$ROOT/flake.nix" ] || [ -e "$ROOT/flake.lock" ] || [ -d "$ROOT/nixfied" ]; then
                          NEEDS_OVERWRITE=true
                        fi

                	          if [ "$NEEDS_OVERWRITE" = "true" ] && [ -z "''${NIXFIED_INSTALL_FORCE:-}" ]; then
                	          if [ -t 0 ]; then
                	            if [ -d "$ROOT/nixfied/project" ] && [ "$RESET_PROJECT" != "true" ]; then
                	              echo "WARN: Existing Nixfied install found in $ROOT"
                	              echo "    This will upgrade framework files and preserve nixfied/project/"
                	              echo "    It will overwrite: flake.nix, flake.lock, nixfied/ (except nixfied/project/)"
                	            else
                	              echo "WARN: Existing Nix files found in $ROOT"
                	              echo "    This will overwrite: flake.nix, flake.lock, nixfied/"
                	            fi
                            echo -n "Continue? [y/N]: "
                            read -r REPLY
                            if [[ ! "$REPLY" =~ ^[Yy]$ ]]; then
                              echo "Aborted."
                              exit 1
                            fi
                	          else
                	            echo "ERROR: Existing Nix files found. Re-run with NIXFIED_INSTALL_FORCE=1 to overwrite." >&2
                	            exit 1
                	          fi
                	        fi

                	        echo "INFO: Installing framework files"

                        cp -f "$SRC/flake.nix" "$ROOT/flake.nix"
                        if [ -f "$SRC/flake.lock" ]; then
                          cp -f "$SRC/flake.lock" "$ROOT/flake.lock"
                        fi

                	        PRESERVE_PROJECT=false
                	        if [ -d "$ROOT/nixfied/project" ] && [ "$RESET_PROJECT" != "true" ]; then
                	          PRESERVE_PROJECT=true
                	        fi
                	        if [ "$MODE" = "upgrade" ] && [ "$RESET_PROJECT" != "true" ]; then
                	          PRESERVE_PROJECT=true
                	        fi
                	
                	        PRESERVE_LOCAL=false
                	        if [ -d "$ROOT/nixfied/local" ]; then
                	          PRESERVE_LOCAL=true
                	        fi

                		        if [ -n "$FILTERS_RAW" ] && [ "$PRESERVE_PROJECT" = "true" ]; then
                		          echo "INFO: Skipping --filter on upgrade (nixfied/project is preserved)."
                		          FILTERS_RAW=""
                		        fi

                        if [ -d "$ROOT/nixfied" ]; then
                          chmod -R u+w "$ROOT/nixfied" 2>/dev/null || true
                        fi

                	        RSYNC_EXCLUDES=()
                	        PRESERVE_MSG=""
                	        if [ "$PRESERVE_PROJECT" = "true" ]; then
                	          RSYNC_EXCLUDES+=(--exclude='/project/')
                	          PRESERVE_MSG="nixfied/project/"
                	        fi
                	        if [ "$PRESERVE_LOCAL" = "true" ]; then
                	          RSYNC_EXCLUDES+=(--exclude='/local/')
                	          if [ -n "$PRESERVE_MSG" ]; then
                	            PRESERVE_MSG="$PRESERVE_MSG and nixfied/local/"
                	          else
                	            PRESERVE_MSG="nixfied/local/"
                	          fi
                	        fi
                	
                		        if [ "''${#RSYNC_EXCLUDES[@]}" -gt 0 ]; then
                		          echo "INFO: Upgrading nixfied/ (preserving $PRESERVE_MSG)"
                		          ${pkgs.rsync}/bin/rsync -a --delete --chmod=Du+w,Fu+w "''${RSYNC_EXCLUDES[@]}" "$SRC/nixfied/" "$ROOT/nixfied/"
                		        else
                		          ${pkgs.rsync}/bin/rsync -a --delete --chmod=Du+w,Fu+w "$SRC/nixfied/" "$ROOT/nixfied/"
                		        fi

                        chmod -R u+w "$ROOT/nixfied" 2>/dev/null || true
                        if command -v chflags >/dev/null 2>&1; then
                          chflags -R nouchg "$ROOT/nixfied" 2>/dev/null || true
                        fi
                        if command -v chattr >/dev/null 2>&1; then
                          chattr -R -i "$ROOT/nixfied" 2>/dev/null || true
                        fi
                        chmod u+w "$ROOT/nixfied/.framework" 2>/dev/null || true
                        if command -v chflags >/dev/null 2>&1; then
                          chflags nouchg "$ROOT/nixfied/.framework" 2>/dev/null || true
                        fi
                        if command -v chattr >/dev/null 2>&1; then
                          chattr -i "$ROOT/nixfied/.framework" 2>/dev/null || true
                        fi
                        rm -f "$ROOT/nixfied/.framework"

                        if [ -n "$FILTERS_RAW" ]; then
                          IFS=',' read -r -a FILTERS <<< "$FILTERS_RAW"
                          declare -A KEEP
                          KEEP[conf]=1

                          for f in "''${FILTERS[@]}"; do
                            f="''${f,,}"
                            case "$f" in
                              build)
                                KEEP[prod]=1
                                ;;
                              conf|dev|test|prod|quality|ci)
                                KEEP["$f"]=1
                                ;;
                              "")
                                ;;
                	              *)
                	                echo "ERROR: Unknown filter: $f" >&2
                	                exit 1
                	                ;;
                	            esac
                	          done

                          for f in dev test prod quality ci; do
                            if [ -z "''${KEEP[$f]:-}" ]; then
                              rm -f "$ROOT/nixfied/project/$f.nix" 2>/dev/null || true
                            fi
                          done

                          {
                            echo "{ pkgs ? null }:"
                            echo ""
                            echo "let"
                            echo "  conf = import ./conf.nix { inherit pkgs; };"
                            echo "  project = conf.project or { };"
                            echo "  parts = ["
                            echo "    conf"
                            for f in dev test prod quality ci; do
                              if [ -n "''${KEEP[$f]:-}" ]; then
                                echo "    (import ./$f.nix { inherit pkgs project; })"
                              fi
                            done
                            echo "  ];"
                            echo "in"
                            echo "pkgs.lib.foldl' pkgs.lib.recursiveUpdate { } parts"
                          } > "$ROOT/nixfied/project/default.nix"
                        fi

                        if [ "$PROMPT_PLAN" = "true" ]; then
                          PLAN_EXIT=0
                          set +e
                          if [ "$PROMPT_PLAN_FORCE" = "true" ]; then
                            ${promptPlanScript} --force
                          else
                            ${promptPlanScript}
                          fi
                          PLAN_EXIT=$?
                          set -e
                	          if [ "$PLAN_EXIT" -ne 0 ]; then
                	            echo "WARN: Prompt plan generation failed or was skipped."
                	          fi
                	        fi

                		        if [ "$PRESERVE_PROJECT" = "true" ] || [ "$PRESERVE_LOCAL" = "true" ] || [ "$MODE" = "upgrade" ]; then
                		          echo "OK: Framework upgraded."
                		          echo "    Preserved nixfied/project/ and nixfied/local/ (pass --reset-project to overwrite project templates)."
                		        else
                		          echo "OK: Framework installed."
                		        fi
                        echo "Next:"
                        echo "  - Edit nixfied/project/conf.nix"
                        echo "  - Customize nixfied/project/{dev,test,prod,quality,ci}.nix (prod.nix defines the build command)"
  '';
in
{
  install = mkNixfiedApp {
    name = "install";
    api = {
      version = 1;
      summary = "Install Nixfied framework into a repository";
      details = "Installs the Nixfied framework into a target repository (writes flake.nix and nixfied/), optionally generating project scaffolding.";
      usage = [
        "nix run .#framework::install -- [--force] [--filter=...] [--reset-project] [--no-prompt-plan]"
      ];
      category = "framework";
    };
    env = { };
    useDeps = false;
    script = installScript;
  };

  upgrade = mkNixfiedApp {
    name = "upgrade";
    api = {
      version = 1;
      summary = "Upgrade Nixfied framework in-place (preserving nixfied/project and nixfied/local by default)";
      details = "Upgrades the Nixfied framework in-place. By default it preserves nixfied/project and nixfied/local so project-specific configuration and extensions remain intact.";
      usage = [ "nix run .#framework::upgrade -- [--force] [--reset-project] [--no-prompt-plan]" ];
      category = "framework";
    };
    env = {
      NIXFIED_INSTALL_MODE = "upgrade";
    };
    useDeps = false;
    script = installScript;
  };

  "prompt-plan" = mkNixfiedApp {
    name = "prompt-plan";
    api = {
      version = 1;
      summary = "Generate Nixfied prompt plan from project docs";
      details = "Generates a prompt plan document (for agents) from the repository's project docs. This is framework-only and can be disabled via NIXFIED_PROMPT_PLAN=0.";
      usage = [ "nix run .#framework::prompt-plan -- [--force] [--output=PATH]" ];
      category = "framework";
    };
    env = { };
    useDeps = false;
    script = ''
      ${promptPlanScript} "$@"
    '';
  };
}
