use super::*;

/// Subject identity for an internal collector checkpoint fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_subject",
    version = "1",
    schema = "mfm.bitcoin.fact.collector_checkpoint.subject"
)]
pub struct CollectorCheckpointSubject {
    collector_kind: String,
    semantic_source_identity: String,
    scope: String,
    partition: String,
    network: String,
    bitcoin_network: String,
}

impl CollectorCheckpointSubject {
    /// Creates checkpoint subject material.
    pub fn new(
        collector_kind: impl Into<String>,
        source_identity: &BtcSourceIdentity,
        partition: impl Into<String>,
        bitcoin_network: impl Into<String>,
        network: &BtcNetworkId,
    ) -> Result<Self, BtcStateError> {
        let bitcoin_network = bitcoin_network.into();
        validate_bitcoin_network(&bitcoin_network)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        Ok(Self {
            collector_kind: collector_kind.into(),
            semantic_source_identity: source_identity.as_str().to_owned(),
            scope: DEFAULT_SCOPE.to_owned(),
            partition: partition.into(),
            network: network.as_str().to_owned(),
            bitcoin_network,
        })
    }

    /// Returns the collector kind.
    pub fn collector_kind(&self) -> &str {
        &self.collector_kind
    }

    /// Returns the non-secret semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the checkpoint visibility scope tag.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Returns the checkpoint partition.
    pub fn partition(&self) -> &str {
        &self.partition
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the Bitcoin Core network tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }
}

/// Observed result for an internal collector checkpoint fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_response",
    version = "1",
    schema = "mfm.bitcoin.fact.collector_checkpoint.response"
)]
pub struct CollectorCheckpointResponse {
    high_watermark_height: u64,
    high_watermark_hash: String,
    predecessor_checkpoint_ref: Option<String>,
    predecessor_checkpoint_hash: Option<String>,
    finality_policy: String,
    confirmation_depth: Option<u64>,
}

impl CollectorCheckpointResponse {
    /// Creates checkpoint response material.
    pub fn new(
        high_watermark_height: u64,
        high_watermark_hash: impl Into<String>,
        predecessor_checkpoint_ref: Option<String>,
        predecessor_checkpoint_hash: Option<String>,
        finality: BtcFinality,
    ) -> Self {
        Self {
            high_watermark_height,
            high_watermark_hash: high_watermark_hash.into(),
            predecessor_checkpoint_ref,
            predecessor_checkpoint_hash,
            finality_policy: finality_policy_tag(finality).to_owned(),
            confirmation_depth: finality.confirmation_depth(),
        }
    }

    /// Returns the high-watermark height.
    pub const fn high_watermark_height(&self) -> u64 {
        self.high_watermark_height
    }

    /// Returns the high-watermark hash.
    pub fn high_watermark_hash(&self) -> &str {
        &self.high_watermark_hash
    }

    /// Returns the predecessor checkpoint material hash, when present.
    pub fn predecessor_checkpoint_hash(&self) -> Option<&str> {
        self.predecessor_checkpoint_hash.as_deref()
    }

    /// Returns the finality policy tag.
    pub fn finality_policy(&self) -> &str {
        &self.finality_policy
    }

    /// Returns the confirmation depth attached to the finality policy.
    pub const fn confirmation_depth(&self) -> Option<u64> {
        self.confirmation_depth
    }
}

