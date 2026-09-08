# mfm-capabilities

ReadCapabilityContract names typed Intent, Evidence and OperationalError, and binds evidence to
the exact intent value reference and typed intent. EffectCapabilityContract names typed Command,
Evidence and OperationalError, and binds settlement to the exact EffectId and command. Program's
structured capability identity includes mode and these exact contracts.

`AdapterError<E>` separates a reviewed typed operational cause from `AdapterInvariantError`, a
local trusted implementation failure. Operational causes are ordinary bounded MfmValue data;
raw provider/signing diagnostics never cross this boundary. Debug and Display redact payloads.
State implementations own typed incident context, and Runtime owns recovery and error reporting.

The crate owns no State outcome, scheduler, provider API, transport, signer or mutation authority.
