# mfm-config

`mfm-config` owns domain-generic custody for immutable named configuration revisions through
`ConfigRepository`.

`mfm-ids` owns the checked `ConfigName`; this crate owns the canonical-JSON-specific `ConfigDigest`.
Each `(ConfigName, ConfigDigest)` identifies one immutable, bounded opaque canonical document. A
name may retain multiple independent revisions. Import creates or compares one exact revision,
load and idempotent delete require its exact name/digest pair, and listing returns every revision in
ascending name/digest order. There is deliberately no mutable current pointer, collection-count
limit, or pagination.

The crate has no config schema, Program, Runtime, Journal parser, provider, or IO locator dependency.
`MemoryConfigRepository` is the hermetic implementation; durable transports implement the same
small port.
