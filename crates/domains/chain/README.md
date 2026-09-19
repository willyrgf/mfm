# mfm-chain

Shared deterministic chain-domain contracts. Native EVM contracts and live resources remain with
their existing owners; this crate has no Runtime, Store, network, signing or keystore dependency.

`balance` owns the shared request and evidence boundary. `BalanceRequest` preserves declaration
order, nonempty/64-source bounds, unique public IDs and one ledger. `DecimalScale` checks 0 through
30. Constructors and decoders share those checks. `BalanceTarget` retains an exact native Object;
its selected implementation must qualify asset, account and route semantics.

`BalanceRead` binds the semantic intent and original reference on every outcome, and the exact
observation point on successful evidence. `BalanceOutcome` retains Observed, Rejected, SafeFailure
and IntegrityBlocked without fabricating a scalar for unsuccessful evidence. This boundary alone
does not implement collection progression, native confirmation or caller continuation.

`BalanceRequest::scale_units` and `total_scaled` use private checked eighty-digit decimal arithmetic.
Zero remains canonical at every scale. Amounts below one target unit become zero; larger amounts
with discarded nonzero digits produce `BalanceCollectionFailure::InexactScale`. Scaling and the first
overflowing partial sum retain their operation and concrete size cause. These arithmetic helpers
do not establish confirmation or collection completion. Failure decoding rejects inconsistent facts.

`BalanceContext<K>` retains the exact caller and completed request prefix. `PreparedBalance<K>` checks
the active source and common observation point; `ObserveBalance<K>` produces a candidate or the
declared Permanent observation failure. Candidate decoding does not advance completion or scale an
amount. Native confirmation must compare anchors before calling `append_confirmed`; its outer result
keeps local invocation errors separate from inner arithmetic rejection. Successful append adds one
source. Completed-prefix decoding checks individual amounts but leaves aggregate overflow for
consolidation. Native confirmation and caller integration remain unfinished.

`CheckedAddition` supplies two checked `Unsigned256` operands. `CheckedAdd` returns their full-width
sum or its exact `AdditionOverflow` domain failure. The enclosing Runtime call retains the operands;
the failure does not duplicate them. Overflow is Permanent. Scalar mechanics live in Values.

`ConfigurationValue` gives lifecycle configuration the same full-width arithmetic with a distinct
nominal contract. Ledger, artifact, locator, transaction and observation envelopes retain exact
native Objects. These envelopes certify structural custody only: the selected native owner must
check the native type, protocol and ledger correspondence. `ContractExecutionConfig` retains checked
implementation selectors and a native options Object; it does not resolve implementations or handles.

`TransactionEffect<R>` associates each request with its applied result. Prepared commands retain
their typed request and native preparation; settlement evidence retains the native original and
checks common ledger agreement. Semantic binding checks Effect, command, implementation and original
references. Native implementations still own protocol authentication and interpretation.

Concrete `Deploy` interprets that settlement into checked `DeployedContract` or a Permanent
`DeploymentFailure`. Its success context starts effective configuration at the requested value and
retains the original deployment evidence. Rejected evidence cannot construct or decode a success
context. `TransactionResult` uses external JSON tagging so checked Object decoding receives native
canonical payloads directly, without Serde's internal-tag content buffering.

`CheckedAddConfigurationValue` consumes a deployed context, changes only its effective scalar and
returns the existing `AdditionOverflow` failure on overflow. `Configure` consumes that complete
context as its transaction request and constructs checked `ConfiguredContract` only for Applied
evidence. `ConfigurationApplied` is a nominal unit value (`null` on the wire), so deployment and
configuration keep distinct request/result capability identities while sharing transaction machinery.

`ContractExecutionConfig` v2 retains the caller's public `observation_route_ref`. `Observe` copies
it into `ReadContractValue` v2 alongside the deployed locator and configuration settlement's exact
observation point. Shared code never decodes native configuration to recover the expected route.
Observation/report consistency reconstructs the same route-bearing intent; missing route fields
are rejected on decoding.
`ContractValueEvidence` retains the native original and an outcome: Observed carries the point/value;
Rejected, SafeFailure and IntegrityBlocked produce corresponding Permanent Observe failures.
`ObservedConfiguration` checks the reconstructed semantic intent identity and exact anchor while
allowing a scalar mismatch. `Validate` turns that mismatch into its declared Permanent failure;
checked `ValidatedConfiguration` requires equality. `Report` moves the retained request, effective
value and three evidence records into one flat `ContractDeploymentReport` without re-encoding native
originals. Report construction and decoding share the contexts' borrowed consistency checks for
Applied evidence, ledgers, target/anchor identity and scalar equality. These checks establish internal
semantic consistency, not native authenticity or proof of execution history.

`ContractDeploymentLifecycle` and `ConfigureAndObserve` are ordinary typed Operation aliases over
these States. `LifecyclePlanning` borrows the original request from maintained contexts without
fabricating future inputs. `LifecycleDefaults` installs Stop with empty targets and resolves each
optional allowance with zero fallback. The configuration/observation child inherits its parent
policy; actual effective configuration continues to come from execution input.

This is an isolated Phase A checkpoint. Lifecycle/balance contracts and complete Program construction
are still in progress; the new crate does not enable shipping transaction execution.
