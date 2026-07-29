# Recoverability contract index

[`v2`](v2/README.md) is the sole current production recoverability registry
and conformance corpus. Production code and tests load only
`v2/annex.json` and `v2/corpus.json`.

[`v1`](v1/README.md) is a byte-identical archive of the completed v1 cutover.
Its annex, corpus, and historical README remain frozen for audit and explicit
hostile legacy-input rejection; they are not a compatibility registry or
supported reader.
