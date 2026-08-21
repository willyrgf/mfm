# mfm-catalog

`mfm-catalog` owns two mechanical persistence ports. `ConfigCatalog` provides bounded, atomic
custody for named opaque canonical config bytes. `RunIndex` enumerates current run heads without
loading frames or interpreting run state.

Config names are mutable locators. Canonical config digests and run head digests are content
identities. The config catalog returns its complete fixed-capacity set in ascending bytewise name
order. The run index uses bounded ascending keyset pages with the last returned `RunId` as its
exclusive continuation; pages are not snapshots across requests.

The crate deliberately has no config schema, Program, Runtime, Journal parser, provider, or IO
locator dependency. `MemoryCatalog` is the hermetic custody implementation; concrete durable
adapters implement the same ports.