/// Control fact for a collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_fact",
    version = "1",
    schema = "mfm.bitcoin.fact.collector_checkpoint"
)]
#[mfm_fact(kind = "collector.checkpoint")]
#[mfm_fact(field(
    id = "subject.collector_kind",
    source = "subject",
    path = "collector_kind",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.semantic_source_identity",
    source = "subject",
    path = "semantic_source_identity",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.scope",
    source = "subject",
    path = "scope",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.partition",
    source = "subject",
    path = "partition",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.bitcoin_network",
    source = "subject",
    path = "bitcoin_network",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "result.high_watermark_height",
    source = "result",
    path = "high_watermark_height",
    value_type = "unsigned_integer",
    operators(
        equal,
        greater_than,
        greater_than_or_equal,
        less_than,
        less_than_or_equal
    ),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(field(
    id = "result.high_watermark_hash",
    source = "result",
    path = "high_watermark_hash",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "result.predecessor_checkpoint_ref",
    source = "result",
    path = "predecessor_checkpoint_ref",
    value_type = "string",
    exposure = "hidden",
    optional
))]
#[mfm_fact(field(
    id = "result.predecessor_checkpoint_hash",
    source = "result",
    path = "predecessor_checkpoint_hash",
    value_type = "digest",
    exposure = "hidden",
    optional
))]
#[mfm_fact(field(
    id = "result.finality_policy",
    source = "result",
    path = "finality_policy",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(ordering(
    name = "result.high_watermark_height.desc",
    term(
        field = "result.high_watermark_height",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct CollectorCheckpointFact {
    subject: CollectorCheckpointSubject,
    response: CollectorCheckpointResponse,
}

impl CollectorCheckpointFact {
    /// Creates a collector checkpoint fact from subject and response material.
    pub const fn new(
        subject: CollectorCheckpointSubject,
        response: CollectorCheckpointResponse,
    ) -> Self {
        Self { subject, response }
    }

    /// Returns the checkpoint subject material.
    pub const fn subject(&self) -> &CollectorCheckpointSubject {
        &self.subject
    }

    /// Returns the checkpoint response material.
    pub const fn response(&self) -> &CollectorCheckpointResponse {
        &self.response
    }
}

/// Config for querying the latest collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.query_collector_checkpoint",
    validate = "validate_query_collector_checkpoint_config"
)]
pub struct QueryCollectorCheckpointConfig {
    /// Collector kind whose checkpoint should be loaded.
    pub collector_kind: String,
    /// Non-secret semantic source identity whose checkpoint should be loaded.
    pub semantic_source_identity: String,
    /// Semantic checkpoint partition.
    pub partition: String,
    /// Semantic Bitcoin network id.
    pub network: String,
    /// Expected Bitcoin Core network tag (`main`, `test`, `signet`, or `regtest`).
    pub bitcoin_network: String,
    /// Non-secret store scope for this internal fact-index query.
    pub store_scope: String,
}

/// Validates collector checkpoint query config.
pub fn validate_query_collector_checkpoint_config(
    config: &QueryCollectorCheckpointConfig,
) -> Result<(), String> {
    validate_collector_checkpoint_common(
        &config.collector_kind,
        &config.semantic_source_identity,
        &config.partition,
        &config.network,
        &config.bitcoin_network,
    )?;
    StoreScopeRef::new(&config.store_scope).map_err(|error| error.to_string())?;
    Ok(())
}

impl QueryCollectorCheckpointConfig {
    /// Builds the subject identity for this checkpoint query.
    pub fn checkpoint_subject(&self) -> Result<CollectorCheckpointSubject, BtcStateError> {
        validate_query_collector_checkpoint_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let source_identity = BtcSourceIdentity::new(&self.semantic_source_identity)?;
        let network = BtcNetworkId::new(&self.network)?;
        CollectorCheckpointSubject::new(
            self.collector_kind.clone(),
            &source_identity,
            self.partition.clone(),
            self.bitcoin_network.clone(),
            &network,
        )
    }

    /// Builds the Control fact-index read request for the latest checkpoint.
    pub fn request(&self) -> Result<FactIndexReadRequest, BtcStateError> {
        validate_query_collector_checkpoint_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let descriptor =
            CollectorCheckpointFact::descriptor().map_err(|error| BtcStateError::InvalidInput {
                reason: format!("collector checkpoint descriptor invalid: {error}"),
            })?;
        let plan = compile_fact_query_plan(&descriptor, collector_checkpoint_query_input(self)?)
            .map_err(|error| BtcStateError::InvalidInput {
                reason: error.to_string(),
            })?;
        FactIndexReadRequest::new(plan).map_err(|error| BtcStateError::InvalidInput {
            reason: error.to_string(),
        })
    }

    /// Materializes a checkpoint fact from retained response material.
    pub fn checkpoint_fact_from_response(
        &self,
        response: CollectorCheckpointResponse,
    ) -> Result<CollectorCheckpointFact, BtcStateError> {
        Ok(CollectorCheckpointFact::new(
            self.checkpoint_subject()?,
            response,
        ))
    }

    /// Rebuilds loaded checkpoint output from recorded query evidence and retained response material.
    pub fn loaded_checkpoint_from_replay_evidence(
        &self,
        evidence: &mfm_facts::FactQueryEvidence,
        response: Option<CollectorCheckpointResponse>,
    ) -> Result<LoadedCollectorCheckpoint, BtcStateError> {
        mfm_facts::validate_fact_query_evidence(evidence).map_err(|error| {
            BtcStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?;
        match (
            evidence.receipt().returned_refs().len(),
            evidence.selection().selected_indices(),
            response,
        ) {
            (0, [], None) => Ok(LoadedCollectorCheckpoint::new(None)),
            (1, [0], Some(response)) => self
                .checkpoint_fact_from_response(response)
                .map(|checkpoint| LoadedCollectorCheckpoint::new(Some(checkpoint))),
            _ => Err(BtcStateError::InvalidInput {
                reason: "checkpoint replay evidence did not match latest-checkpoint selection"
                    .to_owned(),
            }),
        }
    }
}

impl Default for QueryCollectorCheckpointConfig {
    fn default() -> Self {
        Self {
            collector_kind: "btc-chain-head".to_owned(),
            semantic_source_identity: "public-bitcoin-core".to_owned(),
            partition: "chain-head".to_owned(),
            network: "bitcoin-mainnet".to_owned(),
            bitcoin_network: "main".to_owned(),
            store_scope: DEFAULT_STORE_SCOPE.to_owned(),
        }
    }
}

/// Input for querying a collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.query_collector_checkpoint")]
pub struct QueryCollectorCheckpointInput {}

/// Complete deterministic plan for loading the latest collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_read_plan",
    version = "1",
    schema = "mfm.bitcoin.external_read.collector_checkpoint.plan"
)]
pub struct QueryCollectorCheckpointReadPlan {
    collector_kind: String,
    semantic_source_identity: String,
    partition: String,
    network: String,
    bitcoin_network: String,
    store_scope: String,
}

impl QueryCollectorCheckpointReadPlan {
    /// Creates a plan from a validated checkpoint-query config.
    pub fn from_config(config: &QueryCollectorCheckpointConfig) -> Result<Self, BtcStateError> {
        validate_query_collector_checkpoint_config(config)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        Ok(Self {
            collector_kind: config.collector_kind.clone(),
            semantic_source_identity: config.semantic_source_identity.clone(),
            partition: config.partition.clone(),
            network: config.network.clone(),
            bitcoin_network: config.bitcoin_network.clone(),
            store_scope: config.store_scope.clone(),
        })
    }

    /// Reconstructs the exact fact-index request declared by this plan.
    pub fn request(&self) -> Result<FactIndexReadRequest, BtcStateError> {
        self.config().request()
    }

    /// Returns the sole retained response ref admitted by the latest-checkpoint plan.
    pub fn selected_response_ref<'a>(
        &self,
        response: &'a mfm_facts::FactQueryResult,
    ) -> Result<Option<&'a mfm_facts::InternalFactRef>, BtcStateError> {
        latest_checkpoint_response_ref(response)
    }

    /// Builds the state-owned latest-checkpoint selection evidence.
    pub fn selection_evidence(
        &self,
        response: &mfm_facts::FactQueryResult,
    ) -> Result<FactSelectionEvidence, BtcStateError> {
        let selected_indices = if has_latest_checkpoint_row(response)? {
            vec![0]
        } else {
            Vec::new()
        };
        FactSelectionEvidence::new(selection_policy_hash(), selected_indices, None).map_err(
            |error| BtcStateError::InvalidInput {
                reason: error.to_string(),
            },
        )
    }

    fn config(&self) -> QueryCollectorCheckpointConfig {
        QueryCollectorCheckpointConfig {
            collector_kind: self.collector_kind.clone(),
            semantic_source_identity: self.semantic_source_identity.clone(),
            partition: self.partition.clone(),
            network: self.network.clone(),
            bitcoin_network: self.bitcoin_network.clone(),
            store_scope: self.store_scope.clone(),
        }
    }
}

