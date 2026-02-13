# Shared service observability helpers
{
  pkgs,
  slots,
  processRegistry,
}:

let
  mkLogScript =
    service:
    pkgs.writeShellScript "${service}-log" ''
      set -euo pipefail
      SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
      eval "$SLOT_INFO_OUT"
      exec ${processRegistry.serviceLogs} --service ${service} --slot "$SLOT" --env "$ENV" "$@"
    '';

  mkEventsScript =
    service:
    pkgs.writeShellScript "${service}-events" ''
      set -euo pipefail
      SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
      eval "$SLOT_INFO_OUT"
      exec ${processRegistry.serviceEvents} --service ${service} --slot "$SLOT" --env "$ENV" "$@"
    '';

  mkStatusMergeBlock =
    {
      service,
      defaultLogPathExpr,
    }:
    ''
      LOCAL_RUNNING="$RUNNING"
      REGISTRY_FOUND="0"
      REGISTRY_RUNNING="false"
      REGISTRY_STATE="unknown"
      OWNER_RUN_ID=""
      OWNER_SCOPE=""
      EPHEMERAL_ROOT=""
      WAIT_REASON=""
      LOG_PATH=""
      SLOT_OWNER=""
      REGISTRY_SCOPE="global"

      REG_OUT="$(${processRegistry.serviceStatus} --service ${service} --slot "$SLOT" --env "$ENV" 2>/dev/null || true)"
      if [ -n "$REG_OUT" ]; then
        eval "$REG_OUT"
      fi

      if [ "$RUNNING" != "true" ] && [ "$REGISTRY_RUNNING" = "true" ]; then
        RUNNING=true
      fi

      SCOPE="none"
      if [ "$LOCAL_RUNNING" = "true" ]; then
        SCOPE="local"
      elif [ "$REGISTRY_RUNNING" = "true" ]; then
        SCOPE="global"
      fi

      EFFECTIVE_LOG_PATH=${defaultLogPathExpr}
      if [ -n "$LOG_PATH" ]; then
        EFFECTIVE_LOG_PATH="$LOG_PATH"
      fi
    '';

  mkEmitServiceEventFunction = service: ''
    emit_service_event() {
      local event_type="$1"
      local state="$2"
      shift 2 || true
      ${processRegistry.emitEvent} \
        --event-type "$event_type" \
        --service ${service} \
        --state "$state" \
        --slot "$SLOT" \
        --env "$ENV" \
        "$@" >/dev/null 2>&1 || true
    }
  '';
in
{
  inherit
    mkLogScript
    mkEventsScript
    mkStatusMergeBlock
    mkEmitServiceEventFunction
    ;
}
