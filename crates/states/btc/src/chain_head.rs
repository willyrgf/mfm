use super::*;

/// Subject identity for a Bitcoin chain-head platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_subject",
    version = "1",
    schema = "mfm.bitcoin.fact.chain_head.subject"
)]
pub struct BtcChainHeadSubject {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    head_kind: String,
}

impl BtcChainHeadSubject {
    /// Creates chain-head subject material from a verified capability response.
    pub fn from_capability_response(response: &CapabilityChainHeadResponse) -> Self {
        let evidence = &response.evidence;
        Self {
            network: evidence.network_id.as_str().to_owned(),
            bitcoin_network: evidence.bitcoin_network.clone(),
            semantic_source_identity: evidence.source_identity.as_str().to_owned(),
            head_kind: head_kind_tag(response.head_kind).to_owned(),
        }
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the Bitcoin Core network tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }

    /// Returns the non-secret semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the head-kind tag.
    pub fn head_kind(&self) -> &str {
        &self.head_kind
    }
}

/// Observed result for a Bitcoin chain-head platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_response",
    version = "1",
    schema = "mfm.bitcoin.fact.chain_head.response"
)]
pub struct BtcChainHeadResponse {
    block_height: u64,
    block_hash: String,
    observed_source_status: String,
    observed_bitcoin_network: String,
    finality_policy: String,
    confirmation_depth: Option<u64>,
    provider_time_unix_ms: Option<u64>,
    observed_at_unix_ms: Option<u64>,
}

impl BtcChainHeadResponse {
    /// Creates fact response material from a verified capability response.
    pub fn from_capability_response(
        response: &CapabilityChainHeadResponse,
        observed_at_unix_ms: Option<u64>,
    ) -> Self {
        Self {
            block_height: response.block_height,
            block_hash: response.block_hash.as_str().to_owned(),
            observed_source_status: source_status_tag(response.evidence.source_status).to_owned(),
            observed_bitcoin_network: response.evidence.observed_bitcoin_network.clone(),
            finality_policy: finality_policy_tag(response.finality).to_owned(),
            confirmation_depth: response.finality.confirmation_depth(),
            provider_time_unix_ms: response.provider_time_unix_ms,
            observed_at_unix_ms,
        }
    }

    /// Returns the observed block height.
    pub const fn block_height(&self) -> u64 {
        self.block_height
    }

    /// Returns the observed block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the observed source synchronization status tag.
    pub fn observed_source_status(&self) -> &str {
        &self.observed_source_status
    }

    /// Returns the provider-observed Bitcoin network tag.
    pub fn observed_bitcoin_network(&self) -> &str {
        &self.observed_bitcoin_network
    }

    /// Returns the finality policy tag attached to the observation.
    pub fn finality_policy(&self) -> &str {
        &self.finality_policy
    }

    /// Returns the confirmation depth attached to the observation, when any.
    pub const fn confirmation_depth(&self) -> Option<u64> {
        self.confirmation_depth
    }

    /// Returns the provider-reported block time in Unix milliseconds.
    pub const fn provider_time_unix_ms(&self) -> Option<u64> {
        self.provider_time_unix_ms
    }

    /// Returns the state observation time in Unix milliseconds.
    pub const fn observed_at_unix_ms(&self) -> Option<u64> {
        self.observed_at_unix_ms
    }
}

