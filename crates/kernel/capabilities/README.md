# mfm-capabilities

The crate exposes only `ReadCapabilityContract`, `CapabilityError`, and its Result alias. A contract
names typed Intent/Evidence, a stable contract ID, and one deterministic evidence-binding check.

It owns no State outcome, mode, attempt/retry, mutation/effect, call correlation, or provider API.