/// Canonical retained evidence for a collector-checkpoint fact query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_read_evidence",
    version = "1",
    schema = "mfm.bitcoin.external_read.collector_checkpoint.evidence"
)]
pub struct QueryCollectorCheckpointReadEvidence {
    fact_query_evidence_hashes: Vec<String>,
    checkpoint_response: Option<CollectorCheckpointResponse>,
}

impl QueryCollectorCheckpointReadEvidence {
    /// Binds hydrated checkpoint material to the exact retained fact-query evidence.
    pub fn new(
        query: &mfm_facts::FactQueryEvidence,
        checkpoint_response: Option<CollectorCheckpointResponse>,
    ) -> Result<Self, BtcStateError> {
        let hash = mfm_facts::fact_query_evidence_hash(query).map_err(|error| {
            BtcStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?;
        Ok(Self {
            fact_query_evidence_hashes: vec![hash.as_str().to_owned()],
            checkpoint_response,
        })
    }

    fn validate_query(&self, query: &mfm_facts::FactQueryEvidence) -> Result<(), BtcStateError> {
        let hash = mfm_facts::fact_query_evidence_hash(query).map_err(|error| {
            BtcStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?;
        if self.fact_query_evidence_hashes.as_slice() != [hash.as_str()] {
            return Err(BtcStateError::InvalidInput {
                reason: "checkpoint evidence does not bind the retained fact query".to_owned(),
            });
        }
        Ok(())
    }
}

/// Output from loading a collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "loaded_collector_checkpoint",
    version = "1",
    schema = "mfm.bitcoin.state.output.loaded_collector_checkpoint"
)]
pub struct LoadedCollectorCheckpoint {
    checkpoint: Option<CollectorCheckpointFact>,
}

impl LoadedCollectorCheckpoint {
    /// Creates loaded checkpoint output.
    pub const fn new(checkpoint: Option<CollectorCheckpointFact>) -> Self {
        Self { checkpoint }
    }

    /// Returns the loaded checkpoint fact, when present.
    pub const fn checkpoint(&self) -> Option<&CollectorCheckpointFact> {
        self.checkpoint.as_ref()
    }
}

/// State contract for querying the latest Control collector checkpoint.
pub struct QueryCollectorCheckpointState {
    config: QueryCollectorCheckpointConfig,
}

impl QueryCollectorCheckpointState {
    /// Builds the fact-index request declared by this state config.
    pub fn request(&self) -> Result<FactIndexReadRequest, BtcStateError> {
        self.config.request()
    }

    /// Materializes a loaded checkpoint from an evidence-backed fact-index response.
    pub fn materialize_response(
        &self,
        response: &mfm_facts::FactQueryResult,
        checkpoint: Option<CollectorCheckpointFact>,
    ) -> Result<LoadedCollectorCheckpoint, BtcStateError> {
        match (has_latest_checkpoint_row(response)?, checkpoint) {
            (false, None) => Ok(LoadedCollectorCheckpoint::new(None)),
            (true, Some(checkpoint)) => Ok(LoadedCollectorCheckpoint::new(Some(checkpoint))),
            (false, Some(_)) => Err(BtcStateError::InvalidInput {
                reason: "checkpoint material was supplied for an empty fact-index receipt"
                    .to_owned(),
            }),
            (true, None) => Err(BtcStateError::InvalidInput {
                reason: "checkpoint receipt row requires materialized checkpoint fact".to_owned(),
            }),
        }
    }

    /// Returns the retained response fact ref selected by the latest-checkpoint policy.
    pub fn selected_checkpoint_response_ref<'a>(
        &self,
        response: &'a mfm_facts::FactQueryResult,
    ) -> Result<Option<&'a mfm_facts::InternalFactRef>, BtcStateError> {
        latest_checkpoint_response_ref(response)
    }

