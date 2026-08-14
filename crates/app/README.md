# mfm-app

`Application` is a fixed-tenant facade constructed by trusted embedding. Public calls accept no
credential, policy, principal, or tenant override. It owns transport-shaped requests and redacted
outputs while Program, Store, Runtime, and adapters own their respective contracts.

Trusted composition supplies one catalog-wide `Runtime`, read/configuration/audit ports from the
same Store opening, typed Portfolio and EVM configuration, and the finite exact EVM binding sets.
`Application::new` rejects a missing, surplus, or mismatched route, binding, configuration head, or
Runtime registration. App remains non-generic: it decodes a bounded selector, asks the owning
domain planner for one `C0`/Program/source-ref product, qualifies it, derives the run id, and calls
Runtime admission. It constructs no Program declarations or journal frames and has no Store
fallback, configuration parser, or credential-bearing configuration endpoint.
