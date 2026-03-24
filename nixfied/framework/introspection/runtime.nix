{
  pkgs,
  assetsDir,
}:
pkgs.writeShellScript "nixfied-introspection-runtime" ''
    set -euo pipefail

    ASSETS_DIR=${pkgs.lib.escapeShellArg (builtins.toString assetsDir)}

    fail() {
      printf 'ERROR: %s\n' "$1" >&2
      exit 2
    }

    print_help() {
      cat <<'EOF'
  Usage: nix run .#introspect -- [query] [--json] [--kind <kind>] [--why <target>] [--reverse <target>] [--resolution-only|--execution|--closure]

  Examples:
    nix run .#introspect -- check
    nix run .#introspect -- app:check --json
    nix run .#introspect -- task:task.check
    nix run .#introspect -- workflow:workflow.ci.full
    nix run .#introspect -- service:postgres
    nix run .#introspect -- service-set:default
    nix run .#introspect -- check --why package:nix-checks
    nix run .#introspect -- reverse package:nix-checks
  EOF
    }

    is_valid_kind() {
      case "''${1:-}" in
        app|package|service|service-set|task|workflow)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
    }

    emit_query_lines() {
      local raw="$1"
      local source_kind="$2"
      local selector="$3"
      printf 'INFO: query.raw=%s\n' "$raw"
      printf 'INFO: query.source_kind=%s\n' "$source_kind"
      printf 'INFO: query.selector=%s\n' "$selector"
    }

    emit_diagnostics_lines() {
      cat "$ASSETS_DIR/diagnostics/human.txt"
    }

    join_labels() {
      local file="$1"
      local labels=""
      local line=""

      while IFS= read -r line || [ -n "$line" ]; do
        [ -n "$line" ] || continue
        if [ -n "$labels" ]; then
          labels="$labels, $line"
        else
          labels="$line"
        fi
      done < "$file"

      printf '%s' "$labels"
    }

    lookup_file_path() {
      local group="$1"
      local scope="$2"
      local token="$3"
      local extension="$4"

      case "$scope" in
        any)
          printf '%s/resolution/%s/any/%s.%s' "$ASSETS_DIR" "$group" "$token" "$extension"
          ;;
        *)
          printf '%s/resolution/%s/by-kind/%s/%s.%s' "$ASSETS_DIR" "$group" "$scope" "$token" "$extension"
          ;;
      esac
    }

    resolve_query_entry() {
      local token="$1"
      local kind_filter="$2"
      local prefix=""
      local entry_file=""
      local ambiguous_file=""
      local scope="any"
      local node_id=""
      local source_kind=""
      local selector=""

      prefix="''${token%%:*}"
      if [ "$prefix" != "$token" ] && is_valid_kind "$prefix"; then
        if [ -n "$kind_filter" ] && [ "$prefix" != "$kind_filter" ]; then
          fail "query '$token' does not match --kind $kind_filter"
        fi

        entry_file="$ASSETS_DIR/resolution/explicit/$token.tsv"
        if [ ! -f "$entry_file" ]; then
          fail "unable to resolve '$token'"
        fi
        IFS=$'\t' read -r node_id source_kind selector < "$entry_file"
        printf '%s\t%s\t%s' "$node_id" "$source_kind" "$selector"
        return 0
      fi

      if [ -n "$kind_filter" ]; then
        scope="$kind_filter"
      fi

      ambiguous_file="$(lookup_file_path "ambiguous" "$scope" "$token" "txt")"
      if [ -f "$ambiguous_file" ]; then
        fail "ambiguous query '$token' matched: $(join_labels "$ambiguous_file")"
      fi

      entry_file="$(lookup_file_path "bare" "$scope" "$token" "tsv")"
      if [ ! -f "$entry_file" ]; then
        fail "unable to resolve '$token'"
      fi

      IFS=$'\t' read -r node_id source_kind selector < "$entry_file"
      printf '%s\t%s\t%s' "$node_id" "$source_kind" "$selector"
    }

    emit_query_json() {
      local node_id="$4"
      cat "$ASSETS_DIR/views/query/json/$VIEW_MODE/$node_id.json"
    }

    emit_why_json() {
      local node_id="$4"
      local target_node_id="$8"

      if [ "$VIEW_MODE" = "resolution" ] || [ "$VIEW_MODE" = "execution" ]; then
        emit_query_json "" "" "" "$node_id"
        return 0
      fi

      cat "$ASSETS_DIR/views/why/json/$VIEW_MODE/$node_id/$target_node_id.json"
    }

    emit_reverse_json() {
      local target_node_id="$4"

      cat "$ASSETS_DIR/views/reverse/json/$VIEW_MODE/$target_node_id.json"
    }

    emit_query_human() {
      local raw="$1"
      local source_kind="$2"
      local selector="$3"
      local node_id="$4"

      emit_query_lines "$raw" "$source_kind" "$selector"
      cat "$ASSETS_DIR/views/query/human/$VIEW_MODE/$node_id.txt"
      emit_diagnostics_lines
    }

    emit_why_human() {
      local raw="$1"
      local source_kind="$2"
      local selector="$3"
      local node_id="$4"
      local target_node_id="$5"

      if [ "$VIEW_MODE" = "resolution" ] || [ "$VIEW_MODE" = "execution" ]; then
        emit_query_human "$raw" "$source_kind" "$selector" "$node_id"
        return 0
      fi

      emit_query_lines "$raw" "$source_kind" "$selector"
      cat "$ASSETS_DIR/views/query/human/$VIEW_MODE/$node_id.txt"
      cat "$ASSETS_DIR/views/why/human/$VIEW_MODE/$node_id/$target_node_id.txt"
      emit_diagnostics_lines
    }

    emit_reverse_human() {
      local target_node_id="$1"

      cat "$ASSETS_DIR/views/reverse/human/$VIEW_MODE/$target_node_id.txt"
      emit_diagnostics_lines
    }

    JSON_OUTPUT=0
    KIND_FILTER=""
    WHY_TARGET=""
    REVERSE_TARGET=""
    VIEW_MODE="default"
    POSITIONALS=()
    VIEW_FLAG_COUNT=0

    while [ "$#" -gt 0 ]; do
      case "$1" in
        -h|--help)
          print_help
          exit 0
          ;;
        --json)
          JSON_OUTPUT=1
          shift
          ;;
        --kind)
          shift
          if [ "$#" -lt 1 ]; then
            fail "--kind requires a value"
          fi
          KIND_FILTER="$1"
          shift
          ;;
        --why)
          shift
          if [ "$#" -lt 1 ]; then
            fail "--why requires a value"
          fi
          WHY_TARGET="$1"
          shift
          ;;
        --reverse)
          shift
          if [ "$#" -lt 1 ]; then
            fail "--reverse requires a value"
          fi
          REVERSE_TARGET="$1"
          shift
          ;;
        --resolution-only)
          VIEW_MODE="resolution"
          VIEW_FLAG_COUNT=$((VIEW_FLAG_COUNT + 1))
          shift
          ;;
        --execution)
          VIEW_MODE="execution"
          VIEW_FLAG_COUNT=$((VIEW_FLAG_COUNT + 1))
          shift
          ;;
        --closure)
          VIEW_MODE="closure"
          VIEW_FLAG_COUNT=$((VIEW_FLAG_COUNT + 1))
          shift
          ;;
        *)
          POSITIONALS+=("$1")
          shift
          ;;
      esac
    done

    if [ -n "$KIND_FILTER" ] && ! is_valid_kind "$KIND_FILTER"; then
      fail "--kind must be one of: app, package, service, service-set, task, workflow"
    fi

    if [ "$VIEW_FLAG_COUNT" -gt 1 ]; then
      fail "only one of --resolution-only, --execution, or --closure may be used"
    fi

    if [ -n "$WHY_TARGET" ] && [ -n "$REVERSE_TARGET" ]; then
      fail "--why and --reverse cannot be used together"
    fi

    if [ "''${#POSITIONALS[@]}" -gt 0 ] && [ "''${POSITIONALS[0]}" = "reverse" ]; then
      if [ -n "$REVERSE_TARGET" ]; then
        fail "reverse target was provided twice"
      fi
      if [ "''${#POSITIONALS[@]}" -ne 2 ]; then
        fail "reverse mode requires exactly one target"
      fi
      REVERSE_TARGET="''${POSITIONALS[1]}"
      POSITIONALS=()
    fi

    if [ "''${#POSITIONALS[@]}" -gt 1 ]; then
      fail "expected at most one query token"
    fi

    QUERY="''${POSITIONALS[0]:-}"

    if [ -n "$WHY_TARGET" ] && [ -z "$QUERY" ]; then
      fail "--why requires a query token"
    fi

    if [ -n "$REVERSE_TARGET" ] && [ -n "$QUERY" ]; then
      fail "reverse mode does not accept a separate query token"
    fi

    if [ -z "$QUERY" ] && [ -z "$REVERSE_TARGET" ]; then
      fail "expected a query token or reverse target"
    fi

    if [ -n "$REVERSE_TARGET" ]; then
      TARGET_ENTRY_TSV="$(resolve_query_entry "$REVERSE_TARGET" "")"
      IFS=$'\t' read -r TARGET_NODE_ID TARGET_SOURCE_KIND TARGET_SELECTOR <<< "$TARGET_ENTRY_TSV"

      if [ "$JSON_OUTPUT" -eq 1 ]; then
        emit_reverse_json "$REVERSE_TARGET" "$TARGET_SOURCE_KIND" "$TARGET_SELECTOR" "$TARGET_NODE_ID"
      else
        emit_reverse_human "$TARGET_NODE_ID"
      fi
      exit 0
    fi

    ENTRY_TSV="$(resolve_query_entry "$QUERY" "$KIND_FILTER")"
    IFS=$'\t' read -r NODE_ID SOURCE_KIND SELECTOR <<< "$ENTRY_TSV"

    if [ -n "$WHY_TARGET" ]; then
      TARGET_ENTRY_TSV="$(resolve_query_entry "$WHY_TARGET" "")"
      IFS=$'\t' read -r TARGET_NODE_ID TARGET_SOURCE_KIND TARGET_SELECTOR <<< "$TARGET_ENTRY_TSV"
      if [ "$JSON_OUTPUT" -eq 1 ]; then
        emit_why_json "$QUERY" "$SOURCE_KIND" "$SELECTOR" "$NODE_ID" "$WHY_TARGET" "$TARGET_SOURCE_KIND" "$TARGET_SELECTOR" "$TARGET_NODE_ID"
      else
        emit_why_human "$QUERY" "$SOURCE_KIND" "$SELECTOR" "$NODE_ID" "$TARGET_NODE_ID"
      fi
      exit 0
    fi

    if [ "$JSON_OUTPUT" -eq 1 ]; then
      emit_query_json "$QUERY" "$SOURCE_KIND" "$SELECTOR" "$NODE_ID"
    else
      emit_query_human "$QUERY" "$SOURCE_KIND" "$SELECTOR" "$NODE_ID"
    fi
''
