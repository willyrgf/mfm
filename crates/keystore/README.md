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

Executing sign failures return SigningError directly through the existing owner reply. SignFailed
retains operation `sign` and the distinct request_send, reply_receive, key_lookup or
sign_prehash_recoverable stage. Channel and primitive failures retain their supplied message;
missing slots have only the local missing_key fact. The pinned signing primitive exposes an opaque
signature error with no child source. Checked compact-signature conversion errors pass unchanged.

Diagnostics never include the Command, signing request, secret scalar or panic payload. A closed
reply does not establish why the owner stopped. Import, startup, shutdown and general checked
cryptographic constructors retain their existing error contracts.
