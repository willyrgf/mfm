eval "$($SLOT_INFO)"

export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

LOGFILE=$(artifact_path "parity-keystore-reth-tx-sign-send.log")
log_capture "$LOGFILE" -- cargo nextest run -p mfm --features parity-tests --test parity_keystore_reth_tx_send
