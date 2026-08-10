# mfm-replay

Callback-free projections and the sole portable export format over `mfm-store` qualified
history reduction.

The crate verifies recorded history through a reader and projects:

- canonical replay summary;
- fixed-head transition trace pages;
- fixed-head access audit pages;
- exact terminal operation outcome; and
- portable structured export encode/decode/offline verification.

It owns the only portable export frame stream, chain/seal digest rules, and offline validator. It owns
no writer, callback, scheduler, process registry, provider, transport, signer, wallet authority, or
alternate reducer. Recorded replay is verification-only and never compares against live state.
Replay summary, transition trace, and access audit use separate closed typed codecs. Transition
entries re-derive their assigned record hashes; audit entries bind status to their optional
observation and re-derive an included observation hash. A fixed-head audit never projects an
observation from a later suffix.
Offline verification uses only bundle bytes, the caller's expected complete-stream `ContentRef`, and
an explicit trust snapshot against the store's read-only qualification entry.

The current export media type is
`application/vnd.mfm.structured-run-export-stream.v4`. The complete stream is the one portable
identity: `encode()` returns the exact bytes and their `ContentRef` together, and a frame is an
internal typed record of that one codec with no identity of its own. The terminal seal binds the
exact closure and fixation, and deliberately restates no frame ordinal, chain digest, or byte count —
physical line order, the expected reference, and the canonical batch predecessor chains already own
those facts. Old bytes are rejected without a compatibility decoder.
