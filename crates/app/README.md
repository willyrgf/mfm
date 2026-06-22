# mfm-app

Typed application assembly for certified MFM runs.

`mfm-app` wires entry-point operation launch and certified typed runtime pieces only:

- entry-point operation registries
- app-owned typed spec certification
- typed runner registries
- the production run store
- narrow artifact read providers over run-store evidence
- typed start/resume/replay dispatch
- typed public-output read authority and rendering

Domain runner behavior lives in adapter crates. For EVM contract lifecycles,
`mfm-app` only wires concrete process resources such as JSON-RPC clients,
artifact read providers, and keystore-backed signer providers into the adapter runner
factory.

It does not depend on old dynamic machine or SDK crates, dynamic DAG planning, context snapshots,
or generic IO providers.

Entry-point start resolves a registered public op name and version, normalizes authored config,
plans a typed draft, certifies it through `mfm-certify`, verifies config and seed inputs against the
certified spec, and hands typed launch material to runtime middleware. The runtime passes launch
artifact bytes in the prepared commit bundle that appends `RunAdmitted`. Resume and replay reload
stored spec/certificate artifacts, verify them against the
production registry, compare them to `RunAdmitted`, and rebuild stream evidence before constructing
runtime or replay authority.

Run status exposes manual-resolution requirements from certified policy only: evidence schema,
manual authorization verifier, signing scheme, certified operator authority id, allowed operator
public identities, and quorum. It does not expose signer runtime sources such as keystore paths,
environment variables, passwords, or provider configuration.

Public-output JSON is an output/cache surface. `mfm-app` renders it only through
`PublicOutputReadAuthority`, which is minted after stored certified spec/certificate artifacts are
verified and the public-output projection is rebuilt from the authoritative typed run stream.
Rendered JSON cannot authorize resume, replay, or another render.
