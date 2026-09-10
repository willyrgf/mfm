# mfm-diagnostics

Shared secret-free causal evidence for IO owners. The crate owns closed diagnostic vocabulary,
checked HTTP status and SQLSTATE values, source/fact ordering, omission attribution, and bounded
serialization. It performs no IO and imports no client library, Program, Runtime, or Store.

`DiagnosticEvidence::new` checks the complete 8 KiB compact canonical budget, up to 32 sources,
eight facts per source, and 32 omissions. Construction orders facts by schema field order;
deserialization rejects noncanonical fact order. Response observations remain separate from the
outermost-first causal chain. Opaque sources may precede reviewed deeper sources. Withheld,
unavailable, and bounded evidence have distinct encodings; none promises raw diagnostic custody.

`DiagnosticEvidence::capture` traverses actual exposed source links at most 32 times. Concrete
owners supply reviewed layers and unlocated omissions; capture assigns locations only to retained
sources. It reserves bound markers before filling variable entries, then retains omissions within
the remaining budget. No arbitrary source formatting or suffix walk is performed. The complete
evidence implements `MfmValue` and `PersistedSchema`; its constituent fields own persisted schemas
without redundant independent value identities.

Live EVM uses this contract for request/body/parser errors and response observations. Other adapter
families and the Runtime execution/context cutover remain work in progress.

Focused verification: `nix develop -c cargo test -p mfm-diagnostics`.
