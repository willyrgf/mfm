# mfm-app

`Application` is a fixed-tenant facade constructed by trusted embedding. Public calls accept no
credential, policy, principal, or tenant override. It owns transport-shaped requests and redacted
outputs while Program, Store, Runtime, and adapters own their respective contracts.

Trusted composition opens Store, publishes explicit typed Portfolio and EVM configuration, and
passes both resolved heads to `Application::new`. App remains non-generic: it selects the head by
entry point and passes it back to Store admission. It has no configuration parser, writer, generic
fallback, or credential-bearing configuration endpoint.
