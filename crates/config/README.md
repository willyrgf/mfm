# mfm-config

`mfm-config` owns domain-generic custody for immutable named configuration revisions through
`ConfigRepository`.

Each `(ConfigName, ConfigDigest)` identifies one immutable, bounded opaque canonical document. A
name has exactly one current revision, and importing a retained historical digest atomically makes
it current again. Listing returns every retained revision in ascending name/digest order with a
current marker; there is deliberately no collection-count limit or delete operation.

The crate has no config schema, Program, Runtime, Journal parser, provider, or IO locator dependency.
`MemoryConfigRepository` is the hermetic implementation; durable transports implement the same
small port.
