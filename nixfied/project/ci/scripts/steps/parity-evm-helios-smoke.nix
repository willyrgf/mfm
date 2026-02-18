{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-evm-helios-smoke.sh" ''
  eval "$($SLOT_INFO)"

  export HELIOS_NETWORK="local"
  export HELIOS_EXECUTION_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  export HELIOS_CONSENSUS_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  export HELIOS_READY_TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-120}"
  HELIOS_SERVICE_LOG=$(artifact_path "parity-evm-helios-service.log")
  fixture_start_service helios test "''${HELIOS_FIXTURE_TIMEOUT_SECS:-120}" 1 "$HELIOS_SERVICE_LOG"

  LOGFILE=$(artifact_path "parity-evm-helios-smoke.log")
  curl -fsS \
    -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
    "http://127.0.0.1:$HELIOSRPC_PORT" \
    | tee "$LOGFILE" \
    | jq -e '.result | strings' >/dev/null
''
