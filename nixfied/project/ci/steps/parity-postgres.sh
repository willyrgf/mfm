eval "$($SLOT_INFO)"
# Fixture `postgres` with profile `test` already waits on SVC_POSTGRES_READY_TEST.

export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

LOGFILE=$(artifact_path "parity-postgres.log")
log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_event_store_postgres_contract