/// Platform fact for a Bitcoin chain-head observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_fact",
    version = "1",
    schema = "mfm.bitcoin.fact.chain_head"
)]
#[mfm_fact(kind = "chain.head")]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.bitcoin_network",
    source = "subject",
    path = "bitcoin_network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.semantic_source_identity",
    source = "subject",
    path = "semantic_source_identity",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.head_kind",
    source = "subject",
    path = "head_kind",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.block_height",
    source = "result",
    path = "block_height",
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
    id = "result.block_hash",
    source = "result",
    path = "block_hash",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.observed_source_status",
    source = "result",
    path = "observed_source_status",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.observed_at_unix_ms",
    source = "result",
    path = "observed_at_unix_ms",
    value_type = "unsigned_integer",
    operators(equal, greater_than_or_equal, less_than_or_equal),
    exposure = "query_only",
    optional,
    sortable
))]
#[mfm_fact(field(
    id = "metadata.observed_at",
    source = "metadata",
    metadata = "observed_at",
    value_type = "timestamp",
    operators(equal, greater_than_or_equal, less_than_or_equal),
    exposure = "query_only",
    optional,
    sortable
))]
#[mfm_fact(ordering(
    name = "result.block_height.desc",
    term(
        field = "result.block_height",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct BtcChainHeadFact {
    subject: BtcChainHeadSubject,
    response: BtcChainHeadResponse,
}

impl BtcChainHeadFact {
    /// Creates a chain-head fact from subject and response material.
    pub const fn new(subject: BtcChainHeadSubject, response: BtcChainHeadResponse) -> Self {
        Self { subject, response }
    }

    /// Returns the chain-head subject material.
    pub const fn subject(&self) -> &BtcChainHeadSubject {
        &self.subject
    }

    /// Returns the chain-head response material.
    pub const fn response(&self) -> &BtcChainHeadResponse {
        &self.response
    }
}

/// Bounded config for a Bitcoin chain-head observation state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.observe_chain_head",
    validate = "validate_observe_chain_head_config"
)]
pub struct ObserveBtcChainHeadConfig {
    /// Semantic network id.
    pub network: String,
    /// Expected Bitcoin Core network tag (`main`, `test`, `signet`, or `regtest`).
    pub bitcoin_network: String,
    /// Non-secret semantic source identity.
    pub semantic_source_identity: String,
    /// Head kind requested by the state.
    pub head_kind: String,
    /// Optional confirmation depth for confirmed-head reads.
    pub confirmation_depth: Option<NonZeroU64>,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates chain-head observation config.
pub fn validate_observe_chain_head_config(
    config: &ObserveBtcChainHeadConfig,
) -> Result<(), String> {
    BtcNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    validate_bitcoin_network(&config.bitcoin_network)?;
    BtcSourceIdentity::new(&config.semantic_source_identity).map_err(|error| error.to_string())?;
    match config.head_kind.as_str() {
        "best" if config.confirmation_depth.is_none() => Ok(()),
        "confirmed" if config.confirmation_depth.is_some() => Ok(()),
        "best" => Err("best head observations must not set confirmation_depth".to_owned()),
        "confirmed" => Err("confirmed head observations require confirmation_depth".to_owned()),
        _ => Err("head_kind must be `best` or `confirmed`".to_owned()),
    }
}

impl ObserveBtcChainHeadConfig {
    /// Returns the semantic head selection requested by this bounded observation.
    pub fn selection(&self) -> Result<BtcHeadSelection, BtcStateError> {
        validate_observe_chain_head_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let selection = match self.head_kind.as_str() {
            "best" => BtcHeadSelection::best(),
            "confirmed" => BtcHeadSelection::confirmed(
                self.confirmation_depth
                    .expect("validated confirmation depth")
                    .get(),
            )?,
            _ => {
                return Err(BtcStateError::InvalidInput {
                    reason: "head_kind must be `best` or `confirmed`".to_owned(),
                })
            }
        };
        Ok(selection)
    }
}

/// Input for a bounded Bitcoin chain-head observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.observe_chain_head")]
pub struct ObserveBtcChainHeadInput {
    /// Loaded collector checkpoint selected by the upstream checkpoint query.
    pub loaded_checkpoint: LoadedCollectorCheckpoint,
    /// Optional observation context supplied by the adapter.
    pub context: BtcChainHeadObservationContext,
}

/// Adapter-supplied context for a bounded Bitcoin chain-head observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_observation_context",
    version = "1",
    schema = "mfm.bitcoin.state.value.chain_head_observation_context"
)]
pub struct BtcChainHeadObservationContext {
    /// State observation time in Unix milliseconds, when supplied by an adapter.
    pub observed_at_unix_ms: Option<u64>,
}

/// Normalized chain-head observation state output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_observation",
    version = "1",
    schema = "mfm.bitcoin.state.output.chain_head_observation"
)]
pub struct BtcChainHeadObservation {
    subject: BtcChainHeadSubject,
    response: BtcChainHeadResponse,
    source_read_count: u64,
}

impl BtcChainHeadObservation {
    /// Creates normalized observation output.
    pub const fn new(
        subject: BtcChainHeadSubject,
        response: BtcChainHeadResponse,
        source_read_count: u64,
    ) -> Self {
        Self {
            subject,
            response,
            source_read_count,
        }
    }

    /// Returns the normalized chain-head subject.
    pub const fn subject(&self) -> &BtcChainHeadSubject {
        &self.subject
    }

    /// Returns the normalized chain-head response.
    pub const fn response(&self) -> &BtcChainHeadResponse {
        &self.response
    }

    /// Builds the platform fact from this whole observation output.
    pub fn to_fact(&self) -> BtcChainHeadFact {
        BtcChainHeadFact::new(self.subject.clone(), self.response.clone())
    }

    /// Converts this whole observation output into the platform fact.
    pub fn into_fact(self) -> BtcChainHeadFact {
        BtcChainHeadFact::new(self.subject, self.response)
    }

    /// Returns the number of source reads used by this bounded observation.
    pub const fn source_read_count(&self) -> u64 {
        self.source_read_count
    }
}

