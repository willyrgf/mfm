# mfm-app

Typed application assembly for certified MFM runs.

`mfm-app` wires certified typed runtime pieces only:

- certified runtime specs
- typed runner registries
- typed run event stores
- typed artifact stores
- typed start/resume/replay dispatch
- typed public-output read authority and rendering

It does not depend on old dynamic machine or SDK crates, dynamic DAG planning, context snapshots,
or generic IO providers.

Public-output JSON is an output/cache surface. `mfm-app` renders it only through
`PublicOutputReadAuthority`, which is minted after stored certified spec/certificate artifacts are
verified and the public-output projection is rebuilt from the authoritative typed run stream.
Rendered JSON cannot authorize resume, replay, or another render.
