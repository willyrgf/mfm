# mfm-state-keystore

Reusable keystore administration and transaction-signing states for MFM workflows.

This crate owns reusable `State` implementations only. The states build typed
`local.keystore.*` requests and route them through `IoProvider`, then publish
non-secret reports into context and domain events so keystore-related ops can
remain thin planners.

Boundary rules:

- typed request and report DTOs live in `mfm-collectors-local-keystore`
- EVM transaction models, signing hashes, and raw transaction encoding live in
  `mfm-evm-core`
- live filesystem, prompt, password, keystore unlock, and signing behavior lives
  in `mfm-transports-local-keystore`
- encrypted keystore and Ethereum private-key signing primitives remain in
  `mfm_core`
- this crate must not deserialize private keys, mnemonics, passwords, or raw
  signed transaction output into persisted context, artifacts, reports, logs, or
  error details
