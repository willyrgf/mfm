# mfm-replay

Callback-free projections and the sole portable export format over the `mfm-store` structured
history fold.

The crate verifies recorded history through a reader and projects:

- canonical replay summary;
- fixed-head transition trace pages;
- fixed-head access audit pages;
- exact terminal operation outcome; and
- portable structured export encode/decode/offline verification.

It owns the only portable export document, top-level digest rules, and offline validator. It owns
no writer, callback, scheduler, process registry, provider, transport, signer, wallet authority, or
alternate reducer. Exact reproduction returns a frozen unavailable result when no candidate is
supplied; it never uses live fallback behavior. Offline verification uses only bundle bytes and an
explicit trust snapshot against the store's read-only fold entry.

The current export media type is
`application/vnd.mfm.structured-run-export.v1+json`. Old framed/pre-structured bytes are rejected.
