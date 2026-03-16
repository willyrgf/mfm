# mfm-collectors-rpc-control

Typed helpers over the `IoCall` surface for `namespace = "rpc.control"`.

This crate does not perform live IO. It defines typed request/response models and an `IoProvider`
client for managed RPC control-plane operations such as routed EVM calls and source
prepare/setup/probe/rank actions. Managed request identity includes explicit `control_scope`
(`shared` by default) and explicit `network_id` for canonical managed calls.

Docs: [`../../../docs/redesign.md`](../../../docs/redesign.md)
