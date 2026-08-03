# mfm-replay

Callback-free projections over the sole `mfm-store` structured-history fold.

The crate verifies recorded history through a reader and projects:

- canonical replay summary;
- fixed-head transition trace pages;
- fixed-head access audit pages;
- exact terminal operation outcome; and
- current portable structured export support.

It owns no writer, callback, scheduler, process registry, provider, transport, signer, wallet
authority, or alternate reducer. Exact reproduction and current comparison return a frozen
unavailable result when no candidate is supplied; they never use live fallback behavior.

The current export media type is
`application/vnd.mfm.structured-run-export.v1+json`. Old framed/pre-structured bytes are rejected.
