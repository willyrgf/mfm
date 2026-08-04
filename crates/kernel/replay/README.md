# mfm-replay

Callback-free projections and the sole portable export format over the `mfm-store` structured
history fold.

The crate verifies recorded history through a reader and projects:

- canonical replay summary;
- fixed-head transition trace pages;
- fixed-head access audit pages;
- exact terminal operation outcome; and
- portable structured export encode/decode/offline verification.

It owns the only portable export frame stream, chain/seal digest rules, and offline validator. It owns
no writer, callback, scheduler, process registry, provider, transport, signer, wallet authority, or
alternate reducer. Recorded replay is verification-only and never compares against live state.
Offline verification uses only bundle bytes and an explicit trust snapshot against the store's
read-only fold entry.

The current export media type is
`application/vnd.mfm.structured-run-export-stream.v2`. Each newline-delimited frame is bounded,
canonical, and linked to its predecessor; the terminal seal binds the exact closure and stream
size. Legacy monolithic JSON objects are rejected without a compatibility decoder.
