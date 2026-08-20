# mfm-transport-security

This crate owns the one checked TLS-root authority shared by live PostgreSQL and EVM transports.
The exhaustive choices are compiled WebPKI anchors or an absolute PEM file whose exact bytes match
a caller-supplied `content:sha256-v1` digest.

PEM files are opened without following the leaf, must be regular files, are bounded at 256 KiB,
and are fully pinned before certificate parsing. A loaded value retains exactly one immutable
Rustls `RootCertStore`; custom PEM anchors never union with compiled roots.

The crate owns no endpoint URL, credential, environment lookup, protocol client, or retry policy.
