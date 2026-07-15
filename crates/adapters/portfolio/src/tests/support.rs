use super::*;
use std::collections::BTreeMap;

pub(super) const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
pub(super) const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
pub(super) const TOKEN: &str = "0x0000000000000000000000000000000000000001";
pub(super) const BTC_HASH: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(super) const EVM_HASH: &str =
    "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Clone)]
pub(super) struct FactFixture {
    pub(super) bytes: Vec<u8>,
    pub(super) evidence: store::ArtifactEvidenceRef,
    pub(super) fact_ref: InternalFactRef,
    pub(super) identity: mfm_facts::FactContentIdentity,
    subject_namespace_hash: ContentDigest,
    fact_key: mfm_facts::FactKey,
    fact_kind: String,
}

pub(super) fn btc_fixture(response: BtcAddressBalanceResponse, source_seq: u64) -> FactFixture {
    let subject = BtcAddressBalanceSubject::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        BTC_ADDRESS,
    )
    .expect("Bitcoin subject");
    let descriptor = BtcAddressBalanceSnapshotFact::descriptor().expect("Bitcoin descriptor");
    typed_fixture(
        "bitcoin.address_balance_snapshot",
        &descriptor,
        &subject,
        &response,
        source_seq,
    )
}

pub(super) fn evm_native_fixture(
    account: &str,
    response: EvmAddressNativeBalanceResponse,
    source_seq: u64,
) -> FactFixture {
    let subject = EvmAddressNativeBalanceSubject::new("ethereum-mainnet", 1, account)
        .expect("EVM native subject");
    let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().expect("native descriptor");
    typed_fixture(
        "evm.address_native_balance_snapshot",
        &descriptor,
        &subject,
        &response,
        source_seq,
    )
}

pub(super) fn evm_erc20_fixture(
    account: &str,
    contract: &str,
    response: EvmAddressErc20BalanceResponse,
    source_seq: u64,
) -> FactFixture {
    let subject = EvmAddressErc20BalanceSubject::new("ethereum-mainnet", 1, contract, account)
        .expect("EVM ERC-20 subject");
    let descriptor = EvmAddressErc20BalanceSnapshotFact::descriptor().expect("ERC-20 descriptor");
    typed_fixture(
        "evm.address_erc20_balance_snapshot",
        &descriptor,
        &subject,
        &response,
        source_seq,
    )
}

fn typed_fixture<S, R>(
    fact_kind: &str,
    descriptor: &mfm_facts::FactDescriptor,
    subject: &S,
    response: &R,
    source_seq: u64,
) -> FactFixture
where
    S: serde::Serialize,
    R: serde::Serialize,
{
    let identity = derive_fact_content_identity_from_typed_values(descriptor, subject, response)
        .expect("typed fact identity");
    let subject_value = serde_json::to_value(subject).expect("subject value");
    let subject_evidence =
        typed_fact_subject_evidence(descriptor, &subject_value).expect("subject evidence");
    let json = serde_json::to_string(response).expect("response json");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical response");
    assert_eq!(
        canonical.content_digest(),
        *identity.response_hash(),
        "fixture response bytes must be the checked identity payload"
    );
    let artifact_id = ArtifactId::from_digest(
        identity.response_hash().algorithm(),
        *identity.response_hash().digest(),
    );
    let producer_node_id = fixture_producer_node_id();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: identity.response_hash().clone(),
        byte_len: canonical.as_bytes().len() as u64,
        media_type: mfm_spec::v1::MediaType::new("application/json").expect("media type"),
        schema_id: Some(identity.response_schema_id().clone()),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    let fact_ref = InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(0x44), source_seq, 0).expect("fact claim id"),
        source_event_id: EventId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(source_seq as u8),
        ),
        recorded_at: "2026-07-15T00:00:00Z".to_owned(),
        producer_node_id,
        observed_at: Some("2026-07-15T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new(fact_kind).expect("fact kind"),
        fact_descriptor_hash: identity.fact_descriptor_hash().clone(),
        subject: FactSubjectRef::from_evidence(&subject_evidence),
        request: None,
        response: FactResponseEvidence::new(
            identity.response_schema_id().clone(),
            identity.response_hash().clone(),
            artifact_id,
            evidence.evidence_hash().expect("artifact evidence hash"),
        ),
        producer: fixture_provenance(),
    })
    .expect("internal fact ref");
    FactFixture {
        bytes: canonical.as_bytes().to_vec(),
        evidence,
        fact_ref,
        identity,
        subject_namespace_hash: subject_evidence.fact_subject_namespace_hash().clone(),
        fact_key: subject_evidence.fact_key().clone(),
        fact_kind: fact_kind.to_owned(),
    }
}

