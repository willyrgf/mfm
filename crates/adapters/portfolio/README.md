# mfm-adapters-portfolio

Portfolio adapter runners.

This crate binds certified portfolio state descriptors to executable typed runners. It receives
store-verified input evidence, artifact-read capabilities, and EVM/Bitcoin read capabilities from
runtime/app assembly, executes state-classified portfolio read intent or pure behavior, and stages
typed fact/artifact evidence through those supplied boundaries.

Replay and resume authority remains with the certified spec and typed run stream. This crate does
not create workflow topology, own live JSON-RPC transport config, or store event envelopes.
