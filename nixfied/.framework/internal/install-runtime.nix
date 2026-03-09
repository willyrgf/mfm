{
  pkgs,
  filterHelpValues,
  filterAliasNotes,
  frameworkRevision ? "unknown",
  frameworkRoot,
  promptPlanScript,
  templateFilterPlanDataJson,
}:

pkgs.writeText "nixfied-install-runtime.sh" ''
  require_next_arg() {
    local flag="$1"
    local requirement="$2"
    shift 2 || true
    if [ "$#" -lt 2 ]; then
      log_error "$flag requires $requirement"
      exit 1
    fi
    printf '%s\n' "$2"
  }

  PROJECT_TEMPLATE_FILTER_PLAN_DATA_JSON=${pkgs.lib.escapeShellArg templateFilterPlanDataJson}

  compute_template_filter_plan() {
    local filters_raw="$1"

    printf '%s' "$PROJECT_TEMPLATE_FILTER_PLAN_DATA_JSON" | ${pkgs.jq}/bin/jq -c --arg filtersRaw "$filters_raw" '
      . as $data
      | ($data.requiredKeys // []) as $requiredKeys
      | ($data.optionalTemplates // []) as $optionalTemplates
      | ($data.tokenToKey // {}) as $tokenToKey
      | ($filtersRaw | split(",") | map(ascii_downcase) | map(select(length > 0))) as $tokens
      | reduce $tokens[] as $token (
          {
            unknown: [],
            selectedKeys: []
          };
          ([ $tokenToKey[$token] ] | map(select(. != null)) | first) as $matchedKey
          | if $matchedKey == null then
              .unknown += [ $token ]
            else
              .selectedKeys += [ $matchedKey ]
            end
        )
      | .selectedKeys |= (unique | sort)
      | .keepKeys = (($requiredKeys + .selectedKeys) | unique | sort)
      | .keepKeys as $keepKeys
      | .pruneFiles = [
          $optionalTemplates[]
          | .key as $templateKey
          | select(($keepKeys | index($templateKey)) == null)
          | .file
        ]
      | .defaultNix = (
          [
            "{ pkgs ? null }:",
            "",
            "let",
            "  conf = import ./conf.nix { inherit pkgs; };",
            "  project = conf.project or { };",
            "  frameworkLib = import ../.framework/lib { inherit pkgs; project = conf; };",
            "  commandLib = import ./lib/command.nix { inherit project; appApi = frameworkLib.appApi; };",
            "  mkPart = path: import path { inherit pkgs project commandLib; };",
            "  parts = [",
            "    conf"
          ]
          + [
            $optionalTemplates[]
            | .key as $templateKey
            | select(($keepKeys | index($templateKey)) != null)
            | "    (mkPart ./\(.file))"
          ]
          + [
            "  ];",
            "in",
            "pkgs.lib.foldl\u0027 pkgs.lib.recursiveUpdate { } parts"
          ]
          | join("\n")
        )
    '
  }

  print_install_usage() {
    echo "Usage:"
    echo "  nix run github:willyrgf/nixfied#framework::install [options]"
    echo "  nix run github:willyrgf/nixfied#framework::upgrade [options]"
    echo ""
    echo "Install options:"
    echo "  --force                Overwrite existing nix files (flake.nix/flake.lock/nixfied)"
    echo "                         and, without --worktree, apply changes on the current branch"
    echo "  --filter=LIST          Fresh install only: install subset of project templates"
    echo "                         (values: ${filterHelpValues}${
      if filterAliasNotes != "" then "; ${filterAliasNotes}" else ""
    })"
    echo "  --worktree             Install into a git worktree for the nixfied branch (keeps current checkout unchanged)"
    echo "  --target=PATH          With --worktree: worktree directory path (default: <repo>_nixfied)"
    echo ""
    echo "Upgrade options:"
    echo "  --upgrade              Treat as an upgrade (preserves nixfied/project unless --reset-project)"
    echo "  --reset-project        Overwrite nixfied/project templates during upgrade"
    echo ""
    echo "Prompt plan:"
    echo "  --no-prompt-plan       Skip generating NIXFIED_PROMPT_PLAN.md (default is to generate)"
    echo "  --prompt-plan          Generate NIXFIED_PROMPT_PLAN.md after install/upgrade (best effort; default)"
    echo "  --prompt-plan-force    Overwrite existing NIXFIED_PROMPT_PLAN.md"
  }

  parse_install_args() {
    ORIG_ARGS=("$@")
    FORCE=false
    FILTERS_RAW=""
    TARGET_PATH=""
    USE_WORKTREE=false
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
          FILTERS_RAW="$(require_next_arg --filter "a value (example: --filter=conf,ci)" "$@")"
          shift 2
          ;;
        --target=*)
          TARGET_PATH="''${1#--target=}"
          shift
          ;;
        --target)
          TARGET_PATH="$(require_next_arg --target "a path" "$@")"
          shift 2
          ;;
        --worktree)
          USE_WORKTREE=true
          shift
          ;;
        --sync)
          log_error "unsupported option: --sync (removed; run without --sync)."
          exit 1
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
          print_install_usage
          exit 0
          ;;
        --)
          shift
          ;;
        *)
          shift
          ;;
      esac
    done

    if [ "$FORCE" = "true" ]; then
      export NIXFIED_INSTALL_FORCE=1
    fi
  }

  resolve_install_repo_root() {
    GIT="${pkgs.git}/bin/git"
    if [ ! -x "$GIT" ]; then
      log_error "git is required to install/upgrade."
      exit 1
    fi

    ROOT=$("$GIT" rev-parse --show-toplevel 2>/dev/null || true)
    if [ -z "$ROOT" ]; then
      log_error "Not inside a git repository."
      exit 1
    fi
    ROOT=$(cd "$ROOT" && pwd -P)
    INSTALL_BRANCH="''${NIXFIED_INSTALL_BRANCH:-nixfied}"
  }

  maybe_reenter_install_worktree() {
    if [ -z "''${NIXFIED_INSTALL_REENTRY:-}" ] && [ "$USE_WORKTREE" = "true" ]; then
      TARGET="''${TARGET_PATH:-''${ROOT}_''${INSTALL_BRANCH}}"
      case "$TARGET" in
        "$ROOT"/*)
          log_error "Target must not be inside the source repo (got: $TARGET)"
          exit 1
          ;;
      esac

      if [ -e "$TARGET" ]; then
        if [ "$FORCE" = "true" ]; then
          log_warn "Target already exists: $TARGET"
          echo "    Reusing existing target (no new worktree created)."
        else
          log_error "Target already exists: $TARGET"
          echo "   Remove it or pass --force to reuse." >&2
          exit 1
        fi
      else
        DIRTY=$("$GIT" -C "$ROOT" status --porcelain 2>/dev/null || true)
        if [ -n "$DIRTY" ] && [ "$FORCE" != "true" ]; then
          log_error "Working tree is dirty; refusing to create a worktree without --force"
          echo "   (uncommitted changes would not be present in the worktree)" >&2
          exit 1
        fi
        log_info "Creating git worktree at $TARGET (branch: $INSTALL_BRANCH)..."
        if "$GIT" -C "$ROOT" show-ref --verify --quiet "refs/heads/$INSTALL_BRANCH"; then
          "$GIT" -C "$ROOT" worktree add "$TARGET" "$INSTALL_BRANCH" >/dev/null
        else
          "$GIT" -C "$ROOT" worktree add -b "$INSTALL_BRANCH" "$TARGET" >/dev/null
        fi
      fi

      if ! "$GIT" -C "$TARGET" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        log_error "Target exists but is not a git worktree: $TARGET"
        exit 1
      fi

      log_ok "Worktree ready. Re-running installer in $TARGET"
      (
        cd "$TARGET"
        NIXFIED_INSTALL_REENTRY=1 "$0" "''${ORIG_ARGS[@]}"
      )
      exit 0
    fi

    if [ -n "$TARGET_PATH" ] && [ "$USE_WORKTREE" != "true" ] && [ -z "''${NIXFIED_INSTALL_REENTRY:-}" ]; then
      log_info "--target is only used with --worktree; ignoring."
    fi
  }

  resolve_install_branch_context() {
    HEAD_REF=$("$GIT" -C "$ROOT" symbolic-ref -q HEAD 2>/dev/null || true)
    if [[ "$HEAD_REF" == refs/heads/* ]]; then
      CURRENT_BRANCH="''${HEAD_REF#refs/heads/}"
    else
      CURRENT_BRANCH=$("$GIT" -C "$ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    fi

    TARGET_BRANCH="$INSTALL_BRANCH"
    if [ "$FORCE" = "true" ] && [ "$USE_WORKTREE" != "true" ]; then
      TARGET_BRANCH="$CURRENT_BRANCH"
      log_info "--force enabled; applying $MODE on current branch: $TARGET_BRANCH"
    fi

    if [ "$MODE" = "upgrade" ]; then
      if [ ! -d "$ROOT/nixfied" ]; then
        log_error "No nixfied/ directory found in $ROOT"
        echo "   Run install first: nix run github:willyrgf/nixfied#framework::install" >&2
        exit 1
      fi
      if [ "$TARGET_BRANCH" = "$INSTALL_BRANCH" ] && [ "$HEAD_REF" != "refs/heads/$INSTALL_BRANCH" ] && ! "$GIT" -C "$ROOT" show-ref --verify --quiet "refs/heads/$INSTALL_BRANCH"; then
        log_error "Upgrade requires an existing branch: $INSTALL_BRANCH"
        exit 1
      fi
    fi

    if [ "$CURRENT_BRANCH" != "$TARGET_BRANCH" ]; then
      if "$GIT" -C "$ROOT" show-ref --verify --quiet "refs/heads/$TARGET_BRANCH"; then
        DIRTY=$("$GIT" -C "$ROOT" status --porcelain 2>/dev/null || true)
        if [ -n "$DIRTY" ] && [ "$FORCE" != "true" ]; then
          log_error "Working tree is dirty; refusing to switch to '$TARGET_BRANCH' without --force"
          echo "   (commit/stash your changes, or pass --worktree)" >&2
          exit 1
        fi
        log_info "Switching to $TARGET_BRANCH branch..."
        "$GIT" -C "$ROOT" switch "$TARGET_BRANCH" >/dev/null
      else
        if [ "$MODE" = "upgrade" ]; then
          log_error "Upgrade requires an existing branch: $TARGET_BRANCH"
          exit 1
        fi
        log_info "Creating and switching to $TARGET_BRANCH branch..."
        "$GIT" -C "$ROOT" switch -c "$TARGET_BRANCH" >/dev/null
      fi
    fi
  }

  resolve_framework_revision() {
    FRAMEWORK_REVISION=${pkgs.lib.escapeShellArg frameworkRevision}
    if [ -z "$FRAMEWORK_REVISION" ] || [ "$FRAMEWORK_REVISION" = "unknown" ]; then
      FRAMEWORK_REVISION=$("$GIT" -C "$SRC" rev-parse --short=12 HEAD 2>/dev/null || true)
    fi
    if [ -z "$FRAMEWORK_REVISION" ]; then
      FRAMEWORK_REVISION="unknown"
    fi
    log_info "Framework source revision rev=$FRAMEWORK_REVISION"
  }

  maybe_confirm_install_overwrite() {
    local needs_overwrite=false

    if [ -e "$ROOT/flake.nix" ] || [ -e "$ROOT/flake.lock" ] || [ -d "$ROOT/nixfied" ]; then
      needs_overwrite=true
    fi

    if [ "$needs_overwrite" = "true" ] && [ -z "''${NIXFIED_INSTALL_FORCE:-}" ]; then
      if [ -t 0 ]; then
        if [ -d "$ROOT/nixfied/project" ] && [ "$RESET_PROJECT" != "true" ]; then
          log_warn "Existing Nixfied install found in $ROOT"
          echo "    This will upgrade framework files and preserve nixfied/project/"
          echo "    It will overwrite: flake.nix, flake.lock, nixfied/ (except nixfied/project/)"
        else
          log_warn "Existing Nix files found in $ROOT"
          echo "    This will overwrite: flake.nix, flake.lock, nixfied/"
        fi
        echo -n "Continue? [y/N]: "
        read -r REPLY
        if [[ ! "$REPLY" =~ ^[Yy]$ ]]; then
          echo "Aborted."
          exit 1
        fi
      else
        log_error "Existing Nix files found. Re-run with NIXFIED_INSTALL_FORCE=1 to overwrite."
        exit 1
      fi
    fi
  }

  maybe_generate_prompt_plan() {
    local plan_exit=0

    if [ "$PROMPT_PLAN" != "true" ]; then
      return 0
    fi

    set +e
    if [ "$PROMPT_PLAN_FORCE" = "true" ]; then
      ${promptPlanScript} --force
    else
      ${promptPlanScript}
    fi
    plan_exit=$?
    set -e

    if [ "$plan_exit" -ne 0 ]; then
      log_warn "Prompt plan generation failed or was skipped."
    fi
  }
''