/// Builds a fact reference with one checked identity component deliberately substituted.
pub(super) fn forged_reference(
    fixture: &FactFixture,
    fact_descriptor_hash: ContentDigest,
    subject_material_hash: ContentDigest,
    response_schema_id: mfm_ids::SchemaId,
    response_hash: ContentDigest,
    source_seq: u64,
) -> InternalFactRef {
    let artifact_id = ArtifactId::from_digest(response_hash.algorithm(), *response_hash.digest());
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: response_hash.clone(),
        byte_len: fixture.bytes.len() as u64,
        media_type: mfm_spec::v1::MediaType::new("application/json").expect("media type"),
        schema_id: Some(response_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(fixture_producer_node_id()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(0x45), source_seq, 0).expect("fact claim id"),
        source_event_id: EventId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(source_seq as u8),
        ),
        recorded_at: "2026-07-15T00:00:00Z".to_owned(),
        producer_node_id: fixture_producer_node_id(),
        observed_at: Some("2026-07-15T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new(&fixture.fact_kind).expect("fact kind"),
        fact_descriptor_hash,
        subject: FactSubjectRef::new(
            fixture.subject_namespace_hash.clone(),
            fixture.fact_key.clone(),
            subject_material_hash,
        ),
        request: None,
        response: FactResponseEvidence::new(
            response_schema_id,
            response_hash,
            artifact_id,
            evidence.evidence_hash().expect("artifact evidence hash"),
        ),
        producer: fixture_provenance(),
    })
    .expect("forged internal fact ref")
}

pub(super) fn dual_mainnet_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "dual-mainnet".parse().expect("portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![
            NetworkConfig::new(
                "ethereum-mainnet".to_owned(),
                NetworkFamilyConfig::Evm,
                Some(1),
                Some(18),
                None,
                None,
                BTreeMap::new(),
            )
            .expect("EVM network"),
            NetworkConfig::new(
                "bitcoin-mainnet".to_owned(),
                NetworkFamilyConfig::Bitcoin,
                None,
                None,
                Some("main".to_owned()),
                Some("public-bitcoin-core".to_owned()),
                BTreeMap::new(),
            )
            .expect("Bitcoin network"),
        ],
        wallets: vec![
            wallet(
                "wallet_eth",
                EVM_ACCOUNT,
                WalletSubjectKind::EvmAddress,
                "ethereum-mainnet",
                vec!["eth.native.ethereum-mainnet"],
            ),
            wallet(
                "wallet_btc",
                BTC_ADDRESS,
                WalletSubjectKind::BitcoinAddress,
                "bitcoin-mainnet",
                vec!["btc.native.bitcoin-mainnet"],
            ),
        ],
        symbol_configs: vec![
            native_symbol(
                "eth.native.ethereum-mainnet",
                "ethereum-mainnet",
                "ETH",
                "2.5",
            ),
            native_symbol("btc.native.bitcoin-mainnet", "bitcoin-mainnet", "BTC", "1"),
        ],
        metadata: PublicMetadata::default(),
    }
    .normalized()
}

pub(super) fn evm_native_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "evm-native".parse().expect("portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![evm_network()],
        wallets: vec![wallet(
            "wallet_eth",
            EVM_ACCOUNT,
            WalletSubjectKind::EvmAddress,
            "ethereum-mainnet",
            vec!["eth.native.ethereum-mainnet"],
        )],
        symbol_configs: vec![native_symbol(
            "eth.native.ethereum-mainnet",
            "ethereum-mainnet",
            "ETH",
            "1",
        )],
        metadata: PublicMetadata::default(),
    }
    .normalized()
}

pub(super) fn evm_native_and_erc20_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "evm-native-and-token".parse().expect("portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![evm_network()],
        wallets: vec![wallet(
            "wallet_eth",
            EVM_ACCOUNT,
            WalletSubjectKind::EvmAddress,
            "ethereum-mainnet",
            vec![
                "eth.native.ethereum-mainnet",
                "usdc.wallet.ethereum-mainnet",
            ],
        )],
        symbol_configs: vec![
            native_symbol(
                "eth.native.ethereum-mainnet",
                "ethereum-mainnet",
                "ETH",
                "1",
            ),
            erc20_symbol("usdc.wallet.ethereum-mainnet", "USDC", "1"),
        ],
        metadata: PublicMetadata::default(),
    }
    .normalized()
}

