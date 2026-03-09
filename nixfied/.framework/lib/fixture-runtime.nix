{ }:

{
  fixtureRuntime = ''
    _service_token() {
      local service="$1"
      echo "$service" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_'
    }

    _service_hook_name() {
      local service="$1"
      local op="$2"
      local token
      token=$(_service_token "$service")
      echo "SVC_''${token}_''${op}"
    }

    # fixture_start_service SERVICE [profile] [timeout] [interval] [logfile] [keep_running]
    # - start service lifecycle from hook contract and register cleanup unless keep_running=1.
    #
    # Lifecycle policy invariants:
    # - start hooks may return before the service is externally ready.
    # - READY/HEALTH hooks must be safe to poll and deterministic when not ready.
    # - profile-specific readiness hooks (for example *_READY_TEST) take precedence when present.
    # - fixtures must poll READY/HEALTH with timeout loops after start (never one-shot checks).
    # - failure diagnostics must not introduce secondary errors (for example tailing missing logs).
    fixture_start_service() {
      local service="$1"
      local profile="''${2:-default}"
      local timeout="''${3:-60}"
      local interval="''${4:-1}"
      local logfile="''${5:-}"
      local keep_running_raw="''${6:-0}"
      local keep_running="0"

      if [ -z "$service" ]; then
        echo "usage: fixture_start_service <service> [profile] [timeout] [interval] [logfile] [keep_running]" >&2
        return 1
      fi

      case "$keep_running_raw" in
        1|true|TRUE|yes|YES)
          keep_running="1"
          ;;
        0|false|FALSE|no|NO|"")
          keep_running="0"
          ;;
        *)
          log_error "fixture_start_service keep_running must be 0/1/true/false (got '$keep_running_raw')"
          return 1
          ;;
      esac
      require_positive_int "timeout" "$timeout" || return 1
      require_positive_number "interval" "$interval" || return 1

      local start_hook=""
      local init_hook=""
      local check_hook=""
      local stop_hook=""
      local health_hook=""
      local ready_hook=""
      local profile_ready_hook=""
      local wait_hook=""
      local status_hook=""
      local start_cmd=""
      local op=""
      local hook=""
      local profile_token=""
      local -a start_ops=()

      case "$profile" in
        test)
          start_ops=(FULL_START_TEST FULL_START START)
          ;;
        *)
          start_ops=(FULL_START START)
          ;;
      esac

      for op in "''${start_ops[@]}"; do
        hook=$(_service_hook_name "$service" "$op")
        if has_hook "$hook"; then
          start_hook="$hook"
          break
        fi
      done

      if [ -z "$start_hook" ]; then
        log_error "fixture_start_service could not resolve start hook service=$service profile=$profile"
        return 1
      fi

      stop_hook=$(_service_hook_name "$service" "STOP")
      health_hook=$(_service_hook_name "$service" "HEALTH")
      ready_hook=$(_service_hook_name "$service" "READY")
      status_hook=$(_service_hook_name "$service" "STATUS")
      init_hook=$(_service_hook_name "$service" "INIT")
      check_hook=$(_service_hook_name "$service" "CHECK_CONFIG")
      start_cmd="''${!start_hook}"

      if [ -n "$profile" ] && [ "$profile" != "default" ]; then
        profile_token=$(echo "$profile" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_')
        profile_ready_hook=$(_service_hook_name "$service" "READY_''${profile_token}")
      fi

      if has_hook "$init_hook"; then
        run_hook "$init_hook"
      fi
      if has_hook "$check_hook"; then
        run_hook "$check_hook"
      fi

      local pid=""
      if [ -n "$logfile" ]; then
        if ! NIXFIED_START_SERVICE_MANAGED_CLEANUP=1 start_service_into pid "$service" --log "$logfile" -- "$start_cmd"; then
          log_error "fixture service start failed service=$service hook=$start_hook"
          return 1
        fi
      else
        if ! NIXFIED_START_SERVICE_MANAGED_CLEANUP=1 start_service_into pid "$service" -- "$start_cmd"; then
          log_error "fixture service start failed service=$service hook=$start_hook"
          return 1
        fi
      fi
      if [ -z "$pid" ]; then
        log_error "fixture service start returned empty pid service=$service hook=$start_hook"
        return 1
      fi

      # Clean wrapper process and module-native process state unless persistence is requested.
      if [ "$keep_running" = "1" ]; then
        log_info "fixture service keep_running enabled service=$service pid=$pid"
      else
        with_cleanup stop_service "$pid" "$service"
        if has_hook "$stop_hook"; then
          with_cleanup run_hook "$stop_hook"
        fi
      fi

      if [ -n "$profile_ready_hook" ] && has_hook "$profile_ready_hook"; then
        wait_hook="$profile_ready_hook"
      elif has_hook "$ready_hook"; then
        wait_hook="$ready_hook"
      elif has_hook "$health_hook"; then
        wait_hook="$health_hook"
      fi

      if [ -n "$wait_hook" ]; then
        local start_ts
        start_ts=$(date +%s)

        while true; do
          if run_hook "$wait_hook" >/dev/null 2>&1; then
            break
          fi

          # If the wrapper process died, fail fast unless STATUS indicates service is up.
          if ! kill -0 "$pid" 2>/dev/null; then
            local status_ok=0
            if has_hook "$status_hook"; then
              if run_hook "$status_hook" >/dev/null 2>&1; then
                status_ok=1
              fi
            fi

            if [ "$status_ok" -ne 1 ]; then
              log_error "fixture service start exited early service=$service pid=$pid hook=$start_hook"
              if [ -n "$logfile" ]; then
                print_log_tail "$logfile" 200
              fi
              if has_hook "$stop_hook"; then
                run_hook "$stop_hook" >/dev/null 2>&1 || true
              fi
              stop_service "$pid" "$service" >/dev/null 2>&1 || true
              return 1
            fi
          fi

          if [ $(( $(date +%s) - start_ts )) -ge "$timeout" ]; then
            log_error "fixture readiness check failed service=$service hook=$wait_hook timeout=''${timeout}s"
            if [ -n "$logfile" ]; then
              print_log_tail "$logfile" 200
            fi
            if has_hook "$stop_hook"; then
              run_hook "$stop_hook" >/dev/null 2>&1 || true
            fi
            stop_service "$pid" "$service" >/dev/null 2>&1 || true
            return 1
          fi

          sleep "$interval"
        done
      fi

      log_ok "fixture service ready service=$service profile=$profile"
      return 0
    }
  '';
}
