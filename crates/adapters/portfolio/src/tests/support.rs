use super::*;

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
            .expect("evm network"),
            NetworkConfig::new(
                "bitcoin-mainnet".to_owned(),
                NetworkFamilyConfig::Bitcoin,
                None,
                None,
                Some("main".to_owned()),
                Some("public-bitcoin-core".to_owned()),
                BTreeMap::new(),
            )
            .expect("btc network"),
        ],
        wallets: vec![
            WalletConfig {
                wallet_id: "wallet_eth".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "0x000000000000000000000000000000000000dead",
                    WalletSubjectKind::EvmAddress,
                )
                .expect("subject"),
                network_id: "ethereum-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
            WalletConfig {
                wallet_id: "wallet_btc".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
                    WalletSubjectKind::BitcoinAddress,
                )
                .expect("subject"),
                network_id: "bitcoin-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["btc.native.bitcoin-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
        ],
        symbol_configs: vec![
            SymbolConfig {
                symbol_id: "eth.native.ethereum-mainnet".parse().expect("symbol"),
                display_symbol: Some("ETH".to_owned()),
                network_id: "ethereum-mainnet".parse().expect("network"),
                source: HoldingSourceConfig::Native,
                valuation: SymbolValuationConfig {
                    quotes: vec![QuoteValuationConfig {
                        quote: QuoteCode::Usd,
                        priced_symbol_id: "eth.native.ethereum-mainnet".parse().expect("priced"),
                        unit_price_dec: "2.5".parse().expect("price"),
                    }],
                },
                metadata: PublicMetadata::default(),
            },
            SymbolConfig {
                symbol_id: "btc.native.bitcoin-mainnet".parse().expect("symbol"),
                display_symbol: Some("BTC".to_owned()),
                network_id: "bitcoin-mainnet".parse().expect("network"),
                source: HoldingSourceConfig::Native,
                valuation: SymbolValuationConfig {
                    quotes: vec![QuoteValuationConfig {
                        quote: QuoteCode::Usd,
                        priced_symbol_id: "btc.native.bitcoin-mainnet".parse().expect("priced"),
                        unit_price_dec: "1".parse().expect("price"),
                    }],
                },
                metadata: PublicMetadata::default(),
            },
        ],
        metadata: PublicMetadata::default(),
    }
}

pub(super) fn dual_wallet_same_network_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "coherent".parse().expect("portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            Some(18),
            None,
            None,
            BTreeMap::new(),
        )
        .expect("evm network")],
        wallets: vec![
            WalletConfig {
                wallet_id: "wallet_a".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "0x000000000000000000000000000000000000000a",
                    WalletSubjectKind::EvmAddress,
                )
                .expect("subject"),
                network_id: "ethereum-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
            WalletConfig {
                wallet_id: "wallet_b".parse().expect("wallet"),
                subject: WalletSubject::new(
                    "0x000000000000000000000000000000000000000b",
                    WalletSubjectKind::EvmAddress,
                )
                .expect("subject"),
                network_id: "ethereum-mainnet".parse().expect("network"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet".parse().expect("symbol")],
                metadata: PublicMetadata::default(),
            },
        ],
        symbol_configs: vec![SymbolConfig {
            symbol_id: "eth.native.ethereum-mainnet".parse().expect("symbol"),
            display_symbol: Some("ETH".to_owned()),
            network_id: "ethereum-mainnet".parse().expect("network"),
            source: HoldingSourceConfig::Native,
            valuation: SymbolValuationConfig {
                quotes: vec![QuoteValuationConfig {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet".parse().expect("priced"),
                    unit_price_dec: "1".parse().expect("price"),
                }],
            },
            metadata: PublicMetadata::default(),
        }],
        metadata: PublicMetadata::default(),
    }
}

pub(super) fn holding_artifact_and_ref<T: serde::Serialize>(
    response: &T,
    fact_kind: &str,
    seed: u8,
    store_commit_order: u64,
) -> (Vec<u8>, store::ArtifactEvidenceRef, InternalFactRef) {
    let _ = store_commit_order; // carried on receipt row, not artifact
    let bytes = serde_json::to_vec(response).expect("response json");
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let producer = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 20));
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: digest.clone(),
        byte_len: bytes.len() as u64,
        media_type: mfm_spec::v1::MediaType::new("application/json").expect("media"),
        schema_id: Some(schema_id(seed + 30)),
        semantic_type_id: None,
        producer_node_id: Some(producer.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    let artifact_evidence_hash = evidence.evidence_hash().expect("evidence hash");
    let descriptor_hash = match fact_kind {
        "bitcoin.address_balance_snapshot" => {
            fact_descriptor_hash(&BtcAddressBalanceSnapshotFact::descriptor().expect("d"))
                .expect("hash")
        }
        "evm.address_native_balance_snapshot" => {
            fact_descriptor_hash(&EvmAddressNativeBalanceSnapshotFact::descriptor().expect("d"))
                .expect("hash")
        }
        other => panic!("unknown fact kind {other}"),
    };
    let fact_ref = InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(seed), u64::from(seed), 0).expect("claim"),
        source_event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 1)),
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        producer_node_id: producer,
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new(fact_kind).expect("kind"),
        fact_descriptor_hash: descriptor_hash,
        subject: FactSubjectRef::new(
            digest_seed(seed + 3),
            mfm_facts::FactKey::from_digest(digest_seed(seed + 4)),
            digest_seed(seed + 5),
        ),
        request: None,
        response: FactResponseEvidence::new(
            schema_id(seed + 30),
            digest,
            artifact_id,
            artifact_evidence_hash,
        ),
        producer: FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.test",
                "fact.record",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 10),
            )
            .expect("cap"),
            CapabilityVersion::new("mfm.test.fact.record.v1").expect("ver"),
            AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 11),
            )
            .expect("adapter"),
            AdapterVersion::new("mfm.test.adapter.v1").expect("adapter ver"),
        ),
    })
    .expect("fact ref");
    (bytes, evidence, fact_ref)
}