pub(super) fn select_input_for_dual_mainnet(
    btc: &FactFixture,
    evm: &FactFixture,
) -> SelectHoldingsInput {
    SelectHoldingsInput {
        receipt: collection_receipt(vec![
            receipt_part(
                "wallet_btc",
                "btc.native.bitcoin-mainnet",
                "bitcoin-mainnet",
                HoldingSourceKey::BitcoinNative {
                    network_id: "bitcoin-mainnet".to_owned(),
                    bitcoin_network: "main".to_owned(),
                    semantic_source_identity: "public-bitcoin-core".to_owned(),
                    address: BTC_ADDRESS.to_owned(),
                },
                ExecutionAnchor::Bitcoin {
                    height: 850_000,
                    block_hash: BTC_HASH.to_owned(),
                },
                "configured_only",
                btc,
            ),
            receipt_part(
                "wallet_eth",
                "eth.native.ethereum-mainnet",
                "ethereum-mainnet",
                HoldingSourceKey::EvmNative {
                    network_id: "ethereum-mainnet".to_owned(),
                    chain_id: 1,
                    account: EVM_ACCOUNT.to_owned(),
                },
                ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 21_000_000,
                    block_hash: EVM_HASH.to_owned(),
                },
                "configured_only",
                evm,
            ),
        ]),
    }
}

pub(super) fn select_input_for_evm_native(
    fixture: &FactFixture,
    block_number: u64,
    block_hash: &str,
) -> SelectHoldingsInput {
    SelectHoldingsInput {
        receipt: collection_receipt(vec![receipt_part(
            "wallet_eth",
            "eth.native.ethereum-mainnet",
            "ethereum-mainnet",
            HoldingSourceKey::EvmNative {
                network_id: "ethereum-mainnet".to_owned(),
                chain_id: 1,
                account: EVM_ACCOUNT.to_owned(),
            },
            ExecutionAnchor::Evm {
                chain_id: 1,
                block_number,
                block_hash: block_hash.to_owned(),
            },
            "configured_only",
            fixture,
        )]),
    }
}

pub(super) fn select_input_for_evm_native_and_erc20(
    native: &FactFixture,
    token: &FactFixture,
    block_number: u64,
    block_hash: &str,
) -> SelectHoldingsInput {
    SelectHoldingsInput {
        receipt: collection_receipt(vec![
            receipt_part(
                "wallet_eth",
                "eth.native.ethereum-mainnet",
                "ethereum-mainnet",
                HoldingSourceKey::EvmNative {
                    network_id: "ethereum-mainnet".to_owned(),
                    chain_id: 1,
                    account: EVM_ACCOUNT.to_owned(),
                },
                ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number,
                    block_hash: block_hash.to_owned(),
                },
                "configured_only",
                native,
            ),
            receipt_part(
                "wallet_eth",
                "usdc.wallet.ethereum-mainnet",
                "ethereum-mainnet",
                HoldingSourceKey::EvmErc20 {
                    network_id: "ethereum-mainnet".to_owned(),
                    chain_id: 1,
                    contract_address: TOKEN.to_owned(),
                    account: EVM_ACCOUNT.to_owned(),
                },
                ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number,
                    block_hash: block_hash.to_owned(),
                },
                "complete_at_anchor",
                token,
            ),
        ]),
    }
}

struct ReceiptPart<'a> {
    requirement: HoldingRequirementKey,
    source: HoldingSourceKey,
    anchor: ExecutionAnchor,
    coverage: &'static str,
    fixture: &'a FactFixture,
}

fn receipt_part<'a>(
    wallet_id: &str,
    symbol_id: &str,
    network_id: &str,
    source: HoldingSourceKey,
    anchor: ExecutionAnchor,
    coverage: &'static str,
    fixture: &'a FactFixture,
) -> ReceiptPart<'a> {
    ReceiptPart {
        requirement: HoldingRequirementKey {
            wallet_id: wallet_id.to_owned(),
            symbol_id: symbol_id.to_owned(),
            network_id: network_id.to_owned(),
        },
        source,
        anchor,
        coverage,
        fixture,
    }
}

