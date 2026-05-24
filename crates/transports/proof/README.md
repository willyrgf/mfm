# mfm-transports-proof

Deterministic typed proof runners and conformance fixture.

This crate registers erased typed runners for the certified proof state descriptors and verifies
the enabled proof implementation against the RFC conformance shape. It intentionally does not
provide a legacy `proof.*` live-IO namespace.
