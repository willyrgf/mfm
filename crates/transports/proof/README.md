# mfm-transports-proof

Deterministic typed proof runners and replay verifier.

This crate registers erased typed runners for certified proof state descriptors and exposes a
replay verifier for recorded proof side-effect evidence. Operation-aware conformance fixtures live
outside this transport crate so the transport does not depend on proof workflow planning code.