fn collection_receipt(parts: Vec<ReceiptPart<'_>>) -> PortfolioCollectionReceipt {
    let mut manifest = parts
        .iter()
        .map(|part| {
            HoldingManifestEntry::new(part.requirement.clone(), part.source.clone())
                .expect("receipt manifest entry")
        })
        .collect::<Vec<_>>();
    manifest.sort_by(|left, right| left.requirement().cmp(right.requirement()));
    let holdings = parts
        .iter()
        .map(|part| {
            CollectedHoldingReceipt::new(
                part.requirement.clone(),
                part.source.clone(),
                part.anchor.clone(),
                part.coverage.to_owned(),
                "ok".to_owned(),
                mfm_facts::FactContentIdentityEvidence::from_verified(&part.fixture.identity),
            )
            .expect("receipt holding")
        })
        .collect::<Vec<_>>();
    let mut anchors = BTreeMap::new();
    for part in &parts {
        match anchors.get(&part.requirement.network_id) {
            Some(existing) => assert_eq!(
                existing, &part.anchor,
                "all logical sources on one network must share its receipt anchor"
            ),
            None => {
                anchors.insert(part.requirement.network_id.clone(), part.anchor.clone());
            }
        }
    }
    let network_anchors = anchors
        .into_iter()
        .map(|(network_id, anchor)| NetworkPin { network_id, anchor })
        .collect();
    PortfolioCollectionReceipt::new(&manifest, holdings, network_anchors).expect("receipt")
}

pub(super) struct MockFactIndex {
    calls: Mutex<usize>,
    rows_by_source: HashMap<String, Vec<(InternalFactRef, u64)>>,
    limit: Option<u64>,
    mixed_frontiers: bool,
}

impl MockFactIndex {
    pub(super) fn with_rows(rows_by_source: HashMap<String, Vec<(InternalFactRef, u64)>>) -> Self {
        Self {
            calls: Mutex::new(0),
            rows_by_source,
            limit: Some(11),
            mixed_frontiers: false,
        }
    }

    pub(super) fn with_saturated_rows(
        rows_by_source: HashMap<String, Vec<(InternalFactRef, u64)>>,
    ) -> Self {
        Self::with_rows(rows_by_source)
    }

    pub(super) fn with_mixed_frontiers(
        rows_by_source: HashMap<String, Vec<(InternalFactRef, u64)>>,
    ) -> Self {
        Self {
            mixed_frontiers: true,
            ..Self::with_rows(rows_by_source)
        }
    }

    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().expect("calls")
    }
}

impl FactIndexReadProvider for MockFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.adapters.portfolio.test.fact-index.v2"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> mfm_fact_capabilities::FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            *self.calls.lock().expect("calls") += 1;
            let mut responses = Vec::with_capacity(requests.len());
            for (index, request) in requests.iter().enumerate() {
                let rows_with_order = self
                    .rows_by_source
                    .get(&plan_source_key(request.plan()))
                    .cloned()
                    .unwrap_or_default();
                let rows = rows_with_order
                    .into_iter()
                    .map(|(fact_ref, order)| {
                        FactQueryResultRow::new(
                            fact_ref,
                            vec![mfm_facts::FactFieldValue::new(
                                mfm_facts::FactFieldId::new("metadata.store_commit_order")
                                    .expect("field"),
                                FactFieldValueType::UnsignedInteger,
                                FactCanonicalScalar::UnsignedInteger(order),
                            )
                            .expect("field value")],
                        )
                    })
                    .collect::<Vec<_>>();
                let receipt = signed_receipt_for_rows(
                    &rows,
                    StoreCommitOrder::new(if self.mixed_frontiers {
                        11 + index as u64
                    } else {
                        11
                    }),
                    self.limit,
                );
                responses.push(
                    mfm_facts::FactQueryResult::new(
                        mfm_facts::fact_query_result_rows_from_receipt(&receipt),
                        receipt,
                    )
                    .expect("fact query result"),
                );
            }
            Ok(responses)
        })
    }
}

pub(super) struct MockArtifacts {
    by_id: HashMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>,
    reads: Mutex<usize>,
}

impl MockArtifacts {
    pub(super) fn with_fixtures(fixtures: &[FactFixture]) -> Self {
        let by_id = fixtures
            .iter()
            .map(|fixture| {
                (
                    fixture.fact_ref.artifact_id().clone(),
                    (fixture.bytes.clone(), fixture.evidence.clone()),
                )
            })
            .collect();
        Self {
            by_id,
            reads: Mutex::new(0),
        }
    }

    pub(super) fn reads(&self) -> usize {
        *self.reads.lock().expect("artifact reads")
    }
}

impl store::RetainedArtifactReadProvider for MockArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            *self.reads.lock().expect("artifact reads") += 1;
            let Some((bytes, evidence)) = self.by_id.get(&requirement.artifact_id) else {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            store::VerifiedRunArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
        })
    }
}