    /// Materializes a checkpoint fact from retained response material.
    pub fn checkpoint_fact_from_response(
        &self,
        response: CollectorCheckpointResponse,
    ) -> Result<CollectorCheckpointFact, BtcStateError> {
        self.config.checkpoint_fact_from_response(response)
    }

    /// Builds selection evidence for the state-owned latest-checkpoint policy.
    pub fn selection_evidence(
        &self,
        response: &mfm_facts::FactQueryResult,
    ) -> Result<FactSelectionEvidence, BtcStateError> {
        QueryCollectorCheckpointReadPlan::from_config(&self.config)?.selection_evidence(response)
    }
}

fn has_latest_checkpoint_row(response: &mfm_facts::FactQueryResult) -> Result<bool, BtcStateError> {
    latest_checkpoint_response_ref(response).map(|row| row.is_some())
}

fn latest_checkpoint_response_ref(
    response: &mfm_facts::FactQueryResult,
) -> Result<Option<&mfm_facts::InternalFactRef>, BtcStateError> {
    match response.rows() {
        [] => Ok(None),
        [row] => Ok(Some(row.fact_ref())),
        _ => Err(BtcStateError::InvalidInput {
            reason: "latest checkpoint query must return at most one row".to_owned(),
        }),
    }
}

impl StateSpec for QueryCollectorCheckpointState {
    type Config = QueryCollectorCheckpointConfig;
    type Context = NoContext;
    type Input = QueryCollectorCheckpointInput;
    type Output = LoadedCollectorCheckpoint;
    type Effect = ReadExternal;
    type Caps = (FactIndexReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("collector_checkpoint.query")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("collector_checkpoint.query")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.collector_checkpoint.query"
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for QueryCollectorCheckpointState {
    type Plan = QueryCollectorCheckpointReadPlan;
    type Evidence = QueryCollectorCheckpointReadEvidence;

    fn plan(
        &self,
        _input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        QueryCollectorCheckpointReadPlan::from_config(&self.config).map_err(StateError::from)
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        let [query] = evidence.fact_query_evidence() else {
            return Err(StateError::Message(
                "checkpoint read requires exactly one fact-query evidence value".to_owned(),
            ));
        };
        mfm_facts::validate_fact_query_evidence(query)
            .map_err(|error| StateError::Message(error.to_string()))?;
        evidence.primary_evidence().validate_query(query)?;
        let plan = self.plan(input, context)?;
        let request = plan.request()?;
        if query.plan() != request.plan() {
            return Err(StateError::Message(
                "checkpoint fact-query evidence does not match the state-authored plan".to_owned(),
            ));
        }
        let rows = mfm_facts::fact_query_result_rows_from_receipt(query.receipt());
        let result = mfm_facts::FactQueryResult::new(rows, query.receipt().clone())
            .map_err(|error| StateError::Message(error.to_string()))?;
        let expected_selection = self.selection_evidence(&result)?;
        if query.selection() != &expected_selection {
            return Err(StateError::Message(
                "checkpoint fact-query selection does not match state policy".to_owned(),
            ));
        }
        self.config
            .loaded_checkpoint_from_replay_evidence(
                query,
                evidence.primary_evidence().checkpoint_response.clone(),
            )
            .map_err(StateError::from)
    }
}

/// Config for a collector checkpoint fact recording state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.record_collector_checkpoint",
    validate = "validate_record_collector_checkpoint_config"
)]
pub struct RecordCollectorCheckpointConfig {
    /// Collector kind whose checkpoint should be recorded.
    pub collector_kind: String,
    /// Semantic checkpoint partition.
    pub partition: String,
}