/// Normalizes a verified Bitcoin capability response into a chain-head observation output.
pub fn normalize_chain_head_response(
    config: &ObserveBtcChainHeadConfig,
    response: &CapabilityChainHeadResponse,
    input: &ObserveBtcChainHeadInput,
) -> Result<BtcChainHeadObservation, BtcStateError> {
    let selection = config.selection()?;
    validate_loaded_checkpoint_for_config(config, selection, &input.loaded_checkpoint)?;
    let observation = BtcChainHeadObservation::new(
        BtcChainHeadSubject::from_capability_response(response),
        BtcChainHeadResponse::from_capability_response(response, input.context.observed_at_unix_ms),
        1,
    );
    reject_observation_behind_checkpoint(&observation, &input.loaded_checkpoint)?;
    Ok(observation)
}

/// State contract for a bounded Bitcoin chain-head observation.
pub struct ObserveBtcChainHeadState {
    config: ObserveBtcChainHeadConfig,
}

impl ObserveBtcChainHeadState {
    /// Materializes a normalized observation from a verified capability response.
    pub fn materialize_response(
        &self,
        input: &ObserveBtcChainHeadInput,
        response: &CapabilityChainHeadResponse,
    ) -> Result<BtcChainHeadObservation, BtcStateError> {
        normalize_chain_head_response(&self.config, response, input)
    }

    /// Returns the certified state config.
    pub const fn config(&self) -> &ObserveBtcChainHeadConfig {
        &self.config
    }
}

impl StateSpec for ObserveBtcChainHeadState {
    type Config = ObserveBtcChainHeadConfig;
    type Context = NoContext;
    type Input = ObserveBtcChainHeadInput;
    type Output = BtcChainHeadObservation;
    type Effect = ReadExternal;
    type Caps = (BtcChainHeadReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("chain_head.observe")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("chain_head.observe")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.chain_head.observe"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for ObserveBtcChainHeadState {
    type Plan = BtcChainHeadReadPlan;
    type Evidence = BtcChainHeadReadEvidence;

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        let selection = self.config.selection()?;
        validate_loaded_checkpoint_for_config(&self.config, selection, &input.loaded_checkpoint)?;
        BtcChainHeadReadPlan::new(
            &self.config.network,
            &self.config.bitcoin_network,
            &self.config.semantic_source_identity,
            selection,
        )
        .map_err(StateError::from)
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        if !evidence.fact_query_evidence().is_empty() {
            return Err(StateError::Message(
                "Bitcoin chain-head read received unexpected fact-query evidence".to_owned(),
            ));
        }
        let plan = self.plan(input, context)?;
        let response = evidence.primary_evidence().response(&plan)?;
        self.materialize_response(input, &response)
            .map_err(StateError::from)
    }
}

/// Config for a chain-head fact recording state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.bitcoin.state.config.record_chain_head_fact")]
pub struct RecordBtcChainHeadFactConfig {}

/// Input for recording a Bitcoin chain-head fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.record_chain_head_fact")]
pub struct RecordBtcChainHeadFactInput {
    /// Whole chain-head observation output to transform into a fact.
    pub observation: BtcChainHeadObservation,
}

/// State contract for recording a Bitcoin chain-head fact.
pub struct RecordBtcChainHeadFactState;

impl StateSpec for RecordBtcChainHeadFactState {
    type Config = RecordBtcChainHeadFactConfig;
    type Context = NoContext;
    type Input = RecordBtcChainHeadFactInput;
    type Output = BtcChainHeadFact;
    type Effect = ManagedPlatformWrite;
    type Caps = (FactRecordCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("chain_head.record")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("chain_head.record")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.chain_head.record"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<BtcChainHeadFact>()?])
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl ManagedWriteState for RecordBtcChainHeadFactState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Ok(input.observation.into_fact()))
    }
}

fn validate_loaded_checkpoint_for_config(
    config: &ObserveBtcChainHeadConfig,
    selection: BtcHeadSelection,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<(), BtcStateError> {
    let Some(checkpoint) = loaded_checkpoint.checkpoint() else {
        return Ok(());
    };
    let subject = checkpoint.subject();
    let response = checkpoint.response();
    if subject.network() != config.network.as_str()
        || subject.bitcoin_network() != config.bitcoin_network.as_str()
        || subject.semantic_source_identity() != config.semantic_source_identity.as_str()
        || subject.scope() != DEFAULT_SCOPE
        || response.finality_policy() != finality_policy_tag(selection.finality())
        || response.confirmation_depth() != selection.finality().confirmation_depth()
    {
        return Err(BtcStateError::InvalidInput {
            reason: "loaded checkpoint is incompatible with requested Bitcoin source".to_owned(),
        });
    }
    Ok(())
}

fn reject_observation_behind_checkpoint(
    observation: &BtcChainHeadObservation,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<(), BtcStateError> {
    if let Some(previous) = loaded_checkpoint.checkpoint() {
        if observation.response().block_height() < previous.response().high_watermark_height() {
            return Err(BtcStateError::InvalidInput {
                reason: "observed chain head must not be behind loaded checkpoint".to_owned(),
            });
        }
    }
    Ok(())
}