pub(super) fn plan_source_key(plan: &mfm_facts::CanonicalFactQueryPlan) -> String {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).expect("query shape");
    let subject = shape
        .predicates()
        .iter()
        .find_map(|predicate| {
            matches!(
                predicate.field_id().as_str(),
                "subject.address" | "subject.account"
            )
            .then(|| match predicate.value() {
                FactCanonicalScalar::String(value) => value.as_str().to_owned(),
                _ => panic!("address predicate must be string"),
            })
        })
        .expect("source subject predicate");
    format!("{}:{subject}", plan_fact_kind(plan))
}

pub(super) fn plan_fact_kind(plan: &mfm_facts::CanonicalFactQueryPlan) -> String {
    let value: serde_json::Value =
        serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query JSON");
    value["fact_kind"].as_str().expect("fact kind").to_owned()
}

fn signed_receipt_for_rows(
    rows: &[FactQueryResultRow],
    store_commit_order: StoreCommitOrder,
    limit: Option<u64>,
) -> FactQueryReceipt {
    let read_frontier = StoreReadFrontier::new(
        StoreScopeRef::new("mfm.store.default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        mfm_facts::DescriptorCatalogWatermark::new(1),
        store_commit_order,
    );
    fact_query_receipt_for_test(FactQueryReceiptFixtureInputForTest {
        read_frontier,
        rows,
        include_returned_field_summaries: true,
        limit,
    })
}

fn evm_network() -> NetworkConfig {
    NetworkConfig::new(
        "ethereum-mainnet".to_owned(),
        NetworkFamilyConfig::Evm,
        Some(1),
        Some(18),
        None,
        None,
        BTreeMap::new(),
    )
    .expect("EVM network")
}

fn wallet(
    wallet_id: &str,
    address: &str,
    subject_kind: WalletSubjectKind,
    network_id: &str,
    symbols: Vec<&str>,
) -> WalletConfig {
    WalletConfig {
        wallet_id: wallet_id.parse().expect("wallet id"),
        subject: WalletSubject::new(address, subject_kind).expect("wallet subject"),
        network_id: network_id.parse().expect("network id"),
        implementation: WalletImplementationConfig::AddressOnly {},
        symbol_ids: symbols
            .into_iter()
            .map(|symbol| symbol.parse().expect("symbol id"))
            .collect(),
        metadata: PublicMetadata::default(),
    }
}

fn native_symbol(
    symbol_id: &str,
    network_id: &str,
    display_symbol: &str,
    unit_price: &str,
) -> SymbolConfig {
    SymbolConfig {
        symbol_id: symbol_id.parse().expect("symbol id"),
        display_symbol: Some(display_symbol.to_owned()),
        network_id: network_id.parse().expect("network id"),
        source: HoldingSourceConfig::Native,
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: symbol_id.parse().expect("priced symbol"),
                unit_price_dec: unit_price.parse().expect("unit price"),
            }],
        },
        metadata: PublicMetadata::default(),
    }
}

fn erc20_symbol(symbol_id: &str, display_symbol: &str, unit_price: &str) -> SymbolConfig {
    SymbolConfig {
        symbol_id: symbol_id.parse().expect("symbol id"),
        display_symbol: Some(display_symbol.to_owned()),
        network_id: "ethereum-mainnet".parse().expect("network id"),
        source: HoldingSourceConfig::Erc20 {
            contract_address: TOKEN.parse().expect("token address"),
        },
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: symbol_id.parse().expect("priced symbol"),
                unit_price_dec: unit_price.parse().expect("unit price"),
            }],
        },
        metadata: PublicMetadata::default(),
    }
}

fn fixture_producer_node_id() -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x91))
}

fn fixture_provenance() -> FactProducerProvenance {
    FactProducerProvenance::new(
        CapabilityKind::new(
            "mfm.test",
            "portfolio-fact",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0x92),
        )
        .expect("capability kind"),
        CapabilityVersion::new("mfm.test.portfolio-fact.v1").expect("capability version"),
        AdapterKind::new(
            "mfm.test",
            "portfolio-adapter",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(0x93),
        )
        .expect("adapter kind"),
        AdapterVersion::new("mfm.test.portfolio-adapter.v1").expect("adapter version"),
    )
}

fn run_id(seed: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

pub(super) fn digest(seed: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

pub(super) fn digest_bytes(seed: u8) -> DigestBytes {
    DigestBytes::from_array([seed; 32])
}