pub(super) struct MockFactIndex {
    calls: Mutex<usize>,
    by_kind: HashMap<String, Vec<(InternalFactRef, u64)>>,
    by_account: HashMap<String, Vec<(InternalFactRef, u64)>>,
}

impl MockFactIndex {
    pub(super) fn with_plan_refs(by_kind: HashMap<String, Vec<(InternalFactRef, u64)>>) -> Self {
        Self {
            calls: Mutex::new(0),
            by_kind,
            by_account: HashMap::new(),
        }
    }

    pub(super) fn with_account_refs(
        by_account: HashMap<String, Vec<(InternalFactRef, u64)>>,
    ) -> Self {
        Self {
            calls: Mutex::new(0),
            by_kind: HashMap::new(),
            by_account,
        }
    }

    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().expect("calls")
    }
}

impl FactIndexReadProvider for MockFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.adapters.portfolio.test.fact-index.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> mfm_fact_capabilities::FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            // One batch call counts as one shared-frontier selection read.
            *self.calls.lock().expect("calls") += 1;
            let mut responses = Vec::with_capacity(requests.len());
            for request in requests {
                let plan = request.plan();
                let kind = plan_fact_kind(plan);
                let account = plan_account_predicate(plan);

                // Prefer account-keyed fixtures (same-network multi-wallet tests); fall
                // back to fact-kind keyed fixtures (dual-mainnet BTC+EVM).
                let refs_with_order: Vec<(InternalFactRef, u64)> = account
                    .as_ref()
                    .and_then(|acct| self.by_account.get(acct.as_str()).cloned())
                    .or_else(|| self.by_kind.get(&kind).cloned())
                    .unwrap_or_default();

                let rows: Vec<FactQueryResultRow> = refs_with_order
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
                    .collect();

                let receipt = signed_receipt_for_plan(plan, &rows);
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

/// Test provider that stamps a different store-commit watermark per response.
pub(super) struct MixedFrontierFactIndex {
    pub(super) by_kind: HashMap<String, Vec<(InternalFactRef, u64)>>,
}

impl FactIndexReadProvider for MixedFrontierFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.adapters.portfolio.test.mixed-frontier.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> mfm_fact_capabilities::FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            let mut responses = Vec::with_capacity(requests.len());
            for (index, request) in requests.iter().enumerate() {
                let plan = request.plan();
                let kind = plan_fact_kind(plan);
                let refs_with_order = self.by_kind.get(&kind).cloned().unwrap_or_default();
                let rows: Vec<FactQueryResultRow> = refs_with_order
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
                    .collect();
                let receipt = signed_receipt_for_plan_with_order(
                    plan,
                    &rows,
                    StoreCommitOrder::new(11 + index as u64),
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

pub(super) fn plan_fact_kind(plan: &mfm_facts::CanonicalFactQueryPlan) -> String {
    let value: serde_json::Value =
        serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
    value["fact_kind"].as_str().expect("fact_kind").to_owned()
}

pub(super) fn plan_account_predicate(plan: &mfm_facts::CanonicalFactQueryPlan) -> Option<String> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).expect("query shape");
    for predicate in shape.predicates() {
        if predicate.field_id().as_str() == "subject.account" {
            if let FactCanonicalScalar::String(value) = predicate.value() {
                return Some(value.as_str().to_owned());
            }
        }
    }
    None
}

pub(super) fn signed_receipt_for_plan(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    rows: &[FactQueryResultRow],
) -> FactQueryReceipt {
    signed_receipt_for_plan_with_order(plan, rows, StoreCommitOrder::new(11))
}

pub(super) fn signed_receipt_for_plan_with_order(
    _plan: &mfm_facts::CanonicalFactQueryPlan,
    rows: &[FactQueryResultRow],
    store_commit_order: StoreCommitOrder,
) -> FactQueryReceipt {
    let read_frontier = StoreReadFrontier::new(
        StoreScopeRef::new("mfm.store.default").expect("store scope"),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        DescriptorCatalogWatermark::new(1),
        store_commit_order,
    );
    fact_query_receipt_for_test(FactQueryReceiptFixtureInputForTest {
        read_frontier,
        rows,
        include_returned_field_summaries: true,
        limit: None,
    })
}

pub(super) struct MockArtifacts {
    by_id: HashMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>,
}

impl MockArtifacts {
    pub(super) fn with_map(
        by_id: HashMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>,
    ) -> Self {
        Self { by_id }
    }
}

impl store::RetainedArtifactReadProvider for MockArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let Some((bytes, evidence)) = self.by_id.get(&requirement.artifact_id) else {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            store::VerifiedRunArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
        })
    }
}

pub(super) fn run_id(seed: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

pub(super) fn schema_id(seed: u8) -> SchemaId {
    SchemaId::new(
        "mfm.test.fact_response",
        "v1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(seed),
    )
    .expect("schema")
}

pub(super) fn digest_bytes(seed: u8) -> DigestBytes {
    DigestBytes::from_array([seed; 32])
}

pub(super) fn digest_seed(seed: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}
