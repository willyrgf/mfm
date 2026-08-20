# mfm-app

Portfolio-only facade over one `ComposedRuntime`. The sole composer accepts the same backend as both
`Store` and `RunIndex`, plus one opaque checked `BoundCapabilitySet`; it builds the immutable Runtime
assembly, planning targets, and public binding views together. Callers cannot pair a Runtime, run
index, and discovery list from unrelated sources.

Bindings are empty or strictly sorted and unique by `(chain_id, endpoint_id)`, with at most 256 EVM
routes. Multiple named endpoints may bind one chain. Each public `PublicBindingView` is derived from
the exact `EvmPhysicalTarget` used to register the provider handle; it accepts no locator or second
caller-authored view.

`start_portfolio` plans checked Program/C0 from selector, process-local config, and a bounded subset
of the composed targets, then calls Runtime with the caller's explicit RunId. `resume` and `read`
return Runtime RunView directly.

`start_portfolio` rejects more than 64 target descriptors before cloning, planning, or Runtime
entry. It moves only bounded, secret-free planning inputs into an immediately awaited blocking
task; Runtime execution remains asynchronous.

`ComposedRuntime::compose` is the one trusted live composition: it registers the thirteen Portfolio
and EVM State implementations the snapshot Program declares, binds the three EVM Read callbacks for
every route, and freezes the assembly. `register_portfolio_states` is public only for adapterless
composition tests that prove association rejects a Program before Store IO.

Invalid selector is `InvalidRequest`; trusted planner/composition failure is `Internal`; typed
Runtime failures pass through `ApplicationError::Runtime`. Application owns no session, frame fold,
status projection, provider handle, configuration service, or transaction submission.