/// Validates collector checkpoint record config.
pub fn validate_record_collector_checkpoint_config(
    config: &RecordCollectorCheckpointConfig,
) -> Result<(), String> {
    validate_non_secret_label("collector_kind", &config.collector_kind)?;
    validate_non_secret_label("partition", &config.partition)
}

impl RecordCollectorCheckpointConfig {
    /// Builds the checkpoint subject identity for a recorded chain-head fact.
    pub fn checkpoint_subject(
        &self,
        chain_head_fact: &BtcChainHeadFact,
    ) -> Result<CollectorCheckpointSubject, BtcStateError> {
        validate_record_collector_checkpoint_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        Ok(CollectorCheckpointSubject {
            collector_kind: self.collector_kind.clone(),
            semantic_source_identity: chain_head_fact
                .subject()
                .semantic_source_identity()
                .to_owned(),
            scope: DEFAULT_SCOPE.to_owned(),
            partition: self.partition.clone(),
            network: chain_head_fact.subject().network().to_owned(),
            bitcoin_network: chain_head_fact.subject().bitcoin_network().to_owned(),
        })
    }
}

/// Input for recording a collector checkpoint fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.record_collector_checkpoint")]
pub struct RecordCollectorCheckpointInput {
    /// Recorded upstream chain-head fact output.
    pub chain_head_fact: BtcChainHeadFact,
    /// Whole upstream checkpoint query output.
    pub loaded_checkpoint: LoadedCollectorCheckpoint,
}

