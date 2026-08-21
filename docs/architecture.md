# Architecture

Dependencies point inward from composition and adapters to typed domain/kernel contracts.

| Owner | Responsibility | Must not own |
| --- | --- | --- |
| IDs / Values | checked identities, schema descriptors, canonical typed values, 8 MiB object bound | execution or IO |
| Capabilities | `ReadCapabilityContract` and intent/evidence binding | State outcomes or retries |
| Program | typed Operation authoring, sole private lowering draft, checked v2 State/Match graph, `State`, `PureState`, `ReadState`, `Never` | registries, IO, scheduling |
| Runtime | immutable assembly, Program association, sole fold, typed execution/progression | persisted wire or physical storage |
| Journal | exact frame encoding and complete-history qualification | domain interpretation or persistence IO |
| Store / run index ports | object-safe complete load and atomic append; separate mechanical current-head enumeration | Program, State, capability, reducer, config semantics, or run-status derivation |
| Config repository port | immutable named revisions, exact import/load/delete, and complete listing | config-wire parsing, Program semantics, merging, defaults, or revocation |
| Domains | reusable deterministic Portfolio/EVM semantics and public value contracts | Runtime, Store, provider handles |
| Live adapters | bounded provider ingress and direct typed callback registration | domain planning or State registration |
| Application | injected and live composition; typed config/run/discovery use cases; exhaustive entry-point planning | sockets, argv/HTTP, sessions, frame inspection, status derivation, secret administration |
| Binaries | bounded transport parsing, client-side RunId entropy where offered, one Application call, transport policy, and redacted rendering | composition, domain planning, environment resolution, execution lifecycle, or run semantics |

RuntimeAssemblyBuilder registers exact value codecs, Pure/Read State drivers, Match descriptors, and
Read callbacks. `finish` freezes one immutable assembly. Program association pre-resolves every
implementation and callback; the fold performs no public registry lookup and exposes no erased value
workflow.

The progression sequence is:

```text
caller -> Application -> Runtime -> Journal frame -> Store append
                                  -> Pure driver
                                  -> Read adapter -> provider
Store load -> Journal qualify -> Runtime fold -> RunView
```

Concrete storage backends may implement both `Store` and the separate `RunIndex`, but Runtime
receives only `dyn Store`. Config custody and run enumeration therefore cannot widen Runtime's
append-only storage authority. Config listing returns all retained revisions as one unpaginated
aggregate; each document remains bounded, but the collection has no count limit. Run enumeration
uses ascending `RunId` keyset pages and makes no cross-request snapshot claim.

PostgreSQL owns its raw private locator grammar, target equivalence, SQLx wiring, split-role
provisioner, loopback-only plaintext policy, ambient-input exclusion, and one full-persistence gate.
One backend and pool implement Store, RunIndex, and config custody. It uses stock SQLx directly.
Administrative database authority exists only in the CLI provisioning path and is never retained by
Application.

`ComposedRuntime` is the only live Portfolio assembly constructor. One opaque binding set supplies
typed EVM targets and provider handles in stable order; composition derives both adapter
registrations and public binding views from it. The same concrete backend is coerced to `Store`,
`RunIndex`, and config custody, so production use cases cannot observe different repositories.

CLI and REST render one typed Application use-case surface and install no user authentication or
authorization layer. Binaries own bounded transport parsing/rendering and transport policy only. A
shared Application request contains only bounded, secret-free data and stable selectors for
pre-bound capabilities. It cannot introduce environment resolution, a filesystem or network
locator, secret custody, schema authority, or an unbounded durable effect.

The CLI owns argv, bounded file/stdin input, optional client-side RunId entropy, exit status, and
schema provisioning outside listener-held Application state. REST owns liveness, bounded HTTP
admission, and an unauthenticated Unix socket; it exposes neither schema nor secret administration.
REST requires a caller-owned path RunId and returns HTTP 200 for a durably failed run, while the CLI
may generate an identity and uses exit 1 for Runnable or Failed. These are named transport
asymmetries, not second use-case implementations.

The source-authoring sequence is separate from progression:

```text
domain Operation -> OperationExpansion -> one private flat draft -> immutable Program v2
                         |-> exact capability/State injection policy
```

`match_join` owns both ordinary value-selected topology and recovery selected by a Pure failure
classifier. `with_failure_handler` routes only the named typed State failures; it does not catch
compiler, Runtime, adapter, Store, cancellation, or panic failures. Runtime, Journal, and Store
never receive Operations, authoring setup, callbacks, or scope metadata.

Operation implementations compose children only through `OperationExpansion`, and capability
policies emit support States only through `InjectionWriter`. Direct trait callback calls bypass
kernel callback accounting and are forbidden in reviewed production code. This trusted-code rule
is not a security or authorization boundary; checked Program construction remains the persisted
graph boundary.

Pure work and byte-heavy validation run in immediately awaited pure blocking jobs. Connections,
transactions, Store mutation, and provider IO remain async and outside those jobs. Dropping an
operation is safe: no candidate exists yet, or the one in-flight append commits atomically and the
next complete reload resolves it.

The domain graph is one-way: Portfolio depends on EVM domain contracts; EVM depends on foundations
and Program; live EVM depends on EVM plus Runtime. Neither domain depends on Runtime, Store, live IO,
or Application.

## Material uncertainties

none
