# mfm-catalog

`mfm-catalog` owns bounded, atomic custody for named opaque canonical config bytes through
`ConfigCatalog`.

Config names are mutable locators and canonical config digests are content identities. The config
catalog returns its complete fixed-capacity set in ascending bytewise name order.

The crate deliberately has no config schema, Program, Runtime, Journal parser, provider, or IO
locator dependency. `MemoryCatalog` is the hermetic custody implementation; concrete durable
adapters implement the same port.
