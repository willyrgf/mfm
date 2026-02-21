# Ephemeral Execution Environment
#
# Provides fully isolated, deterministic execution environments for CI and tests.
# All mutable state (build artifacts, database, logs) goes to a temporary
# directory that is:
#   - Cleaned up on success
#   - Preserved on failure for debugging
#
# SLOT LOCKING:
#   Each ephemeral run acquires an exclusive lock on a slot (0-9) to prevent
#   port conflicts between concurrent runs. The lock is held via an open file
#   descriptor for the duration of the run and auto-releases on exit.
#
# Usage:
#   mkEphemeralWrapper { name = "ci"; script = "..."; }
#
{
  pkgs,
  project,
  loggingPrelude ? null,
}:

let
  projectMeta = project.project or { };
  projectId = projectMeta.id or "project";
  projectIdUpper =
    let
      replaced = pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] projectId;
    in
    pkgs.lib.strings.toUpper replaced;

  slotVar = projectMeta.slotVar or "NIX_ENV";
  envVar = projectMeta.envVar or "PROJECT_ENV";

  ephemeralCfg = project.ephemeral or { };
  copyMode = ephemeralCfg.copyMode or "git-files";
  excludePatterns =
    ephemeralCfg.excludePatterns or [
      ".git"
      "node_modules"
      ".next"
      "dist"
      ".turbo"
      ".cache"
      "result"
      "result-*"
      "*.log"
      "test-results"
      "coverage"
    ];
  extraDirs = ephemeralCfg.extraDirs or [ ];
  keepFailures = ephemeralCfg.keepFailures or true;
  maxFailedRoots = ephemeralCfg.maxFailedRoots or 8;
  maxFailedRootAgeHours = ephemeralCfg.maxFailedRootAgeHours or 72;
  maxCopyBytes = ephemeralCfg.maxCopyBytes or 0;
  minFreeBytesAfterCopy = ephemeralCfg.minFreeBytesAfterCopy or 0;

  depsScript = project.install.deps or "";
  runtimePackages = project.tooling.runtimePackages or [ ];
  id = import ./lib/id.nix {
    inherit pkgs project;
    loggingPrelude = resolvedLoggingPrelude;
  };
  envLoader = import ./lib/env-loader.nix {
    inherit pkgs project;
    loggingPrelude = resolvedLoggingPrelude;
  };
  processRegistry =
    if builtins.pathExists ./lib/process-registry.nix then
      import ./lib/process-registry.nix {
        inherit pkgs project;
        loggingPrelude = resolvedLoggingPrelude;
      }
    else
      {
        emitEvent = pkgs.writeShellScript "emit-event-noop" ''
          exit 0
        '';
      };
  shellContract = import ./lib/shell-contract.nix { inherit pkgs; };

  lockPrefix = "${projectId}-slot";

  slotMax = (project.slots or { }).max or 9;
  resolvedLoggingPrelude =
    if loggingPrelude != null && loggingPrelude != "" then
      loggingPrelude
    else
      (import ./lib/helpers.nix {
        inherit pkgs project;
        hooks = { };
        summaryParser = "";
      }).loggingPrelude;

  # Pre-computed bash variable references
  # Nix $${var} doesn't interpolate; use "\$${var}" in "..." strings instead
  refEphRoot = "\$${projectIdUpper}_EPHEMERAL_ROOT";
  refEphSlot = "\$${projectIdUpper}_EPHEMERAL_SLOT";
  refLockFd = "\$${projectIdUpper}_SLOT_LOCK_FD";

  mkUniqueId = id.mkUniqueId;

  acquireSlotLock = pkgs.writeShellScript "acquire-slot-lock" ''
    ${resolvedLoggingPrelude}

    set -euo pipefail

    LOCK_DIR="''${NIXFIED_EPHEMERAL_LOCK_DIR:-''${TMPDIR:-/tmp}}"
    LOCK_PREFIX="${lockPrefix}"
    if ! mkdir -p "$LOCK_DIR"; then
      log_error "Unable to create ephemeral lock directory '$LOCK_DIR'"
      exit 1
    fi

    for slot in $(seq 0 ${toString slotMax}); do
      LOCK_FILE="$LOCK_DIR/$LOCK_PREFIX-$slot.lock"
      FD=$((200 + slot))
      if ! eval "exec $FD>\"$LOCK_FILE\""; then
        continue
      fi

      if ${pkgs.flock}/bin/flock -n "$FD" 2>/dev/null; then
        echo "export ${projectIdUpper}_EPHEMERAL_SLOT=$slot"
        echo "export ${projectIdUpper}_SLOT_LOCK_FD=$FD"
        echo "export ${slotVar}=$slot"
        exit 0
      else
        eval "exec $FD>&-"
      fi
    done

    log_error "All $((${toString slotMax} + 1)) ephemeral slots (0-${toString slotMax}) are in use"
    echo "" >&2
    echo "   This means $((${toString slotMax} + 1)) concurrent runs are already running." >&2
    echo "   Wait for one to complete or check for stale locks:" >&2
    echo "   ls -la $LOCK_DIR/$LOCK_PREFIX-*.lock" >&2
    echo "" >&2
    exit 1
  '';

  releaseSlotLock = pkgs.writeShellScript "release-slot-lock" ''
    if [ -n "''${${projectIdUpper}_SLOT_LOCK_FD:-}" ]; then
      eval "exec ${refLockFd}>&-" 2>/dev/null || true
    fi
  '';

  mkEphemeralRoot = pkgs.writeShellScript "mk-ephemeral-root" ''
    set -euo pipefail

    EPHEMERAL_BASE="''${NIXFIED_EPHEMERAL_ROOT_BASE:-''${TMPDIR:-/tmp}}"
    if ! mkdir -p "$EPHEMERAL_BASE"; then
      echo "ERROR: Unable to create ephemeral base directory '$EPHEMERAL_BASE'" >&2
      exit 1
    fi

    EPHEMERAL_ROOT="$(${pkgs.coreutils}/bin/mktemp -d "$EPHEMERAL_BASE/${projectId}-ephemeral-XXXXXX")"

    mkdir -p "$EPHEMERAL_ROOT"/{source,data/${projectId},build}
    ${pkgs.lib.concatMapStringsSep "\n" (dir: ''
      mkdir -p "$EPHEMERAL_ROOT/${dir}"
    '') extraDirs}

    echo "$EPHEMERAL_ROOT"
  '';

  rsyncExcludes = pkgs.lib.concatMapStringsSep " " (pat: "--exclude='${pat}'") excludePatterns;

  mkSourceCopy = pkgs.writeShellScript "mk-source-copy" ''
    ${resolvedLoggingPrelude}

    set -euo pipefail

    SOURCE_DIR="$1"
    DEST_DIR="$2"
    COPY_MODE=${pkgs.lib.escapeShellArg copyMode}
    MAX_COPY_BYTES=${toString maxCopyBytes}
    MIN_FREE_BYTES_AFTER_COPY=${toString minFreeBytesAfterCopy}

    extract_total_file_size_bytes() {
      local stats="$1"
      local total=""
      total="$(printf '%s\n' "$stats" | ${pkgs.gawk}/bin/awk '
        /^Total file size:/ {
          gsub(/[^0-9]/, "", $4)
          print $4
          exit
        }
      ')"
      if [ -z "$total" ]; then
        printf '0'
      else
        printf '%s' "$total"
      fi
    }

    available_bytes_for_dest() {
      local free_kib
      free_kib="$(${pkgs.coreutils}/bin/df -Pk "$DEST_DIR" | ${pkgs.gawk}/bin/awk 'NR==2 {print $4}')"
      if [ -z "$free_kib" ]; then
        printf '0'
      else
        printf '%s' "$((free_kib * 1024))"
      fi
    }

    enforce_copy_budget() {
      local copy_bytes="$1"
      local free_bytes
      local remaining_bytes
      free_bytes="$(available_bytes_for_dest)"
      remaining_bytes=$((free_bytes - copy_bytes))

      log_info "Ephemeral copy budget bytes_required=$copy_bytes bytes_free=$free_bytes bytes_remaining=$remaining_bytes"

      if [ "$MAX_COPY_BYTES" -gt 0 ] && [ "$copy_bytes" -gt "$MAX_COPY_BYTES" ]; then
        log_error "ephemeral copy budget exceeded: bytes_required=$copy_bytes max_copy_bytes=$MAX_COPY_BYTES"
        exit 1
      fi

      if [ "$free_bytes" -le "$copy_bytes" ]; then
        log_error "ephemeral copy budget exceeded: bytes_required=$copy_bytes bytes_free=$free_bytes"
        exit 1
      fi

      if [ "$MIN_FREE_BYTES_AFTER_COPY" -gt 0 ] && [ "$remaining_bytes" -lt "$MIN_FREE_BYTES_AFTER_COPY" ]; then
        log_error "ephemeral copy budget exceeded: bytes_remaining=$remaining_bytes min_free_after_copy=$MIN_FREE_BYTES_AFTER_COPY"
        exit 1
      fi
    }

    static_copy() {
      local dry_run_stats copy_bytes
      log_info "Using static-excludes copy mode"
      dry_run_stats="$(${pkgs.rsync}/bin/rsync -an --stats \
        ${rsyncExcludes} \
        "$SOURCE_DIR/" "$DEST_DIR/")"
      copy_bytes="$(extract_total_file_size_bytes "$dry_run_stats")"
      enforce_copy_budget "$copy_bytes"
      ${pkgs.rsync}/bin/rsync -a \
        ${rsyncExcludes} \
        "$SOURCE_DIR/" "$DEST_DIR/"
    }

    git_copy() {
      local manifest dry_run_stats copy_bytes
      manifest="$(${pkgs.coreutils}/bin/mktemp)"

      (
        cd "$SOURCE_DIR"
        ${pkgs.git}/bin/git ls-files -z --cached --others --exclude-standard
      ) > "$manifest"

      log_info "Using git-files copy mode"

      if [ ! -s "$manifest" ]; then
        log_warn "Git file manifest is empty; source copy may be incomplete"
      fi

      dry_run_stats="$(${pkgs.rsync}/bin/rsync -an --stats --from0 --files-from="$manifest" "$SOURCE_DIR/" "$DEST_DIR/")"
      copy_bytes="$(extract_total_file_size_bytes "$dry_run_stats")"
      enforce_copy_budget "$copy_bytes"
      ${pkgs.rsync}/bin/rsync -a --from0 --files-from="$manifest" "$SOURCE_DIR/" "$DEST_DIR/"
      rm -f "$manifest"
    }

    log_info "Copying project source to ephemeral location"

    case "$COPY_MODE" in
      git-files)
        if ${pkgs.git}/bin/git -C "$SOURCE_DIR" rev-parse --show-toplevel >/dev/null 2>&1; then
          git_copy
        else
          log_warn "git-files copy mode unavailable outside a git worktree; falling back to static-excludes"
          static_copy
        fi
        ;;
      static-excludes)
        static_copy
        ;;
      *)
        log_error "Unsupported ephemeral copy mode: $COPY_MODE"
        exit 2
        ;;
    esac

    log_ok "Source copied to $DEST_DIR"
  '';

  mkConditionalCleanup = pkgs.writeShellScript "mk-conditional-cleanup" ''
    ${resolvedLoggingPrelude}

    remove_tree() {
      local target="$1"
      if rm -rf "$target" 2>/dev/null; then
        return 0
      fi
      chmod -R u+w "$target" 2>/dev/null || true
      rm -rf "$target" 2>/dev/null
    }

    prune_failed_roots() {
      local base_dir="$1"
      local prefix="${projectId}-ephemeral-failed-"
      local keep_limit=${toString maxFailedRoots}
      local age_hours=${toString maxFailedRootAgeHours}
      local age_minutes=0
      local failed_paths

      if ! [ -d "$base_dir" ]; then
        return 0
      fi

      if [ "$age_hours" -gt 0 ]; then
        age_minutes=$((age_hours * 60))
        while IFS= read -r old_path; do
          [ -z "$old_path" ] && continue
          remove_tree "$old_path" || true
          log_info "Pruned failed ephemeral root (age): $old_path"
        done < <(${pkgs.findutils}/bin/find "$base_dir" -mindepth 1 -maxdepth 1 -type d -name "$prefix*" -mmin "+$age_minutes" | sort)
      fi

      mapfile -t failed_paths < <(${pkgs.findutils}/bin/find "$base_dir" -mindepth 1 -maxdepth 1 -type d -name "$prefix*" | sort)

      if [ "$keep_limit" -ge 0 ] && [ "''${#failed_paths[@]}" -gt "$keep_limit" ]; then
        local to_remove_count
        to_remove_count=$(( ''${#failed_paths[@]} - keep_limit ))
        local i
        for i in $(seq 0 $((to_remove_count - 1))); do
          remove_tree "''${failed_paths[$i]}" || true
          log_info "Pruned failed ephemeral root (count): ''${failed_paths[$i]}"
        done
      fi
    }

    _ephemeral_cleanup() {
      local exit_code=$?
      local eph_root="${refEphRoot}"
      local eph_base=""

      if [ -n "''${_EPHEMERAL_CHILD_PIDS:-}" ]; then
        for pid in $_EPHEMERAL_CHILD_PIDS; do
          kill -TERM "$pid" 2>/dev/null || true
        done
      fi

      if [ $exit_code -ne 0 ]; then
        if [ -d "$eph_root" ]; then
          eph_base="$(${pkgs.coreutils}/bin/dirname "$eph_root")"
          if [ "${if keepFailures then "1" else "0"}" = "1" ]; then
            local failed_stamp slot_label failed_root
            failed_stamp="$(${pkgs.coreutils}/bin/date -u +%Y%m%d-%H%M%S)"
            slot_label="''${${slotVar}:-unknown}"
            slot_label="$(printf '%s' "$slot_label" | ${pkgs.gnused}/bin/sed -E 's/[^A-Za-z0-9_.-]+/_/g')"
            failed_root="$eph_base/${projectId}-ephemeral-failed-$failed_stamp-slot$slot_label-$$"
            if mv "$eph_root" "$failed_root" 2>/dev/null; then
              log_warn "Preserving failed ephemeral state at $failed_root"
            else
              failed_root="$eph_root"
              log_warn "Preserving failed ephemeral state at $failed_root"
            fi
            prune_failed_roots "$eph_base"
          else
            if remove_tree "$eph_root"; then
              log_info "Removed failed ephemeral state root=$eph_root"
            else
              log_warn "Unable to remove failed ephemeral root=$eph_root"
            fi
          fi
        fi
      else
        echo ""
        log_info "Cleaning up ephemeral state (slot ''${${projectIdUpper}_EPHEMERAL_SLOT:-unknown})"
        if remove_tree "$eph_root"; then
          log_ok "Ephemeral state cleaned"
        else
          log_warn "Ephemeral cleanup incomplete root=$eph_root; preserving for manual cleanup"
        fi
      fi

      ${processRegistry.emitEvent} \
        --event-type slot_released \
        --state released \
        --slot "''${${slotVar}:-}" \
        --env "''${${envVar}:-}" \
        --wait-reason "ephemeral_cleanup exit_code=$exit_code" >/dev/null 2>&1 || true

      if [ -n "''${${projectIdUpper}_SLOT_LOCK_FD:-}" ]; then
        eval "exec ${refLockFd}>&-" 2>/dev/null || true
      fi

      return $exit_code
    }
  '';

  mkEphemeralWrapper =
    {
      name,
      script,
      installDeps ? true,
      extraEnv ? "",
      appContract ? null,
    }:
    let
      runtimePath = if runtimePackages == [ ] then "" else pkgs.lib.makeBinPath runtimePackages;
      pathBlock = if runtimePath != "" then ''export PATH="${runtimePath}:$PATH"'' else "";
      contractFile =
        if appContract == null then
          null
        else
          pkgs.writeText "ephemeral-${name}-app-contract.json" (builtins.toJSON appContract);
      contractPrelude =
        if appContract == null then
          ""
        else
          ''
            NIXFIED_APP_CONTRACT_FILE="${toString contractFile}"
            source ${toString shellContract.runtime}
            nixfied_contract_validate_env "$NIXFIED_APP_CONTRACT_FILE"
            nixfied_contract_validate_args "$NIXFIED_APP_CONTRACT_FILE" "$@"
          '';
      contractExitCheck =
        if appContract == null then
          ""
        else
          ''
            _NIXFIED_CONTRACT_RC=0
            nixfied_contract_validate_exit "$NIXFIED_APP_CONTRACT_FILE" "$_NIXFIED_APP_RC" || _NIXFIED_CONTRACT_RC=$?
            if [ "$_NIXFIED_CONTRACT_RC" -ne 0 ]; then
              exit "$_NIXFIED_CONTRACT_RC"
            fi
          '';
    in
    pkgs.writeShellScript "ephemeral-${name}" ''
      ${resolvedLoggingPrelude}

      set -euo pipefail

      CALLER_ROOT="''${NIXFIED_CALLER_PWD:-$(pwd -P)}"
      if ORIGINAL_ROOT="$(${pkgs.git}/bin/git -C "$CALLER_ROOT" rev-parse --show-toplevel 2>/dev/null)"; then
        export ORIGINAL_ROOT
      else
        export ORIGINAL_ROOT="$CALLER_ROOT"
      fi

      # Compatibility aliases:
      # - NIXFIED_ENV: alias for the configured slot variable (default: NIX_ENV).
      if [ -z "''${${slotVar}:-}" ] && [ -n "''${NIXFIED_ENV:-}" ]; then
        export ${slotVar}="''${NIXFIED_ENV}"
      fi

      # Slot acquisition: respect pre-set slot var (e.g. from test-isolation)
      if [ -n "''${${slotVar}:-}" ]; then
        export ${projectIdUpper}_EPHEMERAL_SLOT="''${${slotVar}}"
        export ${projectIdUpper}_SLOT_LOCK_FD=""
        log_info "Using pre-set slot: ''${${slotVar}} (no lock - caller managed)"
      else
        if ! eval "$(${acquireSlotLock})"; then
          exit 1
        fi
      fi

      export ${envVar}="''${${envVar}:-test}"

      if [ -z "''${RUN_ID:-}" ]; then
        export RUN_ID="$(${pkgs.coreutils}/bin/date -u +%Y%m%d-%H%M%S)-$$-''${RANDOM:-0}"
      fi

      export ${projectIdUpper}_EPHEMERAL_ROOT=$(${mkEphemeralRoot})
      export ${projectIdUpper}_EPHEMERAL=1

      echo ""
      log_info "Ephemeral execution mode"
      log_info "Root: ${refEphRoot}"
      log_info "Slot: ${refEphSlot} (${slotVar}=''${${slotVar}}, ${envVar}=''${${envVar}})"
      echo ""

      ${processRegistry.emitEvent} \
        --event-type slot_acquired \
        --state busy \
        --slot "''${${slotVar}}" \
        --env "''${${envVar}}" \
        --wait-reason "ephemeral_start" >/dev/null 2>&1 || true

      source ${mkConditionalCleanup}
      trap _ephemeral_cleanup EXIT INT TERM

      ${mkSourceCopy} "$ORIGINAL_ROOT" "${refEphRoot}/source"

      export XDG_DATA_HOME="${refEphRoot}/data"

      ${extraEnv}

      cd "${refEphRoot}/source"

      ${
        if installDeps && depsScript != "" then
          ''
            log_info "Installing dependencies"
            ${depsScript}
            log_ok "Dependencies installed"
          ''
        else
          ""
      }

      ${pathBlock}

      # Load .env from original location (secrets shouldn't be copied) and
      # source into this shell so exported keys are visible to app scripts.
      source ${envLoader.loadEnvFile} "$ORIGINAL_ROOT/.env"

      ${contractPrelude}

      echo ""
      log_info "Starting ${name}"
      echo ""

      _NIXFIED_APP_RC=0
      (
        set -euo pipefail
        export NIXFIED_CLEANUP_OWNER_BASHPID="''${BASHPID:-}"
        ${script}
      ) || _NIXFIED_APP_RC=$?
      ${contractExitCheck}
      exit "$_NIXFIED_APP_RC"
    '';

  isEphemeral = pkgs.writeShellScript "is-ephemeral" ''
    [ "''${${projectIdUpper}_EPHEMERAL:-}" = "1" ]
  '';

  getEphemeralPaths = pkgs.writeShellScript "get-ephemeral-paths" ''
    if [ "''${${projectIdUpper}_EPHEMERAL:-}" = "1" ] && [ -n "''${${projectIdUpper}_EPHEMERAL_ROOT:-}" ]; then
      echo "EPHEMERAL_SOURCE=${refEphRoot}/source"
      echo "EPHEMERAL_DATA=${refEphRoot}/data"
      echo "EPHEMERAL_BUILD=${refEphRoot}/build"
    else
      echo "EPHEMERAL_SOURCE=$(pwd)"
      echo "EPHEMERAL_DATA=''${XDG_DATA_HOME:-$HOME/.local/share}"
      echo "EPHEMERAL_BUILD=$(pwd)"
    fi
  '';

in
{
  inherit
    mkUniqueId
    mkEphemeralRoot
    mkSourceCopy
    mkConditionalCleanup
    mkEphemeralWrapper
    isEphemeral
    getEphemeralPaths
    acquireSlotLock
    releaseSlotLock
    lockPrefix
    ;
}
