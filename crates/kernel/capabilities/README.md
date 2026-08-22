# mfm-capabilities

The crate exposes `ReadCapabilityContract`, `EffectCapabilityContract`, `CapabilityError`, and its
Result alias. A Read names typed Intent/Evidence and binds evidence to the intent. An Effect names a
typed Command/Evidence pair and binds evidence to the exact `EffectId` and command.

It owns no State outcome, execution mode, attempt/retry, provider API, or mutation transport.
