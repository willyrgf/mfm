# mfm-app

`Application` is a fixed-tenant facade constructed by trusted embedding. Public calls accept no
credential, policy, principal, or tenant override. It owns transport-shaped requests and redacted
outputs while Program, Store, Runtime, and adapters own their respective contracts.

Trusted composition consumes each Store opening into explicit mutation, read, configuration, and
audit ports, publishes typed Portfolio and EVM configuration, and passes the required ports and
resolved heads to `Application::new`. An entry handled by Runtime uses a head from that Runtime's
exact independent Store opening; an App-only entry uses App's configuration/mutation opening. App
remains non-generic: it selects the head by entry point and passes it back to Store admission. It
constructs no journal records or frames and has no configuration parser, generic fallback, or
credential-bearing configuration endpoint.
