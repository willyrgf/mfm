# mfm-keystore

`Keystore` is created inside a dedicated owner thread and remains neither `Send` nor `Sync`.
`KeystoreOwner` communicates over one bounded channel, imports checked zeroizing secp256k1 scalars,
and returns key-bound `KeystoreSigner` handles. Duplicate imports converge on the same public
key-instance content ref; at most 64 distinct instances are retained.

Secret material never appears in persisted, diagnostic, or public surfaces. Dropping the last
sender ends the owner loop. Explicit async shutdown consumes the unique controller and joins the OS
thread without blocking a Tokio worker.
