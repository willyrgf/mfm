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

Generic start parses certified bundle JSON as untrusted transport data, verifies the spec and
certificate through `mfm-certify`, verifies config and seed inputs against the certified spec, and
hands typed launch material to runtime middleware. The runtime stages the referenced artifacts and
admits their evidence in the same prepared start commit that appends `RunStarted`. Domain start
routes may accept domain inputs, but they build typed drafts and certify them before runtime start.
Resume and replay reload stored spec/certificate artifacts, verify them against the production
registry, compare them to `RunStarted`, and rebuild stream evidence before constructing runtime or
replay authority.

Public-output JSON is an output/cache surface. `mfm-app` renders it only through
`PublicOutputReadAuthority`, which is minted after stored certified spec/certificate artifacts are
verified and the public-output projection is rebuilt from the authoritative typed run stream.
Rendered JSON cannot authorize resume, replay, or another render.
