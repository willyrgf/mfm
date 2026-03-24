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

  json_escape_string() {
    local value="$1"
    value="''${value//\\/\\\\}"
    value="''${value//\"/\\\"}"
    value="''${value//$'\n'/\\n}"
    value="''${value//$'\r'/\\r}"
    value="''${value//$'\t'/\\t}"
    value="''${value//$'\f'/\\f}"
    value="''${value//$'\b'/\\b}"
    printf '%s' "$value"
  }

  json_quote_string() {
    printf '"%s"' "$(json_escape_string "$1")"
  }

  json_string_or_null() {
    local value="$1"
    if [ -z "$value" ]; then
      printf '%s' "null"
      return 0
    fi
    json_quote_string "$value"
  }

  json_number_or_null() {
    local value="$1"
    case "$value" in
      ""|null)
        printf '%s' "null"
        ;;
      -*[!0-9]*|*[!0-9]*)
        printf '%s' "null"
        ;;
      *)
        printf '%s' "$value"
        ;;
    esac
  }

  json_bool_or_null() {
    local value="$1"
    case "$value" in
      true|false)
        printf '%s' "$value"
        ;;
      *)
        printf '%s' "null"
        ;;
    esac
  }

  json_object_from_named_env_values() {
    local env_name=""
    local first=1

    printf '{'
    while IFS= read -r env_name; do
      [ -n "$env_name" ] || continue
      if [ -z "''${!env_name+x}" ]; then
        continue
      fi
      if [ "$first" -eq 0 ]; then
        printf ','
      fi
      printf '%s:%s' "$(json_quote_string "$env_name")" "$(json_quote_string "''${!env_name}")"
      first=0
    done
    printf '}'
  }

  positional_args_json() {
    local first=1

    printf '['
    while [ "$#" -gt 0 ]; do
      if [ "$first" -eq 0 ]; then
        printf ','
      fi
      json_quote_string "$1"
      first=0
      shift
    done
    printf ']'
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
