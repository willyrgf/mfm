# Runtime, Store, and Journal cutover manifest

Status: current absence authority for the typed runtime/journal/store cutover.

The supported-current roots are `Cargo.toml`, `Cargo.lock`, `README.md`, `AGENTS.md`, `flake.nix`,
`nixfied.nix`, `.github/`, `.config/`, `nix/`, `bin/`, `crates/`, `docs/`, and `scripts/`.
The immutable RFC, its selected implementation plan, and header-marked prior RFC/plan/handoff
archives are design records, not supported-current product claims. Tests may retain hostile old-wire
literals only when they cannot export or register them.

## Complete deletion ledger

This ledger imports the complete RFC Section 11 inventory. A transferred name survives only at its
named new owner: `ProposedStateOutcome` in Program, `StoreOpenError` in PostgreSQL, and
`EvmPhysicalTarget` in the EVM domain. All other entries below are absent from supported production
surfaces, identities, persistence, tasks, and current documentation.

```text
ProgramCatalog
ProgramCatalogBuilder
ProgramCatalogInner
application_catalog
snapshot_closure_document
validate_runtime_closure
PortfolioAdmissionPlan
ProgramIngress
ProgramRef wrapper
public ProgramDocument/wire DTO separate from checked Program
ValueAssociation
CapabilityAssociation
ReifyValue
public Program canonical_value helper
public/program-owned QualifiedValue
public QualifiedTypedValue
QualifiedTypedValue::erase
public try_downcast
catalog brand
ProgramError::InvalidValue
ProgramError::InvalidCatalog
PublicOutputDescriptor
StateInput marker trait/derive
OperationOutput marker trait/derive
PublicOutputs marker trait/derive
SchemaKind::StateInput/OperationOutput/PublicOutput
FieldSegment/FieldPath
CapabilityKind/CapabilityKindKind
CapabilityVersion/CapabilityVersionKind
DynamicOutcome
DynamicPreparationFailure
DynamicStateRegistration
DynamicPrepared
DynamicCommit
DynamicCall
DynamicResolution
AccessMode
ReadMode marker
EffectMode
EffectKind
EffectVersion
Effect kind/version ID types
AccessCapabilityContract
AccessEvidenceValue marker
mfm-capabilities ProposedStateOutcome export (sole type moves to mfm-program)
PureImplementation
ReadImplementation
AccessImplementation
EvmPureState/EvmAccessState/PortfolioPureState duplicate behavior traits
AccessResolution
AcceptedOutcomeAccess
AcceptedIntegrityAccess
UnresolvedAccess
UnresolvedClassification
ReadResolution
ReadObservation
ReadUnavailable unit error (replaced by ReadAdapterError)
BlockedIntegrityProjection
PreparedExecution
PreparedAccess
PreparedRead
CommittedCall
CommittedRead
PreparationRef
QualifiedRecordedEvidence
AccessIngressFuture
public `'static` BoxFuture<T> alias (only the private borrowing alias remains)
OpenedPreparationCommit
QualifiedAdapter
Runtime-owned PreparationError
Read attempt/replacement counter or relation
RunView Waiting state
HotValue
public/composite StateExecutionKey
separate Runtime/adapter call_id
RunSession
SuspendedRun
ParkedRun
ParkReason
PendingConclusion
AdmissionInput
standalone AdmissionFailure/AdmissionConflict lifecycle types
ResumeFailure
TerminalRun
RuntimeLimits
RuntimeError::Conflict
NoLongerSelected
SpawnStep
ResumeStep
RuntimeStep
PendingAppend
PendingWork
PendingKey
acknowledgement lease
completion/finalization cell
pending-owner limit or permit
resolver token
Runtime cancellation API/token/state
Runtime active-session/CPU/planning/ingress semaphores
max_active_sessions
max_cpu_jobs
max_planning_jobs
max_ingress_jobs
active_sessions
cpu_jobs
planning_jobs
ingress_jobs
Runtime semaphore acquire helpers and OwnedSemaphorePermit fields
Runtime timeout policy
Runtime detached completion task/finalizer
StoreBrand
StructuredStore
OpenedStructuredStore
StoreParts
QualifiedHistoryPort
HistoryReader
StoreAuditPort
QualifiedRun
RunReducer
RunSelection
RunAction
SelectedRun
PreparedAdmission
PreparedConclusion
SelectedConclusion
FactContinuation
SemanticStore
replay_terminality
retained_program
validate_prefix
AccessActionMode
AccessPreparationOutcome
AdmissionOutcome
SelectedConclusionOutcome
SelectedConclusionPreparationOutcome
StructuredStoreBackend
MemoryStructuredBackend
dyn-Store check_ready/readiness method
mfm-store ReadyError/StoreOpenError exports (StoreOpenError moves to mfm-storage-postgres)
mfm-storage-postgres DurabilityProfile
mfm-storage-postgres broad PostgresError/Result alias
public PostgresStore::from_pool
public PostgresStore::rotate_identity
public PostgresStore::check_ready/profile
public PostgresStore::migrate/install schema method
StoredRunBytes captured head sequence/digest
StoreWorkLimits
RawHistoryLoadLimit
BackendFuture/BackendResult/BackendError aliases
BackendAppendCommand
BackendAppendOutcome
AppendDisposition
AppendResult Existing/Stale variants
RunAppend
public RunPosition
Current/Exact load selector
run listing/pagination API
RawRunPrefix
RawFrameBytes
RawFactPublication
RawFactSnapshot
append request row
configuration request row
separate receipt row
AppendRequestId
IdempotencyConflict
AcknowledgementUnknown result
UnavailableBeforeSubmission
InvalidPhysicalCommand
AlreadyConcludedSame
expected_position field
append command digest or digest index
FailureValue
optional State failure contract
Program root_contract_ref v1
SequentialControlAddress and any replacement public StateAddress
declaration address, next_address, failure_next_address, and Match entry_address fields
declaration-address sorting/map, root inference, and cycle DFS
State terminal
State maximum_conclusion_bytes
State/Read total_attempt_bound
Execution fact_selection_required
Execution effect_domain
separate optional execution_binding field
AccessBindingV2 and mfm-access-binding@2 wire
public BindingDescriptor and mfm.execution-binding@1 wire
EvmBalanceBindings and any one-variant live-binding wrapper
EvmAdapterBinding
EvmLiveAssembly contribution wrapper
live-owned EvmPhysicalTarget definition (the sole type moves to mfm-evm)
RunAdmitted entry_point field
StatePrepared/state_prepared record
PreparationMode
State conclusion occurrence field
StateConcludedAccess/state_concluded_access record
State conclusion preparation_sequence field
MatchVariant payload_contract_ref/continuation_contract_ref fields
public/open RunFrame/RunRecord/RunAdmitted/StatePrepared/StateConcluded DTOs
public/open StateOutcome/ImmutableObject/ValueRef/RecordLogicalKey DTOs
open Journal DTO constructors plus validate choreography
mfm-facts
FactSelectionMode
NoPriorFacts
PriorRunFacts
FactSelection
FactProposalSet
FactHead
FactPublication
fact publication
fact proof
fact SQL
MfmConfig
ValidatedConfig
ProvenConfiguration
ConfigurationKey
ConfigurationPosition
ConfigurationRef
ConfigurationRevision
ConfigurationAppend
ConfigurationHeadProjection
ConfigurationStore
ResolvedConfiguration
ResolvedConfigurationHead
ConfigurationWriteSession
PreparedConfigurationAppend
SuspendedConfigurationAppend
ConfigurationAppendCommand
ConfigurationCommitOutcome
ConfigurationAppendDisposition
BackendConfigurationOutcome
RawConfigurationRevision
configuration publication/load API
configuration revision/head/request/receipt SQL
Program configuration contract
configuration envelope middleman
duplicated configuration schema ref
MAX_CONFIGURATION_REVISIONS
MAX_CONFIGURATION_REVISION_BYTES
MAX_CONFIGURATION_STREAM_BYTES
PortableRunBundle
PortableRun
ExportedRun
export_run
portable export/inspection API
portable decoder or semantic ingress
record_digest
separate semantic record hash/proof type
per-State or schema-recursive conclusion-size proof
Open/Replace/Consume reservation instruction
ReservationKey
run_reservations
reserved_bytes
reserved_frames
durable conclusion reservation/accounting
MAX_STATE_CONCLUSION_BYTES
old MAX_RUN_FRAME_BYTES constant
first-reference object closure/dictionary
MAX_RUN_OBJECTS
object_count
reserved_objects
object-slot reservation/accounting
StoreScopeId
StoreEpoch
TenantScopeId
StructuredStoreIdentity
active identity
identity rotation
database incarnation
global object table
per-run object membership
mfm-replay
separate replay reducer
App suspended map
App frame-derived status
AdmitRunRequest without explicit RunId
parse_admission_json/MAX_ADMISSION_BYTES
AdmitRunResponse
DriveResponse
App-owned RunStatus/PublicRunView
ReplayResponse
TraceResponse/TraceRecord
AccessAuditResponse/AccessAuditEntry
ExportedRun response DTO
mfm.evm/submit-transaction@1 entry point
EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID
EvmConfig
EvmTransactionTarget
EvmSubmission* public types
NonceReservationIntent/NonceReservationEvidence
BroadcastIntent/BroadcastEvidence
plan_submission
submission_closure_documents
wallet_nonce_effect_domain
public_signer_key_instance_ref
evm_live_adapter_implementation_ref
PostgresWalletNonce*/WalletNonceDomain* public exports
EvmProviderResponse::Broadcast/PossibleEntry variants
EvmAdapterError
EvmReadSubject::TransactionReceipt/FinalizedHead/CanonicalInclusionBlock variants
EvmReadValue::Receipt/FinalizedHead/CanonicalBlock variants
ReadCapabilityFamily::SubmissionStatus and EvmCapability<3> implementation/IDs
EVM submission request/progress/output/failure and plan/binding surface
EVM nonce-reservation/candidate/broadcast/status States and capabilities
WalletNonceAuthority and every nonce-domain/operation/activation type
mfm-storage-evm-postgres and nonce SQL
live-EVM signer/broadcast/nonce adapter integration
App EVM-submission dispatch and composition
SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md prior-target inventory
IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.log prior-target handoff record
EvmCapability<const KIND: u8>
EvmState<const FAMILY: u8, const STAGE: u8, K>
PortfolioState<const STAGE: u8>
```

## Scoped scanner policy

The executable scanner treats absence by category:

- exact paths cover deleted crates, modules, migration, docs, fixtures, and old authority files;
- Cargo manifests and the lockfile cover packages, normal/dev edges, and retired test features;
- production `src` roots cover public exports and registrations;
- production source, SQL, Nix, and workflow roots cover stable IDs, wire fields, SQL authority, and
  task filters; and
- supported-current Markdown covers stale ownership and product claims.

Hostile tests may name rejected old fields and tags. Archives named above may retain historical
claims. Neither exception permits a production export, registration, Cargo edge, SQL object, task,
or current-document claim. The scanner creates and removes one canary for path/package, public
export, stable wire, SQL, and current-document categories before accepting a clean tree.

## Machine-enforced rules

The scanner reads the tab-separated rules below; this manifest is the executable absence input,
not a parallel prose checklist. `path` rules are repository-relative. Regex scopes are `cargo`,
`production`, `schema`, `sql`, `task`, `docs`, or one exact repository-relative file.

```cutover-rules
ledger	.	ac0fce9687f1ad52354b3b32a1a7a53a1395d05047afb775e313e2d013ce56d6
inventory	.	74
ruleset	.	3a9ee98d8a928c12958b4d1d365c937e1e52cb06a9ce697191b048494c90afcd
path	.	SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md
path	.	IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.log
path	.	crates/kernel/facts
path	.	crates/kernel/replay
path	.	crates/storages/evm-postgres
path	.	crates/kernel/program/src/single_trust.rs
path	.	crates/kernel/journal/src/single_trust.rs
path	.	crates/kernel/runtime/src/single_trust.rs
path	.	crates/kernel/runtime/src/lifecycle.rs
path	.	crates/kernel/store/src/single_trust.rs
path	.	crates/kernel/store/src/backend.rs
path	.	crates/kernel/store/src/backend_conformance.rs
path	.	crates/storages/postgres/build.rs
path	.	crates/storages/postgres/migrations/0001_single_trust.sql
path	.	docs/btc-rpc-routing.md
path	.	docs/preflight/capability-binding-inventory.md
path	.	docs/preflight/capacity-envelope.md
path	.	docs/preflight/cumulative-context-abi.md
path	.	docs/evm-transactions.md
path	.	docs/contracts/evm-portfolio/evm-submission-destination-rejected.json
path	.	docs/contracts/evm-portfolio/evm-submission-failure.json
path	.	docs/contracts/evm-portfolio/evm-submission-nonce-lineage-diverged.json
path	.	docs/contracts/evm-portfolio/evm-submission-provider-unavailable.json
path	.	docs/contracts/evm-portfolio/evm-submission-reverted.json
path	.	docs/contracts/evm-portfolio/evm-submission-succeeded.json
regex	cargo	(?m)^[[:space:]]*name[[:space:]]*=[[:space:]]*['"]mfm-(facts|replay|storage-evm-postgres)['"]
regex	cargo	(?m)^[[:space:]]*['"]?mfm-(facts|replay|storage-evm-postgres)['"]?[[:space:]]*=|package[[:space:]]*=[[:space:]]*['"]mfm-(facts|replay|storage-evm-postgres)['"]|^[[:space:]]*\[(?:[^]\n]*\.)?(?:dev-|build-)?dependencies\.[[:space:]]*['"]?mfm-(facts|replay|storage-evm-postgres)['"]?[[:space:]]*\]
regex	cargo	['"]?(test-support|parity-tests)['"]?[[:space:]]*=
regex	production	pub[[:space:]]+(?:(struct|enum|trait|type|fn|mod)[[:space:]]+|use[[:space:]]+[^;]*\b)(ProgramCatalog|ProgramCatalogBuilder|ProgramCatalogInner|ProgramIngress|ProgramDocumentV[0-9]+|ValueAssociation|CapabilityAssociation|ReifyValue|QualifiedValue|QualifiedTypedValue|PublicOutputDescriptor|StateInput|OperationOutput|PublicOutputs|FieldSegment|FieldPath|CapabilityKind|CapabilityKindKind|CapabilityVersion|CapabilityVersionKind|DynamicOutcome|DynamicPreparationFailure|DynamicStateRegistration|DynamicPrepared|DynamicCommit|DynamicCall|DynamicResolution|AccessMode|ReadMode|EffectMode|EffectKind|EffectVersion|AccessCapabilityContract|AccessEvidenceValue|PureImplementation|ReadImplementation|AccessImplementation|AccessResolution|AcceptedOutcomeAccess|AcceptedIntegrityAccess|UnresolvedAccess|UnresolvedClassification|ReadResolution|ReadObservation|ReadUnavailable|BlockedIntegrityProjection|PreparedExecution|PreparedAccess|PreparedRead|CommittedCall|CommittedRead|PreparationRef|QualifiedRecordedEvidence|OpenedPreparationCommit|QualifiedAdapter|RunSession|SuspendedRun|ParkedRun|ParkReason|PendingConclusion|AdmissionInput|AdmissionFailure|AdmissionConflict|ResumeFailure|TerminalRun|RuntimeLimits|NoLongerSelected|SpawnStep|ResumeStep|RuntimeStep|PendingAppend|PendingWork|PendingKey|StoreBrand|StructuredStore|OpenedStructuredStore|StoreParts|QualifiedHistoryPort|HistoryReader|StoreAuditPort|QualifiedRun|RunReducer|RunSelection|RunAction|SelectedRun|FactContinuation|SemanticStore|AccessActionMode|AccessPreparationOutcome|AdmissionOutcome|SelectedConclusionOutcome|SelectedConclusionPreparationOutcome|StructuredStoreBackend|MemoryStructuredBackend|ReadyError|DurabilityProfile|PostgresError|StoreWorkLimits|RawHistoryLoadLimit|BackendFuture|BackendResult|BackendError|BackendAppendCommand|BackendAppendOutcome|AppendDisposition|RunAppend|RunPosition|RawRunPrefix|RawFrameBytes|RawFactPublication|RawFactSnapshot|AppendRequestId|IdempotencyConflict|AcknowledgementUnknown|UnavailableBeforeSubmission|InvalidPhysicalCommand|AlreadyConcludedSame|FailureValue|SequentialControlAddress|StateAddress|BindingDescriptor|EvmBalanceBindings|EvmAdapterBinding|EvmLiveAssembly|RunFrame|RunRecord|RunAdmitted|StatePrepared|PreparationMode|StateConcludedAccess|StateOutcome|ImmutableObject|ValueRef|RecordLogicalKey|FactSelectionMode|NoPriorFacts|PriorRunFacts|FactSelection|FactProposalSet|FactHead|FactPublication|MfmConfig|ValidatedConfig|ProvenConfiguration|ConfigurationKey|ConfigurationPosition|ConfigurationRef|ConfigurationRevision|ConfigurationAppend|ConfigurationHeadProjection|ConfigurationStore|ResolvedConfiguration|ResolvedConfigurationHead|ConfigurationWriteSession|PreparedConfigurationAppend|SuspendedConfigurationAppend|ConfigurationAppendCommand|ConfigurationCommitOutcome|ConfigurationAppendDisposition|BackendConfigurationOutcome|RawConfigurationRevision|PortableRunBundle|PortableRun|ExportedRun|ReservationKey|StoreScopeId|StoreEpoch|TenantScopeId|StructuredStoreIdentity|AdmitRunRequest|AdmitRunResponse|DriveResponse|RunStatus|PublicRunView|ReplayResponse|TraceResponse|TraceRecord|AccessAuditResponse|AccessAuditEntry|EvmConfig|EvmTransactionTarget|EvmSubmission[A-Za-z0-9_]*|NonceReservationIntent|NonceReservationEvidence|BroadcastIntent|BroadcastEvidence|EvmAdapterError|WalletNonce[A-Za-z0-9_]*|WalletNonceDomain[A-Za-z0-9_]*|PostgresWalletNonce[A-Za-z0-9_]*|AccessBindingV2|CompletionCell|FinalizationCell|PendingOwnerPermit|ResolverToken)\b
regex	production	(struct|enum|trait|type)[[:space:]]+(SuspendedRun|HotValue|StateExecutionKey)\b
regex	production	fn[[:space:]]+application_catalog\b
regex	production	pub[[:space:]]+fn[[:space:]]+canonical_value\b
regex	production	enum[[:space:]]+RuntimeError[[:space:]]*\{[^}]*\bConflict\b
regex	production	\b(max_active_sessions|max_cpu_jobs|max_planning_jobs|max_ingress_jobs|active_sessions|cpu_jobs|planning_jobs|ingress_jobs)\b
regex	production	impl[^{]*ReadCapabilityContract[[:space:]]+for[[:space:]]+EvmCapability[[:space:]]*<[[:space:]]*3[[:space:]]*>
regex	production	(struct|enum|trait|type)[[:space:]]+(AccessIngressFuture|PreparationError|LoadSelector)\b
regex	production	fn[[:space:]]+(replay_terminality|retained_program|validate_prefix|list_runs|parse_admission_json|public_signer_key_instance_ref|evm_live_adapter_implementation_ref)\b
regex	production	impl[[:space:]]+PostgresStore[^}]*fn[[:space:]]+install_schema\b
regex	production	\b(next_address|failure_next_address|continuation_contract_ref|maximum_conclusion_bytes|total_attempt_bound|fact_selection_required|effect_domain|execution_binding|MAX_ADMISSION_BYTES)\b|payload_contract_ref[[:space:]]*:
regex	production	RunAdmitted[[:space:]]*\{[^}]*\bentry_point[[:space:]]*:|enum[[:space:]]+LoadSelector[[:space:]]*\{[^}]*(Current|Exact)\b
regex	production	(struct|enum|trait|type)[[:space:]]+(PortfolioAdmissionPlan|ProgramRef|CatalogBrand|EvmPureState|EvmAccessState|PortfolioPureState|OwnedSemaphorePermit|WalletNonceAuthority)\b
regex	production	QualifiedTypedValue[^}]*fn[[:space:]]+erase\b|enum[[:space:]]+ProgramError[[:space:]]*\{[^}]*(InvalidValue|InvalidCatalog)\b
regex	production	\b(call_id|root_contract_ref|captured_head_sequence|captured_head_digest)\b|struct[[:space:]]+StoredRunBytes[[:space:]]*\{[^}]*(head_sequence|head_digest)\b|failure_contract_ref[[:space:]]*:[[:space:]]*Option
regex	crates/kernel/store/src	(struct|enum|trait|type)[[:space:]]+(PreparedAdmission|PreparedConclusion)\b
regex	docs	Journal[^\n]*(writes|owns|emits)[^\n]*state_prepared
regex	docs	Application[^\n]*(owns|maintains)[^\n]*(suspended run|status[^\n]*frames)
regex	production	pub[[:space:]]+fn[[:space:]]+(application_catalog|snapshot_closure_document|validate_runtime_closure|try_downcast|from_pool|rotate_identity|check_ready|profile|migrate|export_run|plan_submission|submission_closure_documents)\b
regex	crates/kernel/capabilities/src	pub[[:space:]]+(?:(struct|enum|type)[[:space:]]+ProposedStateOutcome\b|use[[:space:]]+[^;]*\bProposedStateOutcome\b)
regex	crates/kernel/store/src	pub[[:space:]]+(?:(struct|enum|type)[[:space:]]+StoreOpenError\b|use[[:space:]]+[^;]*\bStoreOpenError\b)
regex	crates/live/evm/src	pub[[:space:]]+(?:(struct|enum|type)[[:space:]]+EvmPhysicalTarget\b|use[[:space:]]+[^;]*\bEvmPhysicalTarget\b)
regex	production	pub[[:space:]]+type[[:space:]]+BoxFuture\b
regex	production	pub[[:space:]]+enum[[:space:]]+AppendResult[[:space:]]*\{[^}]*(Existing|Stale)\b
regex	production	pub[[:space:]]+enum[[:space:]]+RunViewState[[:space:]]*\{[^}]*Waiting\b
regex	production	pub[[:space:]]+enum[[:space:]]+EvmProviderResponse[[:space:]]*\{[^}]*(Broadcast|PossibleEntry)\b
regex	production	pub[[:space:]]+enum[[:space:]]+EvmReadSubject[[:space:]]*\{[^}]*(TransactionReceipt|FinalizedHead|CanonicalInclusionBlock)\b
regex	production	pub[[:space:]]+enum[[:space:]]+EvmReadValue[[:space:]]*\{[^}]*(Receipt|FinalizedHead|CanonicalBlock)\b
regex	schema	mfm-access-binding@2|mfm\.execution-binding@1|state_prepared|state_concluded_access|preparation_sequence|record_digest
regex	schema	mfm\.evm/submit-transaction@1|EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID|mfm\.evm\.capability\.(nonce|broadcast|submission)|mfm\.evm\.state\.(nonce|broadcast|submission)
regex	schema	MAX_(STATE_CONCLUSION_BYTES|RUN_FRAME_BYTES|RUN_OBJECTS|CONFIGURATION_[A-Z0-9_]*)
regex	task	configuration_capacity_accepts_each_exact_bound|pure_session_advances_through_runtime|evm_submission|submit_transaction
regex	sql	(?i:CREATE[[:space:]]+TABLE[^;]*(run_reservations|configuration|fact|receipt|request|membership|global_object))
regex	sql	(?i:(reserved_bytes|reserved_frames|reserved_objects|expected_position|candidate_digest|append_command_digest|predecessor_sequence|object_count))
regex	docs	(portable|offline)[[:space:]]+(run[[:space:]]+)?(replay|export)[[:space:]]+(is|are)[[:space:]]+(supported|available)
regex	docs	(publish|load|resolve)[a-z -]*configuration|fact (frontier|publication|proposal|proof)
regex	docs	(nonce|signer|broadcast)[^\n]*(Application|application)[^\n]*(supports|owns|provides)
regex	docs	App-owned (status|frame)|StatePrepared[[:space:]]+(is|record|event)
regex	production	(struct|enum|trait|type)[[:space:]]+(EffectKindKind|EffectVersionKind|ReservationInstruction|ReplayReducer|AcknowledgementLease|AcknowledgmentLease|RuntimeCancellationToken|RuntimeTimeoutPolicy|CompletionCell|FinalizationCell|PendingOwnerPermit|ResolverToken|AccessBindingV2|[A-Za-z0-9_]*(Cancellation|Timeout)[A-Za-z0-9_]*|[A-Za-z0-9_]*(Finalizer|CompletionTask))\b|trait[[:space:]]+Store[^{]*\{[^}]*fn[[:space:]]+check_ready\b|struct[[:space:]]+StateDeclaration[^{]*\{[^}]*(address|entry_address)[[:space:]]*:|\b(next_address|failure_next_address|entry_address|occurrence|terminal|completion_cell|finalization_cell|pending_owner_limit|pending_owner_permit|resolver_token|acknowledgement_lease|cancellation_token|timeout_policy)[[:space:]]*:|\b(preparation_ordinal|permits_replacement|replaces)\b|\bwallet_nonce_effect_domain\b|enum[[:space:]]+SchemaKind[^{]*\{[^}]*(StateInput|OperationOutput|PublicOutput)\b|enum[[:space:]]+ReadCapabilityFamily[^{]*\{[^}]*\bSubmissionStatus\b|ReadCapabilityFamily[[:space:]]*::[[:space:]]*SubmissionStatus\b|fn[[:space:]]+detached_completion_finalizer\b|#[[:space:]]*\[[[:space:]]*proc_macro_derive[[:space:]]*\([[:space:]]*(StateInput|OperationOutput|PublicOutputs)\b
regex	production	\bEvmCapability[[:space:]]*<
regex	production	\bEvmState[[:space:]]*<
regex	production	\bPortfolioState[[:space:]]*<
coverage	.	fd8445fdba6dbd89f20a2cbb1009296d64d2fbd60d6dc766aba1dc0470ede35e
```

## Machine coverage map

Each nonblank row maps the same-numbered entry in the complete deletion ledger to the implicit
`Rnnn` identifier of the same-numbered machine rule above. The scanner requires all 309 ordinals
exactly once, rejects references outside the exact ruleset, fingerprints the ledger and full rule
contents independently, and then executes every referenced rule.

```cutover-coverage
1	R032
2	R032
3	R032
4	R034
5	R050
6	R050
7	R044
8	R032
9	R044
10	R032
11	R032
12	R032
13	R032
14	R035
15	R032
16	R032
17	R045
18	R050
19	R044
20	R045
21	R045
22	R032
23	R070
24	R070
25	R070
26	R070
27	R032
28	R032
29	R032
30	R032
31	R032
32	R032
33	R032
34	R032
35	R032
36	R032
37	R032
38	R032
39	R032
40	R032
41	R032
42	R070
43	R032
44	R032
45	R051
46	R032
47	R032
48	R032
49	R044
50	R032
51	R032
52	R032
53	R032
54	R032
55	R032
56	R032
57	R032
58	R032
59	R032
60	R032
61	R032
62	R032
63	R032
64	R032
65	R032
66	R039
67	R054
68	R032
69	R032
70	R039
71	R070
72	R056
73	R033
74	R033
75	R046
76	R032
77	R032
78	R032
79	R032
80	R032
81	R032
82	R032
83	R032
84	R032
85	R032
86	R036
87	R032
88	R032
89	R032
90	R032
91	R032
92	R032
93	R032
94	R070
95	R070
96	R070
97	R070
98	R070
99	R032
100	R037
101	R037
102	R037
103	R037
104	R037
105	R037
106	R037
107	R037
108	R044
109	R070
110	R070
111	R032
112	R032
113	R032
114	R032
115	R032
116	R032
117	R032
118	R032
119	R032
120	R032
121	R032
122	R032
123	R047
124	R047
125	R032
126	R032
127	R032
128	R040
129	R040
130	R040
131	R032
132	R032
133	R032
134	R032
135	R032
136	R032
137	R032
138	R070
139	R032
140	R032
141	R032
142	R050
143	R050
144	R050
145	R041
146	R046
147	R032
148	R032
149	R032
150	R032
151	R032
152	R032
153	R055
154	R032
155	R032
156	R043
157	R032
158	R032
159	R032
160	R032
161	R032
162	R064
163	R067
164	R064
165	R032
166	R032
167	R032
168	R032
169	R032
170	R032
171	R065
172	R065
173	R032
174	R046
175	R046
176	R032
177	R070
178	R032
179	R070
180	R042
181	R042
182	R042
183	R042
184	R042
185	R070
186	R032
187	R032
188	R032
189	R032
190	R053
191	R043
192	R032
193	R032
194	R070
195	R032
196	R060
197	R042
198	R032
199	R032
200	R032
201	R029
202	R032
203	R032
204	R032
205	R032
206	R032
207	R032
208	R032
209	R067
210	R067
211	R064
212	R032
213	R032
214	R032
215	R032
216	R032
217	R032
218	R032
219	R032
220	R032
221	R032
222	R032
223	R032
224	R032
225	R032
226	R032
227	R032
228	R032
229	R032
230	R032
231	R032
232	R067
233	R064
234	R067
235	R067
236	R067
237	R062
238	R062
239	R062
240	R032
241	R032
242	R032
243	R050
244	R066
245	R066
246	R060
247	R032
248	R032
249	R070
250	R032
251	R064
252	R065
253	R065
254	R064
255	R062
256	R062
257	R032
258	R062
259	R065
260	R065
261	R064
262	R032
263	R032
264	R032
265	R032
266	R064
267	R064
268	R064
269	R064
270	R064
271	R029
272	R070
273	R049
274	R049
275	R032
276	R040
277	R032
278	R032
279	R032
280	R032
281	R032
282	R032
283	R032
284	R032
285	R061
286	R032
287	R032
288	R032
289	R032
290	R032
291	R050
292	R050
293	R070
294	R040
295	R040
296	R032
297	R057
298	R032
299	R058
300	R059
301	R070
302	R061
303	R061
304	R044
305	R029
306	R061
307	R061
308	R004
309	R005
310	R071
311	R072
312	R073
```
