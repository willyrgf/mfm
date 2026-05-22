# mfm-app

Typed application assembly for certified MFM runs.

`mfm-app` wires certified typed runtime pieces only:

- certified runtime specs
- typed runner registries
- typed run event stores
- typed artifact stores
- typed start/resume/replay dispatch
- typed public-output rendering

It does not depend on `mfm-machine`, `mfm-sdk`, dynamic DAG planning, context snapshots, or generic
IO providers. The old dynamic application bridge is isolated in `mfm-app-legacy` while CLI and REST
surfaces are being rewritten.
