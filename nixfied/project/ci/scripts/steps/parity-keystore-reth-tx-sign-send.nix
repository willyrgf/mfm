{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-keystore-reth-tx-sign-send.sh" ''
  source <($SLOT_INFO)

  export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

  LOGFILE=$(artifact_path "parity-keystore-reth-tx-sign-send.log")
  log_capture "$LOGFILE" -- cargo-nightly nextest run --cargo-profile ci -p mfm --features parity-tests --test parity_keystore_reth_tx_send
''
