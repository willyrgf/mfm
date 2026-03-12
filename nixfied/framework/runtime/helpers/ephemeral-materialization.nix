{
  pkgs,
  project,
  projectRoot ? null,
  loggingPrelude ? "",
}:

let
  lib = pkgs.lib;
  ephemeralCfg = project.ephemeral or { };
  copyMode = ephemeralCfg.copyMode or "nix-source";
  includeUntracked = ephemeralCfg.includeUntracked or false;
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
  maxCopyBytes = ephemeralCfg.maxCopyBytes or 0;
  minFreeBytesAfterCopy = ephemeralCfg.minFreeBytesAfterCopy or 0;
  envFileMode = ephemeralCfg.envFileMode or "disabled";
  envFilePath = ephemeralCfg.envFilePath or ".env";
  envLoader = import ./env-loader.nix {
    inherit pkgs project;
    loggingPrelude = loggingPrelude;
  };

  matchesPattern =
    name: pattern:
    if pattern == "*" then
      true
    else if lib.hasPrefix "*" pattern && lib.hasSuffix "*" pattern then
      let
        middle = lib.removePrefix "*" (lib.removeSuffix "*" pattern);
      in
      middle != "" && lib.hasInfix middle name
    else if lib.hasPrefix "*" pattern then
      lib.hasSuffix (lib.removePrefix "*" pattern) name
    else if lib.hasSuffix "*" pattern then
      lib.hasPrefix (lib.removeSuffix "*" pattern) name
    else
      name == pattern;

  shouldExclude = baseName: builtins.any (pattern: matchesPattern baseName pattern) excludePatterns;

  nixSourceRoot =
    if projectRoot == null then
      null
    else
      builtins.path {
        path = projectRoot;
        name = "nixfied-ephemeral-source";
        filter =
          path: _type:
          let
            pathStr = toString path;
          in
          if pathStr == toString projectRoot then true else !(shouldExclude (baseNameOf pathStr));
      };

  rsyncExcludes = lib.concatMapStringsSep " " (pat: "--exclude='${pat}'") excludePatterns;
in
{
  inherit
    copyMode
    includeUntracked
    envFileMode
    envFilePath
    ;

  mkSourceCopy = pkgs.writeShellScript "mk-source-copy" ''
    ${loggingPrelude}

    set -euo pipefail

    SOURCE_DIR="$1"
    DEST_DIR="$2"
    COPY_MODE=${lib.escapeShellArg copyMode}
    INCLUDE_UNTRACKED=${if includeUntracked then "1" else "0"}
    MAX_COPY_BYTES=${toString maxCopyBytes}
    MIN_FREE_BYTES_AFTER_COPY=${toString minFreeBytesAfterCopy}
    NIX_SOURCE_ROOT=${lib.escapeShellArg (if nixSourceRoot == null then "" else toString nixSourceRoot)}

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
        printf 'ERROR: ephemeral copy budget exceeded: bytes_required=%s max_copy_bytes=%s\n' "$copy_bytes" "$MAX_COPY_BYTES"
        exit 1
      fi

      if [ "$free_bytes" -le "$copy_bytes" ]; then
        printf 'ERROR: ephemeral copy budget exceeded: bytes_required=%s bytes_free=%s\n' "$copy_bytes" "$free_bytes"
        exit 1
      fi

      if [ "$MIN_FREE_BYTES_AFTER_COPY" -gt 0 ] && [ "$remaining_bytes" -lt "$MIN_FREE_BYTES_AFTER_COPY" ]; then
        printf 'ERROR: ephemeral copy budget exceeded: bytes_remaining=%s min_free_after_copy=%s\n' "$remaining_bytes" "$MIN_FREE_BYTES_AFTER_COPY"
        exit 1
      fi
    }

    copy_tree() {
      local source_root="$1"
      local dry_run_stats copy_bytes
      dry_run_stats="$(${pkgs.rsync}/bin/rsync -an --stats "$source_root/" "$DEST_DIR/")"
      copy_bytes="$(extract_total_file_size_bytes "$dry_run_stats")"
      enforce_copy_budget "$copy_bytes"
      ${pkgs.rsync}/bin/rsync -a "$source_root/" "$DEST_DIR/"
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
        if [ "$INCLUDE_UNTRACKED" = "1" ]; then
          ${pkgs.git}/bin/git ls-files -z --cached --others --exclude-standard
        else
          ${pkgs.git}/bin/git ls-files -z --cached
        fi
      ) > "$manifest"

      log_info "Using git-files copy mode include_untracked=$INCLUDE_UNTRACKED"

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
      nix-source)
        if [ -z "$NIX_SOURCE_ROOT" ] || [ ! -d "$NIX_SOURCE_ROOT" ]; then
          log_error "nix-source copy mode requires a compiled project source path"
          exit 2
        fi
        log_info "Using nix-source copy mode source=$NIX_SOURCE_ROOT"
        copy_tree "$NIX_SOURCE_ROOT"
        ;;
      git-files)
        if ${pkgs.git}/bin/git -C "$SOURCE_DIR" rev-parse --show-toplevel >/dev/null 2>&1; then
          git_copy
        else
          if [ "$INCLUDE_UNTRACKED" = "1" ]; then
            log_warn "git-files copy mode unavailable outside a git worktree; falling back to static-excludes"
            static_copy
          else
            log_error "git-files copy mode with include_untracked=0 requires a git worktree"
            exit 2
          fi
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

  loadHostEnv = pkgs.writeShellScript "ephemeral-load-host-env" ''
    ${loggingPrelude}

    set -euo pipefail

    ORIGINAL_ROOT="$1"

    case ${lib.escapeShellArg envFileMode} in
      disabled)
        log_info "Skipping host env file import mode=disabled"
        ;;
      original-root)
        log_info "Loading host env file mode=original-root path=$ORIGINAL_ROOT/${envFilePath}"
        source ${envLoader.loadEnvFile} "$ORIGINAL_ROOT/${envFilePath}"
        ;;
      *)
        log_error "Unsupported ephemeral env file mode: ${envFileMode}"
        exit 2
        ;;
    esac
  '';
}
