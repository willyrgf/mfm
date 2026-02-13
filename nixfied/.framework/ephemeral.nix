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
{ pkgs, project }:

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
  excludePatterns =
    ephemeralCfg.excludePatterns or [
      ".git"
      "node_modules"
      ".next"
      "dist"
      ".turbo"
      ".cache"
      "*.log"
      "test-results"
      "coverage"
    ];
  extraDirs = ephemeralCfg.extraDirs or [ ];

  depsScript = project.install.deps or "";
  runtimePackages = project.tooling.runtimePackages or [ ];
  id = import ./lib/id.nix { inherit pkgs; };
  processRegistry = import ./lib/process-registry.nix { inherit pkgs project; };

  lockDir = "/tmp";
  lockPrefix = "${projectId}-slot";

  slotMax = (project.slots or { }).max or 9;

  # Pre-computed bash variable references
  # Nix $${var} doesn't interpolate; use "\$${var}" in "..." strings instead
  refEphRoot = "\$${projectIdUpper}_EPHEMERAL_ROOT";
  refEphSlot = "\$${projectIdUpper}_EPHEMERAL_SLOT";
  refLockFd = "\$${projectIdUpper}_SLOT_LOCK_FD";

  mkUniqueId = id.mkUniqueId;

  acquireSlotLock = pkgs.writeShellScript "acquire-slot-lock" ''
    set -euo pipefail

    LOCK_DIR="${lockDir}"
    LOCK_PREFIX="${lockPrefix}"

    for slot in $(seq 0 ${toString slotMax}); do
      LOCK_FILE="$LOCK_DIR/$LOCK_PREFIX-$slot.lock"
      FD=$((200 + slot))
      eval "exec $FD>\"$LOCK_FILE\""

      if ${pkgs.flock}/bin/flock -n "$FD" 2>/dev/null; then
        echo "export ${projectIdUpper}_EPHEMERAL_SLOT=$slot"
        echo "export ${projectIdUpper}_SLOT_LOCK_FD=$FD"
        echo "export ${slotVar}=$slot"
        exit 0
      else
        eval "exec $FD>&-"
      fi
    done

    echo "ERROR: All $((${toString slotMax} + 1)) ephemeral slots (0-${toString slotMax}) are in use" >&2
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

    UNIQUE_ID=$(${mkUniqueId})
    EPHEMERAL_ROOT="/tmp/${projectId}-ephemeral-$UNIQUE_ID"

    mkdir -p "$EPHEMERAL_ROOT"/{source,data/${projectId},build}
    ${pkgs.lib.concatMapStringsSep "\n" (dir: ''
      mkdir -p "$EPHEMERAL_ROOT/${dir}"
    '') extraDirs}

    echo "$EPHEMERAL_ROOT"
  '';

  rsyncExcludes = pkgs.lib.concatMapStringsSep " " (pat: "--exclude='${pat}'") excludePatterns;

  mkSourceCopy = pkgs.writeShellScript "mk-source-copy" ''
    set -euo pipefail

    SOURCE_DIR="$1"
    DEST_DIR="$2"

    echo "INFO: Copying project source to ephemeral location"

    ${pkgs.rsync}/bin/rsync -a \
      ${rsyncExcludes} \
      "$SOURCE_DIR/" "$DEST_DIR/"

    echo "OK: Source copied to $DEST_DIR"
  '';

  mkConditionalCleanup = pkgs.writeShellScript "mk-conditional-cleanup" ''
    _ephemeral_cleanup() {
      local exit_code=$?

      if [ -n "''${_EPHEMERAL_CHILD_PIDS:-}" ]; then
        for pid in $_EPHEMERAL_CHILD_PIDS; do
          kill -TERM "$pid" 2>/dev/null || true
        done
      fi

      if [ $exit_code -ne 0 ]; then
        :
      else
        echo ""
        echo "INFO: Cleaning up ephemeral state (slot ''${${projectIdUpper}_EPHEMERAL_SLOT:-unknown})"
        rm -rf "${refEphRoot}"
        echo "OK: Ephemeral state cleaned"
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
    }:
    let
      runtimePath = if runtimePackages == [ ] then "" else pkgs.lib.makeBinPath runtimePackages;
      pathBlock = if runtimePath != "" then ''export PATH="${runtimePath}:$PATH"'' else "";
    in
    pkgs.writeShellScript "ephemeral-${name}" ''
      set -euo pipefail

      export ORIGINAL_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

      # Compatibility aliases:
      # - NIXFIED_ENV: alias for the configured slot variable (default: NIX_ENV).
      if [ -z "''${${slotVar}:-}" ] && [ -n "''${NIXFIED_ENV:-}" ]; then
        export ${slotVar}="''${NIXFIED_ENV}"
      fi

      # Slot acquisition: respect pre-set slot var (e.g. from test-isolation)
      if [ -n "''${${slotVar}:-}" ]; then
        export ${projectIdUpper}_EPHEMERAL_SLOT="''${${slotVar}}"
        export ${projectIdUpper}_SLOT_LOCK_FD=""
        echo "INFO: Using pre-set slot: ''${${slotVar}} (no lock - caller managed)"
      else
        eval "$(${acquireSlotLock})"
      fi

      export ${envVar}="''${${envVar}:-test}"

      export RUN_ID="$(${id.resolveId} "''${RUN_ID:-}")"

      export ${projectIdUpper}_EPHEMERAL_ROOT=$(${mkEphemeralRoot})
      export ${projectIdUpper}_EPHEMERAL=1

      echo ""
      echo "INFO: Ephemeral execution mode"
      echo "INFO: Root: ${refEphRoot}"
      echo "INFO: Slot: ${refEphSlot} (${slotVar}=''${${slotVar}}, ${envVar}=''${${envVar}})"
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
            echo "INFO: Installing dependencies"
            ${depsScript}
            echo "OK: Dependencies installed"
          ''
        else
          ""
      }

      ${pathBlock}

      # Load .env from original location (secrets shouldn't be copied)
      if [ -f "$ORIGINAL_ROOT/.env" ]; then
        while IFS='=' read -r key value || [ -n "$key" ]; do
          case "$key" in
            \#*|"") continue ;;
          esac
          value=$(echo "$value" | sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")
          if [ -z "''${!key:-}" ]; then
            export "$key=$value"
          fi
        done < "$ORIGINAL_ROOT/.env"
      fi

      echo ""
      echo "INFO: Starting ${name}"
      echo ""

      ${script}
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
    lockDir
    lockPrefix
    ;
}
