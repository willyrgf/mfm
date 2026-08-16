# mfm-app

`Application` is a fixed-tenant facade constructed by trusted embedding. Public calls accept no
credential, policy, principal, or tenant override. It owns transport-shaped requests and redacted
outputs while Program, Store, Runtime, and adapters own their respective contracts.

Trusted composition supplies one catalog-wide `Runtime`, read/configuration/audit ports from the
same Store opening, typed Portfolio configuration, and the finite exact EVM balance Read bindings.
`Application::new` rejects a missing or mismatched binding, configuration head, or Runtime
registration. App remains non-generic: it decodes a bounded Portfolio selector, asks the domain
planner for one `C0`/Program/source-ref product, qualifies it, derives the run id, and calls Runtime
admission. It constructs no Program declarations or journal frames and has no Store fallback,
configuration parser, or credential-bearing configuration endpoint.
