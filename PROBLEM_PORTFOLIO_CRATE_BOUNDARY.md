# Problem: Portfolio Crate Boundary

## Context

The current `crates/adapters/portfolio` crate binds certified portfolio states to runtime runners. It materializes typed inputs, registers executable state runners, calls EVM and Bitcoin capabilities, validates runtime availability, verifies returned evidence, and maps external capability errors into portfolio read errors.

That placement matches the current architecture taxonomy: states declare authority, transports implement protocol IO, and adapters bind state-owned intent to runtime capabilities.

The concern is that the portfolio adapter can start looking like it defines portfolio behavior rather than merely executing portfolio read intent. That makes the boundary less obvious, especially as multi-network support grows.

## Desired Boundary

Portfolio domain semantics should stay in portfolio model, config, and state crates:

- what a portfolio is
- wallet, symbol, network, and balance-reader validation
- observation and snapshot shapes
- valuation behavior
- rules for deriving domain read intent from configured portfolio data

The portfolio adapter should stay a runtime binding layer:

- register certified portfolio state descriptors as runners
- load retained artifacts and typed inputs
- execute already-classified portfolio read intent through capability providers
- verify capability evidence
- map runtime and capability failures into state-facing errors

Transport/client crates should stay protocol-specific and reusable:

- Ethereum JSON-RPC behavior
- Bitcoin Core JSON-RPC behavior
- source routing and endpoint redaction
- provider-specific limitations such as best-available Bitcoin UTXO scans

## Proposed Direction

Make the boundary more obvious by splitting or renaming around read intent:

1. Move more read classification into `crates/states/portfolio`.
   The state layer should decide the semantic read intent, such as native EVM balance, ERC-20 balance, native Bitcoin balance, or unsupported protocol position. The adapter should receive an already-classified request rather than re-deriving meaning from portfolio config.

2. Keep `crates/adapters/portfolio` focused on runner registration and capability execution.
   The adapter should translate classified portfolio read intents into EVM/BTC capability calls and evidence checks. It should not grow new portfolio rules.

3. Consider a clearer crate/module name if the current name keeps causing confusion.
   Options include `crates/adapters/portfolio-read` or an internal module split such as:
   - `runner_registration`
   - `read_backend`
   - `capability_mapping`

4. Keep protocol implementation out of the portfolio adapter.
   Bitcoin Core, EVM JSON-RPC, and future indexer/client logic should remain in transport or protocol adapter crates. Portfolio should depend on capability contracts, not concrete protocol behavior.

## Why This Matters

As portfolio support expands, the risk is that `crates/adapters/portfolio` becomes a mixed domain/runtime crate. That would make it harder to tell whether a change belongs in portfolio semantics, runtime binding, or live protocol implementation.

A sharper split should make future work easier to review:

- model/config/state changes define portfolio behavior
- adapter changes bind portfolio behavior to runtime capabilities
- transport/client changes implement external protocol behavior

The practical rule: if a change explains what portfolio data means, it probably belongs outside the adapter. If a change explains how a certified portfolio read is executed against available capabilities, it belongs in the adapter.
