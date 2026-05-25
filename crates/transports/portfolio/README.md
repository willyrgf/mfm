# mfm-transports-portfolio

Typed portfolio workflow runners.

This crate binds certified portfolio state descriptors to executable typed runners. It receives
store-verified input evidence and typed artifact handles from runtime/app assembly, executes the
portfolio read or pure behavior, and stages typed fact/artifact evidence through the supplied
boundaries.

Replay and resume authority remains with the certified spec and typed run stream. This crate does
not create workflow topology or store event envelopes.