/// State contract for recording a collector checkpoint fact.
pub struct RecordCollectorCheckpointState {
    config: RecordCollectorCheckpointConfig,
}

impl StateSpec for RecordCollectorCheckpointState {
    type Config = RecordCollectorCheckpointConfig;
    type Context = NoContext;
    type Input = RecordCollectorCheckpointInput;
    type Output = CollectorCheckpointFact;
    type Effect = ManagedPlatformWrite;
    type Caps = (FactRecordCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("collector_checkpoint.record")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("collector_checkpoint.record")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.collector_checkpoint.record"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<CollectorCheckpointFact>()?])
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ManagedWriteState for RecordCollectorCheckpointState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(
            record_collector_checkpoint_from_outputs(
                &self.config,
                &input.chain_head_fact,
                &input.loaded_checkpoint,
            )
            .map_err(StateError::from),
        )
    }
}

/// Recomputes the collector checkpoint fact from the proven chain-head fact and predecessor output.
pub fn record_collector_checkpoint_from_outputs(
    config: &RecordCollectorCheckpointConfig,
    chain_head_fact: &BtcChainHeadFact,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<CollectorCheckpointFact, BtcStateError> {
    validate_loaded_checkpoint_for_recorded_fact(config, chain_head_fact, loaded_checkpoint)?;

    let subject = config.checkpoint_subject(chain_head_fact)?;
    let predecessor_checkpoint_hash = loaded_checkpoint
        .checkpoint()
        .map(checkpoint_material_hash)
        .transpose()?
        .map(|digest| digest.as_str().to_owned());
    let response = CollectorCheckpointResponse {
        high_watermark_height: chain_head_fact.response().block_height(),
        high_watermark_hash: chain_head_fact.response().block_hash().to_owned(),
        predecessor_checkpoint_ref: None,
        predecessor_checkpoint_hash,
        finality_policy: chain_head_fact.response().finality_policy().to_owned(),
        confirmation_depth: chain_head_fact.response().confirmation_depth(),
    };
    Ok(CollectorCheckpointFact::new(subject, response))
}

fn validate_loaded_checkpoint_for_recorded_fact(
    config: &RecordCollectorCheckpointConfig,
    chain_head_fact: &BtcChainHeadFact,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<(), BtcStateError> {
    let Some(previous) = loaded_checkpoint.checkpoint() else {
        return Ok(());
    };
    let previous_subject = previous.subject();
    if previous_subject.collector_kind() != config.collector_kind
        || previous_subject.partition() != config.partition
        || previous_subject.scope() != DEFAULT_SCOPE
        || previous_subject.network() != chain_head_fact.subject().network()
        || previous_subject.bitcoin_network() != chain_head_fact.subject().bitcoin_network()
        || previous_subject.semantic_source_identity()
            != chain_head_fact.subject().semantic_source_identity()
        || previous.response().finality_policy() != chain_head_fact.response().finality_policy()
        || previous.response().confirmation_depth()
            != chain_head_fact.response().confirmation_depth()
    {
        return Err(BtcStateError::InvalidInput {
            reason: "loaded checkpoint is incompatible with recorded chain-head fact".to_owned(),
        });
    }
    if chain_head_fact.response().block_height() < previous.response().high_watermark_height() {
        return Err(BtcStateError::InvalidInput {
            reason: "new checkpoint height must not move behind loaded checkpoint".to_owned(),
        });
    }
    Ok(())
}

fn collector_checkpoint_query_input(
    config: &QueryCollectorCheckpointConfig,
) -> Result<FactQueryInput, BtcStateError> {
    FactQueryInput::new(
        StoreScopeRef::new(&config.store_scope).map_err(|error| BtcStateError::InvalidInput {
            reason: error.to_string(),
        })?,
        FactQueryScope::new(FactAudience::Control, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(checkpoint_scope_decision_hash()),
        vec![
            query_predicate(
                "subject.collector_kind",
                FactCanonicalScalar::string(&config.collector_kind),
            )?,
            query_predicate(
                "subject.network",
                FactCanonicalScalar::string(&config.network),
            )?,
            query_predicate(
                "subject.bitcoin_network",
                FactCanonicalScalar::string(&config.bitcoin_network),
            )?,
            query_predicate(
                "subject.partition",
                FactCanonicalScalar::string(&config.partition),
            )?,
            query_predicate("subject.scope", FactCanonicalScalar::string(DEFAULT_SCOPE))?,
            query_predicate(
                "subject.semantic_source_identity",
                FactCanonicalScalar::string(&config.semantic_source_identity),
            )?,
        ],
        vec![query_return_field("result.high_watermark_height")?],
        FactOrderingName::new("result.high_watermark_height.desc").map_err(|error| {
            BtcStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?,
        Some(1),
    )
    .map_err(|error| BtcStateError::InvalidInput {
        reason: error.to_string(),
    })
}

fn query_return_field(field_id: &str) -> Result<FactFieldId, BtcStateError> {
    FactFieldId::new(field_id).map_err(|error| BtcStateError::InvalidInput {
        reason: error.to_string(),
    })
}

fn query_predicate(
    field_id: &str,
    value: FactCanonicalScalar,
) -> Result<FactQueryPredicate, BtcStateError> {
    let field_id = FactFieldId::new(field_id).map_err(|error| BtcStateError::InvalidInput {
        reason: error.to_string(),
    })?;
    Ok(FactQueryPredicate::new(
        field_id,
        FactQueryOperator::Equal,
        value,
    ))
}

fn validate_collector_checkpoint_common(
    collector_kind: &str,
    semantic_source_identity: &str,
    partition: &str,
    network: &str,
    bitcoin_network: &str,
) -> Result<(), String> {
    validate_non_secret_label("collector_kind", collector_kind)?;
    validate_non_secret_label("partition", partition)?;
    BtcSourceIdentity::new(semantic_source_identity).map_err(|error| error.to_string())?;
    BtcNetworkId::new(network).map_err(|error| error.to_string())?;
    validate_bitcoin_network(bitcoin_network)?;
    Ok(())
}

fn checkpoint_scope_decision_hash() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.bitcoin.collector-checkpoint.scope.default.v1"),
    )
}

fn selection_policy_hash() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(CHECKPOINT_QUERY_SELECTION_POLICY),
    )
}
