# Commands

## Keystore CLI EIP-1559 sign/send

```bash
mfm_cli --output-format json keystore tx-sign \
  --by-label my-key \
  --to 0x1111111111111111111111111111111111111111 \
  --value-wei 1000000000000000 \
  --chain-id 31337 \
  --nonce 0 \
  --max-fee-per-gas 2000000000 \
  --max-priority-fee-per-gas 1000000000 \
  --gas-limit 21000 \
  --out /tmp/signed.tx
```

```bash
mfm_cli --output-format json keystore tx-send-raw \
  --rpc-url http://127.0.0.1:8545 \
  --in /tmp/signed.tx
```

## Validation

```bash
cargo test -p mfm --features parity-tests --test parity_keystore_reth_tx_send --no-run
nix run .#check
nix run .#ci -- --parity --summary
```
