{ pkgs }:
''
  valid_log_level() {
    local value="$1"
    case "$value" in
      error|warn|info|debug|trace)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  valid_output_mode() {
    local value="$1"
    case "$value" in
      stdout|logs|both)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  jq_positional_args_json() {
    if [ "$#" -eq 0 ]; then
      ${pkgs.jq}/bin/jq -cn '$ARGS.positional'
    else
      ${pkgs.jq}/bin/jq -cn '$ARGS.positional' --args -- "$@"
    fi
  }

  call_with_array_args() {
    local array_name="$1"
    shift
    local fn_name="$1"
    shift
    local -n array_ref="$array_name"

    if [ "''${#array_ref[@]}" -gt 0 ]; then
      "$fn_name" "$@" "''${array_ref[@]}"
    else
      "$fn_name" "$@"
    fi
  }

  write_text_file_atomic() {
    local target="$1"
    local value="$2"
    local parent_dir
    local tmp

    if [ -z "$target" ]; then
      return 0
    fi

    parent_dir="$(dirname "$target")" || {
      echo "ERROR: unable to determine parent directory for '$target'" >&2
      return 1
    }
    mkdir -p "$parent_dir" || {
      echo "ERROR: unable to create directory '$parent_dir'" >&2
      return 1
    }
    tmp="$(mktemp "$target.tmp.XXXXXX")" || {
      echo "ERROR: unable to create temp file for '$target'" >&2
      return 1
    }
    if ! printf '%s\n' "$value" > "$tmp"; then
      rm -f "$tmp"
      echo "ERROR: failed to write temp file for '$target'" >&2
      return 1
    fi
    if ! mv "$tmp" "$target"; then
      rm -f "$tmp"
      echo "ERROR: failed to move temp file into '$target'" >&2
      return 1
    fi
  }
''
