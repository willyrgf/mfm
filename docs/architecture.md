# Architecture

Dependencies point inward from composition and adapters to typed domain/kernel contracts.

| Owner | Responsibility | Must not own |
| --- | --- | --- |
| IDs / Values | checked identities, schema descriptors, canonical typed values, 8 MiB object bound | execution or IO |
| Capabilities | `ReadCapabilityContract` and intent/evidence binding | State outcomes or retries |
| Program | typed Operation authoring, sole private lowering draft, checked v2 State/Match graph, `State`, `PureState`, `ReadState`, `Never` | registries, IO, scheduling |
| Runtime | immutable assembly, Program association, sole fold, typed execution/progression | persisted wire or physical storage |
| Journal | exact frame encoding and complete-history qualification | domain interpretation or persistence IO |
| Store | object-safe complete load and atomic append | Program, State, capability, or reducer semantics |
| Domains | reusable deterministic Portfolio/EVM semantics and public value contracts | Runtime, Store, provider handles |
| Live adapters | bounded provider ingress and direct typed callback registration | domain planning or State registration |
| Application | trusted Portfolio assembly composition, Portfolio plan/start, and direct Runtime resume/read facade | sessions, frame inspection, status derivation |
| Binaries | CLI configuration parsing, one live Application composition, and one redacted view rendering; REST unavailable diagnostic | execution lifecycle, run semantics, or a bound REST listener |

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
