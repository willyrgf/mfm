{ pkgs }:
pkgs.writeText "mfm-ci-steps-mainnet-portfolio-snapshot-helios.sh" ''
  source <($SLOT_INFO)

  export HELIOS_NETWORK="mainnet"
  # Execution RPC must support `eth_getProof` for explicit block numbers (not just `latest`).
  export HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-https://eth.drpc.org}"

  # Prefer a stable, up-to-date consensus endpoint for CI (faster and less flaky than relying
  # on the default if it is temporarily unavailable).
  export HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-https://lodestar-mainnet.chainsafe.io}"

  # Mainnet Helios can take a while to sync; gate on eth_blockNumber.
  export HELIOS_READY_TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-900}"

  export MFM_ARTIFACT_ROOT="$CI_ARTIFACTS_DIR/mfm-mainnet-artifacts"
  mkdir -p "$MFM_ARTIFACT_ROOT"

  ADDRESS="0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"

  LOGFILE=$(artifact_path "mainnet-portfolio-snapshot.log")
  RAW_OUTFILE=$(artifact_path "mainnet-portfolio-snapshot.raw.out")
  OUTFILE=$(artifact_path "mainnet-portfolio-snapshot.json")
  OUTPUT_MODE_LOWER="$(printf '%s' "''${OUTPUT_MODE:-stdout}" | tr '[:upper:]' '[:lower:]')"

  run_snapshot_capture() {
    case "$OUTPUT_MODE_LOWER" in
      logs)
        nix run .#mfm::portfolio::snapshot -- "$ADDRESS" >"$RAW_OUTFILE" 2>"$LOGFILE"
        ;;
      stdout|both|"")
        nix run .#mfm::portfolio::snapshot -- "$ADDRESS" \
          > >(tee "$RAW_OUTFILE") \
          2> >(tee "$LOGFILE" >&2)
        ;;
      *)
        # Setup validates OUTPUT_MODE, but keep fallback tolerant.
        nix run .#mfm::portfolio::snapshot -- "$ADDRESS" >"$RAW_OUTFILE" 2>"$LOGFILE"
        ;;
    esac
  }

  set +e
  run_snapshot_capture
  rc=$?
  set -e

  if [ $rc -ne 0 ]; then
    echo "ERROR: mfm::portfolio::snapshot failed rc=$rc" >&2

    HELIOS_EVENTS_FILE=$(artifact_path "mainnet-helios-events.log")
    HELIOS_SERVICE_LOG_FILE=$(artifact_path "mainnet-helios-service-log.log")
    PROCESS_INSPECT_FILE=$(artifact_path "mainnet-process-inspect.log")

    if has_hook SVC_HELIOS_EVENTS; then
      run_hook SVC_HELIOS_EVENTS -- --limit 200 >"$HELIOS_EVENTS_FILE" 2>&1 || true
      echo "INFO: helios events (tail 200) path=$HELIOS_EVENTS_FILE" >&2
      tail -200 "$HELIOS_EVENTS_FILE" >&2 || true
    fi

    if has_hook SVC_HELIOS_LOG; then
      run_hook SVC_HELIOS_LOG -- --lines 200 >"$HELIOS_SERVICE_LOG_FILE" 2>&1 || true
      echo "INFO: helios service log (tail 200) path=$HELIOS_SERVICE_LOG_FILE" >&2
      tail -200 "$HELIOS_SERVICE_LOG_FILE" >&2 || true
    fi

    if [ -n "''${RUN_ID:-}" ]; then
      nix run .#process::inspect -- "$RUN_ID" >"$PROCESS_INSPECT_FILE" 2>&1 || true
      echo "INFO: process inspect path=$PROCESS_INSPECT_FILE run_id=$RUN_ID" >&2
      tail -200 "$PROCESS_INSPECT_FILE" >&2 || true
    fi

    tail -200 "$LOGFILE" >&2 || true
    if [ -s "$RAW_OUTFILE" ]; then
      echo "STDOUT:" >&2
      cat "$RAW_OUTFILE" >&2 || true
    fi
    exit $rc
  fi

  # The app contract is final JSON on stdout, but Rust/tracing notices may still appear
  # ahead of the payload in some environments. Keep only the JSON document.
  sed -n '/^{/,$p' "$RAW_OUTFILE" >"$OUTFILE"
  if ! jq -e . "$OUTFILE" >/dev/null; then
    echo "ERROR: snapshot output is not valid JSON" >&2
    echo "RAW STDOUT:" >&2
    cat "$RAW_OUTFILE" >&2 || true
    echo "STDERR LOG:" >&2
    tail -200 "$LOGFILE" >&2 || true
    exit 1
  fi

  jq -e '.status == "success"' "$OUTFILE" >/dev/null
  jq -e '.data.feature_id == "portfolio.snapshot"' "$OUTFILE" >/dev/null
  jq -e '.data.result.phase == "completed"' "$OUTFILE" >/dev/null
  jq -e '.data.result.snapshot | type == "object"' "$OUTFILE" >/dev/null
  jq -e '.data.result.snapshot.chain_id == .data.result.chain_id' "$OUTFILE" >/dev/null
  jq -e '.data.result.snapshot.block_number == .data.result.block_number' "$OUTFILE" >/dev/null

  ART_ID=$(jq -r '.data.result.snapshot_artifact_id // empty' "$OUTFILE")
  if [ -z "$ART_ID" ] || [ "$ART_ID" = "null" ]; then
    echo "ERROR: missing snapshot_artifact_id" >&2
    cat "$OUTFILE" >&2
    exit 1
  fi

  SNAPSHOT_FILE="$MFM_ARTIFACT_ROOT/''${ART_ID:0:2}/$ART_ID"
  if [ ! -f "$SNAPSHOT_FILE" ]; then
    echo "ERROR: snapshot artifact not found at $SNAPSHOT_FILE" >&2
    exit 1
  fi

  INLINE_BAL_WEI=$(jq -r '.data.result.snapshot.native.raw_u256_dec // empty' "$OUTFILE")
  BAL_WEI=$(jq -r '.native.raw_u256_dec // empty' "$SNAPSHOT_FILE")
  MIN_BAL_WEI="32000000000000000000"

  if [ -z "$INLINE_BAL_WEI" ] || [ "$INLINE_BAL_WEI" = "null" ] || ! echo "$INLINE_BAL_WEI" | grep -Eq '^[0-9]+$'; then
    echo "ERROR: invalid embedded ETH balance; got data.result.snapshot.native.raw_u256_dec=$INLINE_BAL_WEI" >&2
    cat "$OUTFILE" >&2
    exit 1
  fi

  if [ -z "$BAL_WEI" ] || [ "$BAL_WEI" = "null" ] || ! echo "$BAL_WEI" | grep -Eq '^[0-9]+$'; then
    echo "ERROR: invalid ETH balance; got native.raw_u256_dec=$BAL_WEI" >&2
    cat "$SNAPSHOT_FILE" >&2
    exit 1
  fi

  if [ "$INLINE_BAL_WEI" != "$BAL_WEI" ]; then
    echo "ERROR: embedded snapshot balance mismatch inline=$INLINE_BAL_WEI artifact=$BAL_WEI" >&2
    exit 1
  fi

  # Compare large decimal integers without relying on 64-bit shell arithmetic.
  if [ "''${#BAL_WEI}" -lt "''${#MIN_BAL_WEI}" ] || { [ "''${#BAL_WEI}" -eq "''${#MIN_BAL_WEI}" ] && [ "$BAL_WEI" \< "$MIN_BAL_WEI" ]; }; then
    echo "ERROR: expected at least 32 ETH (wei >= $MIN_BAL_WEI); got native.raw_u256_dec=$BAL_WEI" >&2
    cat "$SNAPSHOT_FILE" >&2
    exit 1
  fi

  echo "OK: mainnet snapshot ETH balance >= 32 ETH wei=$BAL_WEI artifact_id=$ART_ID"
''
