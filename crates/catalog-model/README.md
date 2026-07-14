# mfm-catalog-model

Typed, exact identities used to address values in the MFM configuration catalog.

This crate contains no storage or setup-import behavior. `CatalogRef<T>` carries the expected
semantic value type through the request type while its JSON representation contains only the
catalog name and content digest.
