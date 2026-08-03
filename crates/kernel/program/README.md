# mfm-program

Typed authoring and process callback contracts for declaration-ordered structured programs.

The public structured builder exposes `State`, exhaustive `Match`, bounded collect-all `FanOut`,
typed child calls, lexical values, explicit/default failure handling, and nominal lane/operation
outcomes. Builders preserve declaration order and perform no callback or IO. Child calls are
authoring sugar removed by certification expansion.

`Never` is the sealed uninhabited failure sentinel and deliberately has no value schema. State
callbacks receive typed canonical input/committed observation views and return proposed typed
outcomes plus fact proposals. A proposal gains authority only after store append.

This crate owns no serialized-program trust, store, Runtime, transport, or live capability.
