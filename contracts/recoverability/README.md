# Recoverability contract index

[`v3`](v3/README.md) is the sole current production recoverability registry
and conformance corpus. Production code and tests load only
`v3/annex.json` and `v3/corpus.json`.

No superseded registry or artifact tree is retained. Git history is the
archive; current readers fail closed on any non-current contract identity.
