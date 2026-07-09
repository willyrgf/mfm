# mfm-adapters-portfolio

Portfolio adapter runners for **fact-backed, report-only** portfolio snapshots.

This crate binds certified portfolio state descriptors to executable typed runners. It receives
store-verified input evidence, artifact-read capabilities, and a Platform `FactIndexReadProvider`
from runtime/app assembly. `SelectHoldings` hydrates retained `FactResponse` artifacts via the
shared kit (`fact_response_artifact_requirement`, `hydrate_fact_response_json`), runs pure
network-coherent selection, and records fact-query evidence for replay.

There is **no** live portfolio transport factory, pin/observe runners, or chain crawl path.

Replay and resume authority remains with the certified spec and typed run stream. This crate does
not create workflow topology or own collector source IO.
