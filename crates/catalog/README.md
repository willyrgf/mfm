# mfm-catalog

`mfm-catalog` owns two mechanical persistence ports. `ConfigCatalog` provides bounded, conditional
custody for named opaque canonical config bytes. `RunIndex` enumerates current run heads without
loading frames or interpreting run state.

Config names are mutable locators. Canonical config digests and run head digests are content
identities. Both indexes use bounded, resource-specific keyset cursors and ascending bytewise
identity order; pages are not snapshots across requests.

The crate deliberately has no config schema, Program, Runtime, Journal parser, provider, or IO
locator dependency. `MemoryCatalog` is the hermetic custody implementation; concrete durable
adapters implement the same ports.
