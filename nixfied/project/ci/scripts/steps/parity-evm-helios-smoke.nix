{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-evm-helios-smoke.sh" ''
  source <($SLOT_INFO)

  export HELIOS_NETWORK="local"
  export HELIOS_EXECUTION_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  export HELIOS_CONSENSUS_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  export HELIOS_READY_TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-120}"
  HELIOS_SERVICE_LOG=$(artifact_path "parity-evm-helios-service.log")
  fixture_start_service helios test "''${HELIOS_FIXTURE_TIMEOUT_SECS:-120}" 1 "$HELIOS_SERVICE_LOG"

  LOGFILE=$(artifact_path "parity-evm-helios-smoke.log")
  RESPONSE_FILE=$(artifact_path "parity-evm-helios-smoke.response.json")
  set +e
  HELIOS_RPC_URL="http://127.0.0.1:$HELIOSRPC_PORT" \
  HELIOS_RPC_RESPONSE_FILE="$RESPONSE_FILE" \
    log_capture "$LOGFILE" -- bash -c '
      curl -fsS \
        -H "content-type: application/json" \
        --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_chainId\",\"params\":[]}" \
        "$HELIOS_RPC_URL" \
        | tee "$HELIOS_RPC_RESPONSE_FILE"
    '
  rc=$?
  set -e
  if [ "$rc" -ne 0 ]; then
    exit "$rc"
  fi
  jq -e '.result | strings' "$RESPONSE_FILE" >/dev/null
''
