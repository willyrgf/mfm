{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-keystore-reth-tx-sign-send.sh" ''
  source <($SLOT_INFO)

  export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  # Keep CLI JSON stderr contracts deterministic in this parity step even when
  # CI/session logging env vars are enabled.
  unset MFM_LOG
  unset LOG_LEVEL
  unset RUST_LOG
  unset MFM_LOG_FORMAT
  unset LOG_FORMAT
  unset MFM_LOG_SPAN_EVENTS
  unset LOG_SPAN_EVENTS

  LOGFILE=$(artifact_path "parity-keystore-reth-tx-sign-send.log")
  log_capture "$LOGFILE" -- cargo nextest run -p mfm --features parity-tests --test parity_keystore_reth_tx_send
''
