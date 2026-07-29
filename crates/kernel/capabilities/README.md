# mfm-capabilities

Typed kernel crate for sealed effect descriptors, capability descriptors, roles, and
effect-checked capability sets.

`SafeFailureClassifierDescriptor` embeds the optional complete diagnostic `SchemaIdentity`
selected by one admitted capability binding. Its strict canonical decoder and verifier derive that
identity's schema id and invoke the shared structural shape validator over bounded canonical
diagnostic bytes. Append/load/replay can therefore enforce the exact classifier tuple and typed
diagnostic without invoking provider, adapter, capability, or state callbacks.

`docs/design.md` is the normative typed-core authority contract.
This crate is framework-owned and must remain domain-free.
