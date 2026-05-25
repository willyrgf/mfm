# mfm-app

Typed application assembly for certified MFM runs.

`mfm-app` wires certified typed runtime pieces only:

- certified runtime specs
- typed runner registries
- typed run event stores
- typed artifact stores
- typed start/resume/replay dispatch
- typed public-output rendering

It does not depend on old dynamic machine or SDK crates, dynamic DAG planning, context snapshots,
or generic IO providers.
