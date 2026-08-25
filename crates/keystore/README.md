# mfm-keystore

A private append-only key container is created inside a dedicated owner thread and remains neither
`Send` nor `Sync`. Public callers use `KeystoreOwner`, which communicates over one bounded channel,
imports checked zeroizing secp256k1 scalars, and returns key- and immutable-purpose-bound
`KeystoreSigner` handles. Purpose stays on the handle and never crosses the owner channel. A private
stable slot identifies each retained key without reserializing it for lookup. Duplicate same-key
imports converge on the same retained key instance even when their handles have different purposes;
at most 64 distinct keys are retained.

Secret material never appears in persisted, diagnostic, or public surfaces. Dropping the last
sender ends the owner loop. Explicit async shutdown consumes the unique controller and joins the OS
thread without blocking a Tokio worker.
