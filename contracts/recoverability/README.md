# Recoverability contract index

[`v1`](v1/README.md) is the sole current production recoverability registry
and conformance corpus. Production code and tests load only
`v1/annex.json` and `v1/corpus.json`.

No superseded registry or artifact tree is retained. Git history is the
archive; current readers fail closed on any non-current contract identity.
