//! Sealed target-bound PostgreSQL wallet nonce authority.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Mutex;

use alloy_primitives::{B256, U256};
use mfm_capabilities::{
    AccessFaultCode, EffectAdapterCompletion, EffectCapabilityImplementation,
    ReadAdapterCompletion, ReadCapabilityImplementation,
};
use mfm_evm::{
    canonical_wallet_reference, derive_evm_candidate_operation_key,
    derive_evm_nonce_completion_key, derive_exact_candidate_activation_permit,
    ActivateCandidateResponse, ActivateEvmCandidateRequest, ActivateWalletCandidateCapability,
    ActiveWalletCandidate, CanonicalTerminalOutcome, CompleteEvmNonceRequest,
    CompleteWalletNonceCapability, CompleteWalletNonceResponse, CompletedWalletNonce,
    EvmCandidateFamily, EvmReceiptLookupObservation, EvmSubmissionCapabilityImplementation,
    EvmSubmissionFailure, EvmTransactionIntent, EvmTransactionLookupObservation,
    EvmWalletReference, ExecutionDisposition, QualifiedPendingNonceObservation,
    ReadEvmWalletNonceStatusRequest, ReadWalletNonceStatusCapability, ReserveEvmNonceRequest,
    ReserveWalletNonceCapability, ReserveWalletNonceResponse, ReservedWalletNonce,
    TerminalWitnesses, UnsignedWalletCandidate, WalletNonceAuthority,
    WalletNonceDomainActivationAttestation, WalletNonceDomainActivationRecord, WalletNonceStatus,
    WalletNonceStoreIncarnation, WalletNonceStoreLineageHead,
};
use mfm_ids::{ContentDigest, StableId};
use mfm_journal::structured::{HistoryObject, LexicalValueRef};
use sqlx::{PgConnection, PgPool, Row};

use crate::error::{PostgresEvmWalletError, Result};
use crate::provider::{
    target_session_marker_lock_keys, OfflineActivationVerifier, PendingResolutionLease,
    ProviderDisposition, ProviderMutation, ProviderTargetContext, ReadSnapshotLease,
    WriteTransactionLease,
};
use crate::schema::NONCE_APPLICATION_ROLE;
use crate::support::{
    canonical_json, decode_canonical, evidence_reference, open_role_pool, reference_text,
    session_identity, transaction_id,
};
use crate::WalletAuthorityProviderClient;

/// Opens one sealed authority for an exact qualified physical store incarnation.
///
/// The returned value exposes only the domain port. Its PostgreSQL pool and
/// deployment fence remain private and cannot be recovered by Runtime or an
/// application adapter.
pub async fn open_wallet_nonce_authority(
    database_url: &str,
    expected_schema: &str,
    domain_activation_attestation: WalletNonceDomainActivationAttestation,
    store_incarnation: WalletNonceStoreIncarnation,
    current_public_lineage_head: HistoryObject,
    provider: WalletAuthorityProviderClient,
) -> Result<PostgresWalletNonceAuthority> {
    domain_activation_attestation
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    store_incarnation
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    current_public_lineage_head
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let activation_verifier = provider
        .qualify_domain(
            &domain_activation_attestation,
            &store_incarnation,
            &current_public_lineage_head,
        )
        .await?;
    let current_lineage_head = WalletNonceStoreLineageHead {
        wallet_nonce_store_lineage_id: store_incarnation.wallet_nonce_store_lineage_id.clone(),
        writer_epoch: store_incarnation.writer_epoch,
        public_lineage_head_ref: EvmWalletReference::from_content_ref(
            current_public_lineage_head.content_ref.clone(),
        ),
    };
    current_lineage_head
        .validate()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let validator = EvmSubmissionCapabilityImplementation::new()
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let integrity_fault = AccessFaultCode::new(
        StableId::new("mfm.evm-postgres/wallet-authority-integrity-fault")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
    );
    let entry_unknown_fault = AccessFaultCode::new(
        StableId::new("mfm.evm-postgres/wallet-authority-entry-unknown")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
    );
    let pool = open_role_pool(database_url, expected_schema, NONCE_APPLICATION_ROLE).await?;
    Ok(PostgresWalletNonceAuthority {
        pool,
        schema_name: expected_schema.to_owned(),
        store_incarnation,
        current_lineage_head,
        current_public_lineage_head,
        activation_verifier,
        validator,
        integrity_fault,
        entry_unknown_fault,
        supersession_heads: Mutex::new(BTreeMap::new()),
    })
}

/// Real SQL wallet authority sealed to one deployment-fenced target.
pub struct PostgresWalletNonceAuthority {
    pool: PgPool,
    schema_name: String,
    store_incarnation: WalletNonceStoreIncarnation,
    current_lineage_head: WalletNonceStoreLineageHead,
    current_public_lineage_head: HistoryObject,
    activation_verifier: OfflineActivationVerifier,
    validator: EvmSubmissionCapabilityImplementation,
    integrity_fault: AccessFaultCode,
    entry_unknown_fault: AccessFaultCode,
    supersession_heads: Mutex<BTreeMap<String, HistoryObject>>,
}

impl PostgresWalletNonceAuthority {
    /// Returns the exact physical store incarnation sealed into this authority.
    pub const fn store_incarnation(&self) -> &WalletNonceStoreIncarnation {
        &self.store_incarnation
    }

    /// Returns the current public wallet lineage-head object.
    pub const fn current_public_lineage_head(&self) -> &HistoryObject {
        &self.current_public_lineage_head
    }

    /// Returns the provider fence head that qualified this authority.
    pub const fn provider_fence_head_ref(&self) -> &EvmWalletReference {
        self.activation_verifier.provider_fence_head_ref()
    }

    fn validate_activation(
        &self,
        activation: &WalletNonceDomainActivationAttestation,
    ) -> Result<()> {
        self.activation_verifier.verify(activation).map(|verified| {
            let _ = verified.attestation;
        })
    }

    async fn load_validated_candidates(
        &self,
        connection: &mut PgConnection,
        reservation: &ReservationClosure,
    ) -> Result<Vec<ActiveWalletCandidate>> {
        load_candidates(connection, reservation, &self.validator).await
    }

    async fn load_validated_completion(
        &self,
        connection: &mut PgConnection,
        reservation: &ReservationClosure,
        candidates: &[ActiveWalletCandidate],
    ) -> Result<Option<RetainedCompletion>> {
        let completion = load_completion(
            connection,
            reservation.reservation.semantic_reservation_key.as_str(),
        )
        .await?;
        if completion.as_ref().is_some_and(|retained| {
            !self.retained_completion_is_valid(retained, reservation, candidates)
        }) {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(completion)
    }

    async fn load_validated_domain_aggregate(
        &self,
        connection: &mut PgConnection,
        domain_id: &str,
        retained_activation: Option<&WalletNonceDomainActivationAttestation>,
    ) -> Result<ValidatedDomainAggregate> {
        // The domain row is the sole bounded currentness projection. Historical
        // reservations remain append-only and are never scanned to reconstruct
        // the current state.
        let projection = sqlx::query(
            "SELECT local_high_water_nonce::text AS local_high_water_nonce, \
                    active_reservation_key, current_resource_frontier_ref, \
                    current_incarnation_ref \
             FROM wallet_nonce_domains WHERE wallet_nonce_domain_id = $1",
        )
        .bind(domain_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let Some(projection) = projection else {
            return Ok(ValidatedDomainAggregate {
                high_water: None,
                incomplete_reservation_key: None,
                current_resource_frontier_ref: None,
                current_incarnation_ref: None,
            });
        };
        let retained_high_water = projection
            .try_get::<Option<String>, _>("local_high_water_nonce")
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let incomplete_reservation_key = projection
            .try_get::<Option<String>, _>("active_reservation_key")
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let current_resource_frontier_ref = projection
            .try_get::<Option<String>, _>("current_resource_frontier_ref")
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        let current_incarnation_ref = projection
            .try_get::<Option<String>, _>("current_incarnation_ref")
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;

        let Some(activation) = retained_activation else {
            if retained_high_water.is_some()
                || incomplete_reservation_key.is_some()
                || current_resource_frontier_ref.is_some()
                || current_incarnation_ref.is_some()
            {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
            return Ok(ValidatedDomainAggregate {
                high_water: None,
                incomplete_reservation_key: None,
                current_resource_frontier_ref: None,
                current_incarnation_ref: None,
            });
        };
        self.validate_activation(activation)?;
        if activation
            .current_schema_record
            .wallet_nonce_domain
            .as_str()
            != domain_id
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }

        let Some(retained_high_water) = retained_high_water else {
            if incomplete_reservation_key.is_some() {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
            return Err(PostgresEvmWalletError::InvalidAuthority);
        };
        let high_water = mfm_evm::TransactionNonce::from_str(&retained_high_water)
            .map(mfm_evm::TransactionNonce::get)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if current_resource_frontier_ref.is_none() || current_incarnation_ref.is_none() {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(ValidatedDomainAggregate {
            high_water: Some(high_water),
            incomplete_reservation_key,
            current_resource_frontier_ref,
            current_incarnation_ref,
        })
    }

    async fn load_validated_current_domain(
        &self,
        connection: &mut PgConnection,
        domain_id: &str,
        retained_activation: Option<&WalletNonceDomainActivationAttestation>,
    ) -> Result<ValidatedDomainAggregate> {
        let aggregate = self
            .load_validated_domain_aggregate(connection, domain_id, retained_activation)
            .await?;
        let Some(high_water) = aggregate.high_water else {
            return Ok(aggregate);
        };
        let expected_frontier = reference_text(&self.current_lineage_head.public_lineage_head_ref)?;
        let expected_incarnation = reference_text(
            &canonical_wallet_reference(&self.store_incarnation)
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?,
        )?;
        if aggregate.current_resource_frontier_ref.as_deref() != Some(expected_frontier.as_str())
            || aggregate.current_incarnation_ref.as_deref() != Some(expected_incarnation.as_str())
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let frontier_key = sqlx::query_scalar::<_, String>(
            "SELECT semantic_reservation_key FROM wallet_nonce_reservations \
             WHERE wallet_nonce_domain_id = $1 AND nonce = $2::numeric",
        )
        .bind(domain_id)
        .bind(high_water.to_string())
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?
        .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        let frontier = self
            .load_validated_reservation(connection, &frontier_key)
            .await?
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        if frontier.reservation.nonce != high_water
            || frontier.reservation.nonce_domain.as_str() != domain_id
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        if aggregate
            .incomplete_reservation_key
            .as_deref()
            .is_some_and(|key| key != frontier_key)
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        if aggregate.incomplete_reservation_key.is_some()
            && load_completion(
                connection,
                frontier.reservation.semantic_reservation_key.as_str(),
            )
            .await?
            .is_some()
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let frontier_candidates = self
            .load_validated_candidates(connection, &frontier)
            .await?;
        self.load_validated_completion(connection, &frontier, &frontier_candidates)
            .await?;
        Ok(aggregate)
    }

    fn retained_completion_is_valid(
        &self,
        retained: &RetainedCompletion,
        reservation: &ReservationClosure,
        candidates: &[ActiveWalletCandidate],
    ) -> bool {
        let request = &retained.request;
        let completion = &retained.completion;
        let expected_evidence = evidence_reference(
            "mfm.evm.wallet-completion-evidence.v1",
            &(
                request,
                &retained.state_input_ref,
                &reservation.reservation.resource_lineage_ref,
            ),
        );
        let expected_witness_ref = canonical_wallet_reference(&request.terminal_witnesses);
        EffectCapabilityImplementation::<CompleteWalletNonceCapability>::validate_request(
            &self.validator,
            request,
        )
        .is_ok()
            && completion_closure_matches(request, reservation, candidates)
            && retained.terminal_outcome == request.canonical_terminal_outcome
            && completion.nonce_domain == request.nonce_domain
            && completion.nonce == reservation.reservation.nonce
            && completion.semantic_reservation_key
                == reservation.reservation.semantic_reservation_key
            && completion.semantic_completion_key == request.completion_key
            && completion.canonical_terminal_outcome == request.canonical_terminal_outcome
            && completion.terminal_witnesses == request.terminal_witnesses
            && completion.sealed_activated_candidates == candidates
            && completion.validate().is_ok()
            && expected_witness_ref.is_ok_and(|reference| {
                completion.original_terminal_witnesses_ref == reference.content_digest()
            })
            && expected_evidence
                .is_ok_and(|evidence| completion.completion_evidence_ref == evidence)
    }

    async fn load_validated_reservation(
        &self,
        connection: &mut PgConnection,
        reservation_key: &str,
    ) -> Result<Option<ReservationClosure>> {
        let reservation = load_reservation(connection, reservation_key).await?;
        if reservation
            .as_ref()
            .is_some_and(|closure| !self.retained_reservation_is_valid(closure))
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(reservation)
    }

    fn retained_reservation_is_valid(&self, closure: &ReservationClosure) -> bool {
        let request = &closure.request;
        let reservation = &closure.reservation;
        let observed_floor_ref = mfm_journal::structured::domain_content_digest(
            "mfm.evm.wallet-observed-floor-provenance.v1",
            &(&request.qualified_floor, &closure.state_input_ref),
        );
        let reservation_evidence_ref = evidence_reference(
            "mfm.evm.wallet-reservation-evidence.v1",
            &(
                request,
                &closure.state_input_ref,
                &reservation.resource_lineage_ref,
                reservation.nonce,
            ),
        );
        EffectCapabilityImplementation::<ReserveWalletNonceCapability>::validate_request(
            &self.validator,
            request,
        )
        .is_ok()
            && self
                .validate_activation(&request.domain_activation_attestation)
                .is_ok()
            && request.domain_activation_attestation
                == *self.activation_verifier.exact_attestation()
            && reservation.domain_activation_record_ref
                == request.domain_activation_attestation.activation_record_ref
            && reservation.resource_lineage_ref.to_content_ref().is_ok()
            && observed_floor_ref
                .is_ok_and(|digest| reservation.observed_floor_ref == digest.as_str())
            && reservation_evidence_ref
                .is_ok_and(|evidence| reservation.reservation_evidence_ref == evidence)
    }

    fn context(
        &self,
        database_oid: u32,
        backend_pid: i32,
        transaction_id: Option<u32>,
        snapshot_id: Option<String>,
        application_marker: String,
    ) -> ProviderTargetContext {
        ProviderTargetContext {
            database_oid,
            backend_pid,
            transaction_id,
            snapshot_id,
            schema_name: self.schema_name.clone(),
            application_marker,
            store_incarnation: self.store_incarnation.clone(),
        }
    }

    fn cache_supersession(
        &self,
        evidence: &WalletNonceStoreLineageHead,
        public_head: &HistoryObject,
    ) -> bool {
        if evidence.validate().is_err()
            || public_head.validate().is_err()
            || evidence.wallet_nonce_store_lineage_id
                != self.store_incarnation.wallet_nonce_store_lineage_id
            || evidence.writer_epoch <= self.store_incarnation.writer_epoch
            || evidence.public_lineage_head_ref
                != EvmWalletReference::from_content_ref(public_head.content_ref.clone())
        {
            return false;
        }
        self.supersession_heads
            .lock()
            .map(|mut heads| {
                heads.insert(
                    evidence.public_lineage_head_ref.content_digest().to_owned(),
                    public_head.clone(),
                );
            })
            .is_ok()
    }

    fn effect_fence_disposition<T, R>(
        &self,
        disposition: ProviderDisposition<T>,
    ) -> std::result::Result<
        T,
        EffectAdapterCompletion<R, EvmSubmissionFailure, WalletNonceStoreLineageHead>,
    > {
        match disposition {
            ProviderDisposition::Current(value) => Ok(value),
            ProviderDisposition::Superseded {
                evidence,
                public_head,
            } if self.cache_supersession(&evidence, &public_head) => {
                Err(EffectAdapterCompletion::SupersededBeforeEntry(evidence))
            }
            ProviderDisposition::Superseded { .. } | ProviderDisposition::Integrity => Err(
                EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone()),
            ),
            ProviderDisposition::Unavailable => Err(EffectAdapterCompletion::SafeFailure(
                EvmSubmissionFailure::NonceAuthorityUnavailable,
            )),
            ProviderDisposition::EntryUnknown => Err(EffectAdapterCompletion::EntryUnknown(
                self.entry_unknown_fault.clone(),
            )),
        }
    }

    async fn begin_write(
        &self,
        state_input_ref: &LexicalValueRef,
        operation_key: &str,
    ) -> std::result::Result<WriteTransaction<'_>, WriteStartFailure> {
        let pending_lease = self
            .activation_verifier
            .begin_write(state_input_ref, operation_key)
            .await
            .map_err(write_provider_error)?;
        let application_marker = pending_lease.application_marker().to_owned();
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| WriteStartFailure::Safe)?;
        sqlx::query("SELECT pg_catalog.set_config('application_name', $1, FALSE)")
            .bind(&application_marker)
            .execute(&mut *transaction)
            .await
            .map_err(|_| WriteStartFailure::Safe)?;
        acquire_target_session_marker(&mut transaction, &application_marker)
            .await
            .map_err(|_| WriteStartFailure::Safe)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *transaction)
            .await
            .map_err(|_| WriteStartFailure::Safe)?;
        let (database_oid, backend_pid) = session_identity(&mut transaction)
            .await
            .map_err(|_| WriteStartFailure::Safe)?;
        let transaction_id = transaction_id(&mut transaction)
            .await
            .map_err(|_| WriteStartFailure::Safe)?;
        let context = self.context(
            database_oid,
            backend_pid,
            Some(transaction_id),
            None,
            application_marker,
        );
        let lease = match pending_lease
            .bind(context.clone())
            .await
            .map_err(write_provider_error)?
        {
            ProviderDisposition::Current(lease) => lease,
            ProviderDisposition::Superseded {
                evidence,
                public_head,
            } if self.cache_supersession(&evidence, &public_head) => {
                return Err(WriteStartFailure::Superseded(evidence));
            }
            ProviderDisposition::Superseded { .. } | ProviderDisposition::Integrity => {
                return Err(WriteStartFailure::Integrity);
            }
            ProviderDisposition::Unavailable => return Err(WriteStartFailure::Safe),
            ProviderDisposition::EntryUnknown => {
                return Err(WriteStartFailure::EntryUnknown);
            }
        };
        Ok(WriteTransaction { transaction, lease })
    }

    async fn revalidate_write<R>(
        &self,
        write: &mut WriteTransaction<'_>,
    ) -> std::result::Result<
        (),
        EffectAdapterCompletion<R, EvmSubmissionFailure, WalletNonceStoreLineageHead>,
    > {
        let disposition = write
            .lease
            .revalidate()
            .await
            .map_err(|error| match error {
                PostgresEvmWalletError::Unavailable => {
                    EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
                }
                _ => EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone()),
            })?;
        self.effect_fence_disposition(disposition)
    }

    async fn prepare_mutation<R>(
        &self,
        write: &mut WriteTransaction<'_>,
        mutation: Box<ProviderMutation>,
    ) -> std::result::Result<
        (),
        EffectAdapterCompletion<R, EvmSubmissionFailure, WalletNonceStoreLineageHead>,
    > {
        let disposition =
            write
                .lease
                .prepare_mutation(mutation)
                .await
                .map_err(|error| match error {
                    PostgresEvmWalletError::Unavailable => {
                        EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
                    }
                    _ => EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone()),
                })?;
        self.effect_fence_disposition(disposition)
    }

    async fn abort_write<R>(
        &self,
        write: WriteTransaction<'_>,
        completion: EffectAdapterCompletion<R, EvmSubmissionFailure, WalletNonceStoreLineageHead>,
    ) -> EffectAdapterCompletion<R, EvmSubmissionFailure, WalletNonceStoreLineageHead> {
        let WriteTransaction { transaction, lease } = write;
        let database_rollback_observed = transaction.rollback().await.is_ok();
        let provider_finished = matches!(
            lease.finish(false).await,
            Ok(ProviderDisposition::Current(()))
        );
        if database_rollback_observed && provider_finished {
            completion
        } else {
            EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
        }
    }

    fn write_start_completion<R>(
        &self,
        failure: WriteStartFailure,
    ) -> EffectAdapterCompletion<R, EvmSubmissionFailure, WalletNonceStoreLineageHead> {
        match failure {
            WriteStartFailure::Safe => EffectAdapterCompletion::SafeFailure(
                EvmSubmissionFailure::NonceAuthorityUnavailable,
            ),
            WriteStartFailure::Superseded(evidence) => {
                EffectAdapterCompletion::SupersededBeforeEntry(evidence)
            }
            WriteStartFailure::EntryUnknown => {
                EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
            }
            WriteStartFailure::Integrity => {
                EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone())
            }
        }
    }

    fn returned_reservation(
        &self,
        response: ReserveWalletNonceResponse,
    ) -> EffectAdapterCompletion<
        ReserveWalletNonceResponse,
        EvmSubmissionFailure,
        WalletNonceStoreLineageHead,
    > {
        if EffectCapabilityImplementation::<ReserveWalletNonceCapability>::validate_returned(
            &self.validator,
            &response,
        )
        .is_ok()
        {
            EffectAdapterCompletion::Returned(response)
        } else {
            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone())
        }
    }

    fn returned_activation(
        &self,
        response: ActivateCandidateResponse,
    ) -> EffectAdapterCompletion<
        ActivateCandidateResponse,
        EvmSubmissionFailure,
        WalletNonceStoreLineageHead,
    > {
        if EffectCapabilityImplementation::<ActivateWalletCandidateCapability>::validate_returned(
            &self.validator,
            &response,
        )
        .is_ok()
        {
            EffectAdapterCompletion::Returned(response)
        } else {
            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone())
        }
    }

    fn returned_completion(
        &self,
        response: CompleteWalletNonceResponse,
    ) -> EffectAdapterCompletion<
        CompleteWalletNonceResponse,
        EvmSubmissionFailure,
        WalletNonceStoreLineageHead,
    > {
        if EffectCapabilityImplementation::<CompleteWalletNonceCapability>::validate_returned(
            &self.validator,
            &response,
        )
        .is_ok()
        {
            EffectAdapterCompletion::Returned(response)
        } else {
            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone())
        }
    }

    async fn read_status_inner(
        &self,
        request: &ReadEvmWalletNonceStatusRequest,
    ) -> Result<WalletNonceStatus> {
        self.validate_activation(&request.domain_activation_attestation)?;
        let mut read = self.begin_read_snapshot().await?;

        let retained_activation =
            load_domain_activation(&mut read.transaction, request.nonce_domain.as_str(), false)
                .await?;
        let aggregate = self
            .load_validated_current_domain(
                &mut read.transaction,
                request.nonce_domain.as_str(),
                retained_activation.as_ref(),
            )
            .await?;
        if let Some(retained_activation) = retained_activation.as_ref() {
            if retained_activation != &request.domain_activation_attestation {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
        }

        let status = match self
            .load_validated_reservation(
                &mut read.transaction,
                request.semantic_reservation_key.as_str(),
            )
            .await?
        {
            Some(closure) => {
                if retained_activation.is_none() || !status_request_matches(request, &closure) {
                    return Err(PostgresEvmWalletError::InvalidAuthority);
                }
                let candidates = self
                    .load_validated_candidates(&mut read.transaction, &closure)
                    .await?;
                match self
                    .load_validated_completion(&mut read.transaction, &closure, &candidates)
                    .await?
                {
                    Some(completion) => WalletNonceStatus::Completed {
                        reservation: *closure.reservation,
                        transaction_intent: *closure.transaction_intent,
                        candidate_family: *closure.candidate_family,
                        activated_candidates: candidates,
                        completion: completion.completion,
                        resource_head_ref: self
                            .current_lineage_head
                            .public_lineage_head_ref
                            .clone(),
                    },
                    None => WalletNonceStatus::Reserved {
                        reservation: *closure.reservation,
                        transaction_intent: *closure.transaction_intent,
                        candidate_family: *closure.candidate_family,
                        current_candidate: candidates.last().cloned(),
                        activated_candidates: candidates,
                        resource_head_ref: self
                            .current_lineage_head
                            .public_lineage_head_ref
                            .clone(),
                    },
                }
            }
            None => {
                if aggregate.incomplete_reservation_key.is_some() {
                    WalletNonceStatus::Busy
                } else {
                    WalletNonceStatus::Absent
                }
            }
        };
        self.finish_read_snapshot(read).await?;
        Ok(status)
    }

    async fn begin_read_snapshot(&self) -> Result<ReadTransaction<'_>> {
        let pending_lease = self.activation_verifier.begin_read().await?;
        let application_marker = pending_lease.application_marker().to_owned();
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query("SELECT pg_catalog.set_config('application_name', $1, FALSE)")
            .bind(&application_marker)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        acquire_target_session_marker(&mut transaction, &application_marker).await?;
        let (database_oid, backend_pid) = session_identity(&mut transaction).await?;
        let snapshot_id = sqlx::query_scalar::<_, String>("SELECT pg_current_snapshot()::text")
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        if snapshot_id.is_empty() || snapshot_id.len() > 256 {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let context = self.context(
            database_oid,
            backend_pid,
            None,
            Some(snapshot_id),
            application_marker,
        );
        let lease = match pending_lease.bind(context).await? {
            ProviderDisposition::Current(lease) => lease,
            ProviderDisposition::Unavailable | ProviderDisposition::Superseded { .. } => {
                return Err(PostgresEvmWalletError::Unavailable);
            }
            ProviderDisposition::EntryUnknown | ProviderDisposition::Integrity => {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
        };
        Ok(ReadTransaction { transaction, lease })
    }

    async fn begin_resolution_snapshot(
        &self,
        pending_lease: PendingResolutionLease,
    ) -> Result<ReadTransaction<'_>> {
        let application_marker = pending_lease.application_marker().to_owned();
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        sqlx::query("SELECT pg_catalog.set_config('application_name', $1, FALSE)")
            .bind(&application_marker)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        acquire_target_session_marker(&mut transaction, &application_marker).await?;
        let (database_oid, backend_pid) = session_identity(&mut transaction).await?;
        let snapshot_id = sqlx::query_scalar::<_, String>("SELECT pg_current_snapshot()::text")
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        if snapshot_id.is_empty() || snapshot_id.len() > 256 {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let context = self.context(
            database_oid,
            backend_pid,
            None,
            Some(snapshot_id),
            application_marker,
        );
        let lease = match pending_lease.bind(context).await? {
            ProviderDisposition::Current(lease) => lease,
            ProviderDisposition::Unavailable | ProviderDisposition::Superseded { .. } => {
                return Err(PostgresEvmWalletError::Unavailable);
            }
            ProviderDisposition::EntryUnknown | ProviderDisposition::Integrity => {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
        };
        Ok(ReadTransaction { transaction, lease })
    }

    async fn finish_read_snapshot(&self, read: ReadTransaction<'_>) -> Result<()> {
        let ReadTransaction { transaction, lease } = read;
        transaction
            .commit()
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        match lease.finish().await? {
            ProviderDisposition::Current(()) => Ok(()),
            ProviderDisposition::Unavailable | ProviderDisposition::Superseded { .. } => {
                Err(PostgresEvmWalletError::Unavailable)
            }
            ProviderDisposition::EntryUnknown | ProviderDisposition::Integrity => {
                Err(PostgresEvmWalletError::InvalidAuthority)
            }
        }
    }

    async fn require_retained_activation(
        &self,
        transaction: &mut PgConnection,
        domain_id: &str,
    ) -> Result<()> {
        let retained = load_domain_activation(transaction, domain_id, false)
            .await?
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        self.load_validated_domain_aggregate(transaction, domain_id, Some(&retained))
            .await?;
        if &retained != self.activation_verifier.exact_attestation() {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(())
    }

    async fn candidate_replay_is_valid(
        &self,
        transaction: &mut PgConnection,
        request: &ActivateEvmCandidateRequest,
        retained: &RetainedCandidate,
    ) -> Result<bool> {
        if !candidate_request_matches(request, retained) {
            return Ok(false);
        }
        let Some(reservation) = self
            .load_validated_reservation(
                transaction,
                request.next_candidate.semantic_reservation_key.as_str(),
            )
            .await?
        else {
            return Ok(false);
        };
        if reservation.reservation.nonce_domain != request.nonce_domain
            || self
                .validate_activation(&reservation.request.domain_activation_attestation)
                .is_err()
            || reservation.request.domain_activation_attestation
                != *self.activation_verifier.exact_attestation()
        {
            return Ok(false);
        }
        let candidates = self
            .load_validated_candidates(transaction, &reservation)
            .await?;
        let ordinal = usize::from(request.next_candidate.candidate_ordinal);
        let Some(existing) = candidates.get(ordinal) else {
            return Ok(false);
        };
        if existing != &retained.candidate
            || !attested_candidate_semantics_match(request, &reservation)
        {
            return Ok(false);
        }
        // Idempotent activation replay is checked against the original
        // Initial/Replacement permit and the prefix that precedes this ordinal.
        Ok(candidate_progression_matches(
            request,
            &reservation,
            &candidates[..ordinal],
        ))
    }

    async fn resolve_reservation_after_commit(
        &self,
        request: &ReserveEvmNonceRequest,
        pending_lease: PendingResolutionLease,
    ) -> Result<Option<ReserveWalletNonceResponse>> {
        self.validate_activation(&request.domain_activation_attestation)?;
        let mut read = self.begin_resolution_snapshot(pending_lease).await?;
        let retained_activation =
            load_domain_activation(&mut read.transaction, request.nonce_domain.as_str(), false)
                .await?;
        self.load_validated_domain_aggregate(
            &mut read.transaction,
            request.nonce_domain.as_str(),
            retained_activation.as_ref(),
        )
        .await?;
        if let Some(retained_activation) = retained_activation.as_ref() {
            if retained_activation != self.activation_verifier.exact_attestation() {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
        }
        let closure = self
            .load_validated_reservation(&mut read.transaction, request.reservation_key.as_str())
            .await?;
        if closure.is_some() && retained_activation.is_none() {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        let result = closure
            .map(|closure| {
                if reservation_request_matches(request, &closure) {
                    Ok(ReserveWalletNonceResponse::Reserved {
                        reservation: *closure.reservation,
                    })
                } else {
                    Err(PostgresEvmWalletError::InvalidAuthority)
                }
            })
            .transpose()?;
        self.finish_read_snapshot(read).await?;
        Ok(result)
    }

    async fn resolve_activation_after_commit(
        &self,
        request: &ActivateEvmCandidateRequest,
        pending_lease: PendingResolutionLease,
    ) -> Result<Option<ActivateCandidateResponse>> {
        let mut read = self.begin_resolution_snapshot(pending_lease).await?;
        self.require_retained_activation(&mut read.transaction, request.nonce_domain.as_str())
            .await?;
        let retained = load_candidate_by_key(
            &mut read.transaction,
            request.candidate_operation_key.as_str(),
        )
        .await?;
        let result = match retained {
            Some(retained)
                if self
                    .candidate_replay_is_valid(&mut read.transaction, request, &retained)
                    .await? =>
            {
                Some(ActivateCandidateResponse::Activated {
                    candidate: retained.candidate,
                })
            }
            Some(_) => return Err(PostgresEvmWalletError::InvalidAuthority),
            None => None,
        };
        self.finish_read_snapshot(read).await?;
        Ok(result)
    }

    async fn resolve_completion_after_commit(
        &self,
        request: &CompleteEvmNonceRequest,
        pending_lease: PendingResolutionLease,
    ) -> Result<Option<CompleteWalletNonceResponse>> {
        let mut read = self.begin_resolution_snapshot(pending_lease).await?;
        self.require_retained_activation(
            &mut read.transaction,
            request.current_reservation.nonce_domain.as_str(),
        )
        .await?;
        let reservation = self
            .load_validated_reservation(
                &mut read.transaction,
                request
                    .current_reservation
                    .semantic_reservation_key
                    .as_str(),
            )
            .await?
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        let candidates = self
            .load_validated_candidates(&mut read.transaction, &reservation)
            .await?;
        let result = match self
            .load_validated_completion(&mut read.transaction, &reservation, &candidates)
            .await?
        {
            Some(retained) if completion_request_matches(request, &retained) => {
                Some(CompleteWalletNonceResponse::Completed {
                    completion: retained.completion,
                })
            }
            Some(_) => return Err(PostgresEvmWalletError::InvalidAuthority),
            None => None,
        };
        self.finish_read_snapshot(read).await?;
        Ok(result)
    }

    async fn resolve_reservation_ambiguity(
        &self,
        request: &ReserveEvmNonceRequest,
        lease: WriteTransactionLease,
    ) -> DatabaseAttempt<ReserveWalletNonceResponse> {
        let pending = match lease.begin_resolution().await {
            Ok(ProviderDisposition::Current(pending)) => pending,
            _ => {
                return DatabaseAttempt::Return(EffectAdapterCompletion::EntryUnknown(
                    self.entry_unknown_fault.clone(),
                ));
            }
        };
        match self
            .resolve_reservation_after_commit(request, pending)
            .await
        {
            Ok(Some(response)) => DatabaseAttempt::Return(self.returned_reservation(response)),
            Ok(None) => DatabaseAttempt::Retry,
            Err(
                PostgresEvmWalletError::InvalidAuthority
                | PostgresEvmWalletError::PermanentConflict
                | PostgresEvmWalletError::FenceRejected,
            ) => DatabaseAttempt::Return(EffectAdapterCompletion::IntegrityFault(
                self.integrity_fault.clone(),
            )),
            Err(PostgresEvmWalletError::Unavailable) => DatabaseAttempt::Return(
                EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone()),
            ),
        }
    }

    async fn resolve_activation_ambiguity(
        &self,
        request: &ActivateEvmCandidateRequest,
        lease: WriteTransactionLease,
    ) -> DatabaseAttempt<ActivateCandidateResponse> {
        let pending = match lease.begin_resolution().await {
            Ok(ProviderDisposition::Current(pending)) => pending,
            _ => {
                return DatabaseAttempt::Return(EffectAdapterCompletion::EntryUnknown(
                    self.entry_unknown_fault.clone(),
                ));
            }
        };
        match self.resolve_activation_after_commit(request, pending).await {
            Ok(Some(response)) => DatabaseAttempt::Return(self.returned_activation(response)),
            Ok(None) => DatabaseAttempt::Retry,
            Err(
                PostgresEvmWalletError::InvalidAuthority
                | PostgresEvmWalletError::PermanentConflict
                | PostgresEvmWalletError::FenceRejected,
            ) => DatabaseAttempt::Return(EffectAdapterCompletion::IntegrityFault(
                self.integrity_fault.clone(),
            )),
            Err(PostgresEvmWalletError::Unavailable) => DatabaseAttempt::Return(
                EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone()),
            ),
        }
    }

    async fn resolve_completion_ambiguity(
        &self,
        request: &CompleteEvmNonceRequest,
        lease: WriteTransactionLease,
    ) -> DatabaseAttempt<CompleteWalletNonceResponse> {
        let pending = match lease.begin_resolution().await {
            Ok(ProviderDisposition::Current(pending)) => pending,
            _ => {
                return DatabaseAttempt::Return(EffectAdapterCompletion::EntryUnknown(
                    self.entry_unknown_fault.clone(),
                ));
            }
        };
        match self.resolve_completion_after_commit(request, pending).await {
            Ok(Some(response)) => DatabaseAttempt::Return(self.returned_completion(response)),
            Ok(None) => DatabaseAttempt::Retry,
            Err(
                PostgresEvmWalletError::InvalidAuthority
                | PostgresEvmWalletError::PermanentConflict
                | PostgresEvmWalletError::FenceRejected,
            ) => DatabaseAttempt::Return(EffectAdapterCompletion::IntegrityFault(
                self.integrity_fault.clone(),
            )),
            Err(PostgresEvmWalletError::Unavailable) => DatabaseAttempt::Return(
                EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone()),
            ),
        }
    }

    fn bound_database_attempt<R>(
        &self,
        result: DatabaseAttempt<R>,
        attempt_number: usize,
    ) -> std::result::Result<WalletMutationCompletion<R>, RetryDatabaseAttempt> {
        match result {
            DatabaseAttempt::Return(completion) => Ok(completion),
            DatabaseAttempt::Retry if attempt_number < MAX_DATABASE_ATTEMPTS_PER_INVOCATION => {
                Err(RetryDatabaseAttempt)
            }
            DatabaseAttempt::Retry => Ok(EffectAdapterCompletion::EntryUnknown(
                self.entry_unknown_fault.clone(),
            )),
        }
    }

    async fn commit_reservation_attempt(
        &self,
        write: WriteTransaction<'_>,
        request: &ReserveEvmNonceRequest,
        completion: EffectAdapterCompletion<
            ReserveWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    ) -> DatabaseAttempt<ReserveWalletNonceResponse> {
        let WriteTransaction { transaction, lease } = write;
        match transaction.commit().await {
            Ok(()) => {
                let completion = if matches!(
                    lease.finish(true).await,
                    Ok(ProviderDisposition::Current(()))
                ) {
                    completion
                } else {
                    EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
                };
                DatabaseAttempt::Return(completion)
            }
            Err(_) => self.resolve_reservation_ambiguity(request, lease).await,
        }
    }

    async fn commit_activation_attempt(
        &self,
        write: WriteTransaction<'_>,
        request: &ActivateEvmCandidateRequest,
        completion: EffectAdapterCompletion<
            ActivateCandidateResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    ) -> DatabaseAttempt<ActivateCandidateResponse> {
        let WriteTransaction { transaction, lease } = write;
        match transaction.commit().await {
            Ok(()) => {
                let completion = if matches!(
                    lease.finish(true).await,
                    Ok(ProviderDisposition::Current(()))
                ) {
                    completion
                } else {
                    EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
                };
                DatabaseAttempt::Return(completion)
            }
            Err(_) => self.resolve_activation_ambiguity(request, lease).await,
        }
    }

    async fn commit_completion_attempt(
        &self,
        write: WriteTransaction<'_>,
        request: &CompleteEvmNonceRequest,
        completion: EffectAdapterCompletion<
            CompleteWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    ) -> DatabaseAttempt<CompleteWalletNonceResponse> {
        let WriteTransaction { transaction, lease } = write;
        match transaction.commit().await {
            Ok(()) => {
                let completion = if matches!(
                    lease.finish(true).await,
                    Ok(ProviderDisposition::Current(()))
                ) {
                    completion
                } else {
                    EffectAdapterCompletion::EntryUnknown(self.entry_unknown_fault.clone())
                };
                DatabaseAttempt::Return(completion)
            }
            Err(_) => self.resolve_completion_ambiguity(request, lease).await,
        }
    }
}

impl WalletNonceAuthority for PostgresWalletNonceAuthority {
    fn domain_activation_attestation(&self) -> &WalletNonceDomainActivationAttestation {
        self.activation_verifier.exact_attestation()
    }

    fn read_status<'a>(
        &'a self,
        _state_input_ref: &'a LexicalValueRef,
        request: &'a ReadEvmWalletNonceStatusRequest,
    ) -> mfm_capabilities::ComponentFuture<
        'a,
        ReadAdapterCompletion<WalletNonceStatus, EvmSubmissionFailure>,
    > {
        Box::pin(async move {
            if ReadCapabilityImplementation::<ReadWalletNonceStatusCapability>::validate_request(
                &self.validator,
                request,
            )
            .is_err()
                || self
                    .validate_activation(&request.domain_activation_attestation)
                    .is_err()
            {
                return ReadAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
            }
            match self.read_status_inner(request).await {
                Ok(status)
                    if ReadCapabilityImplementation::<ReadWalletNonceStatusCapability>::validate_returned(
                        &self.validator,
                        &status,
                    )
                    .is_ok() => ReadAdapterCompletion::Returned(status),
                Ok(_) | Err(PostgresEvmWalletError::InvalidAuthority | PostgresEvmWalletError::PermanentConflict | PostgresEvmWalletError::FenceRejected) => {
                    ReadAdapterCompletion::IntegrityFault(self.integrity_fault.clone())
                }
                Err(PostgresEvmWalletError::Unavailable) => ReadAdapterCompletion::SafeFailure(
                    EvmSubmissionFailure::NonceAuthorityUnavailable,
                ),
            }
        })
    }

    fn reserve_qualified<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        observation: &'a QualifiedPendingNonceObservation,
        request: &'a ReserveEvmNonceRequest,
    ) -> mfm_capabilities::ComponentFuture<
        'a,
        EffectAdapterCompletion<
            ReserveWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    > {
        Box::pin(async move {
            let expected_chain = request
                .domain_activation_attestation
                .current_schema_record
                .chain_instance_attestation
                .content_ref()
                .ok();
            let valid = observation.validate().is_ok()
                && observation.floor() == &request.qualified_floor
                && expected_chain.as_ref() == Some(observation.chain_instance_ref())
                && observation
                    .sender()
                    .is_ok_and(|sender| format!("{sender:#x}") == request.nonce_domain.sender())
                && self
                    .validate_activation(&request.domain_activation_attestation)
                    .is_ok();
            if !valid {
                return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
            }
            self.reserve(state_input_ref, request).await
        })
    }

    fn reserve<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ReserveEvmNonceRequest,
    ) -> mfm_capabilities::ComponentFuture<
        'a,
        EffectAdapterCompletion<
            ReserveWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    > {
        Box::pin(async move {
            if EffectCapabilityImplementation::<ReserveWalletNonceCapability>::validate_request(
                &self.validator,
                request,
            )
            .is_err()
                || self
                    .validate_activation(&request.domain_activation_attestation)
                    .is_err()
            {
                return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
            }
            let mut database_attempt = 0;
            'database_attempt: loop {
                database_attempt += 1;
                let mut write = match self
                    .begin_write(state_input_ref, request.reservation_key.as_str())
                    .await
                {
                    Ok(write) => write,
                    Err(failure) => return self.write_start_completion(failure),
                };
                match self
                    .load_validated_reservation(
                        &mut write.transaction,
                        request.reservation_key.as_str(),
                    )
                    .await
                {
                    Ok(Some(existing)) if reservation_request_matches(request, &existing) => {}
                    Ok(Some(_))
                    | Err(
                        PostgresEvmWalletError::InvalidAuthority
                        | PostgresEvmWalletError::PermanentConflict
                        | PostgresEvmWalletError::FenceRejected,
                    ) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Ok(None) => {}
                }
                if lock_domain(&mut write.transaction, request.nonce_domain.as_str())
                    .await
                    .is_err()
                {
                    let completion = EffectAdapterCompletion::SafeFailure(
                        EvmSubmissionFailure::NonceAuthorityUnavailable,
                    );
                    return self.abort_write(write, completion).await;
                }
                if let Err(completion) = self.revalidate_write(&mut write).await {
                    return self.abort_write(write, completion).await;
                }
                let existing_reservation = match self
                    .load_validated_reservation(
                        &mut write.transaction,
                        request.reservation_key.as_str(),
                    )
                    .await
                {
                    Ok(Some(existing)) if reservation_request_matches(request, &existing) => {
                        Some(existing)
                    }
                    Ok(Some(_))
                    | Err(
                        PostgresEvmWalletError::InvalidAuthority
                        | PostgresEvmWalletError::PermanentConflict
                        | PostgresEvmWalletError::FenceRejected,
                    ) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Ok(None) => None,
                };

                let retained_domain = match load_domain_activation(
                    &mut write.transaction,
                    request.nonce_domain.as_str(),
                    true,
                )
                .await
                {
                    Ok(value) => value,
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                if retained_domain
                    .as_ref()
                    .is_some_and(|activation| activation != &request.domain_activation_attestation)
                {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                }
                if let Some(retained_domain) = retained_domain.as_ref() {
                    if self.validate_activation(retained_domain).is_err() {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                }
                let aggregate = match self
                    .load_validated_current_domain(
                        &mut write.transaction,
                        request.nonce_domain.as_str(),
                        retained_domain.as_ref(),
                    )
                    .await
                {
                    Ok(aggregate) => aggregate,
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                if let Some(existing) = existing_reservation {
                    let completion =
                        self.returned_reservation(ReserveWalletNonceResponse::Reserved {
                            reservation: *existing.reservation,
                        });
                    match self.bound_database_attempt(
                        self.commit_reservation_attempt(write, request, completion)
                            .await,
                        database_attempt,
                    ) {
                        Ok(completion) => return completion,
                        Err(RetryDatabaseAttempt) => continue 'database_attempt,
                    }
                }
                if aggregate.incomplete_reservation_key.is_some() {
                    let completion =
                        self.returned_reservation(ReserveWalletNonceResponse::NonceDomainBusy);
                    match self.bound_database_attempt(
                        self.commit_reservation_attempt(write, request, completion)
                            .await,
                        database_attempt,
                    ) {
                        Ok(completion) => return completion,
                        Err(RetryDatabaseAttempt) => continue 'database_attempt,
                    }
                }

                let pending_nonce = request.qualified_floor.observed.pending_nonce;
                let pending = pending_nonce.get();
                let nonce = match aggregate.high_water {
                    None if pending
                        == request
                            .domain_activation_attestation
                            .current_schema_record
                            .finalized_sender_nonce_floor =>
                    {
                        pending_nonce.get()
                    }
                    None => {
                        let completion = self
                            .returned_reservation(ReserveWalletNonceResponse::NonceLineageDiverged);
                        match self.bound_database_attempt(
                            self.commit_reservation_attempt(write, request, completion)
                                .await,
                            database_attempt,
                        ) {
                            Ok(completion) => return completion,
                            Err(RetryDatabaseAttempt) => continue 'database_attempt,
                        }
                    }
                    Some(high_water) => {
                        let Ok(high_water_nonce) = mfm_evm::TransactionNonce::new(high_water)
                        else {
                            let completion = self.returned_reservation(
                                ReserveWalletNonceResponse::NonceCapacityExhausted,
                            );
                            match self.bound_database_attempt(
                                self.commit_reservation_attempt(write, request, completion)
                                    .await,
                                database_attempt,
                            ) {
                                Ok(completion) => return completion,
                                Err(RetryDatabaseAttempt) => continue 'database_attempt,
                            }
                        };
                        match high_water_nonce.checked_successor() {
                            Some(next) if pending <= next.get() => next.get(),
                            Some(_) => {
                                let completion = self.returned_reservation(
                                    ReserveWalletNonceResponse::NonceLineageDiverged,
                                );
                                match self.bound_database_attempt(
                                    self.commit_reservation_attempt(write, request, completion)
                                        .await,
                                    database_attempt,
                                ) {
                                    Ok(completion) => return completion,
                                    Err(RetryDatabaseAttempt) => continue 'database_attempt,
                                }
                            }
                            None => {
                                let completion = self.returned_reservation(
                                    ReserveWalletNonceResponse::NonceCapacityExhausted,
                                );
                                match self.bound_database_attempt(
                                    self.commit_reservation_attempt(write, request, completion)
                                        .await,
                                    database_attempt,
                                ) {
                                    Ok(completion) => return completion,
                                    Err(RetryDatabaseAttempt) => continue 'database_attempt,
                                }
                            }
                        }
                    }
                };
                // Final admission gate: never persist an invalid nonce.
                if mfm_evm::TransactionNonce::new(nonce).is_err() {
                    let completion = self
                        .returned_reservation(ReserveWalletNonceResponse::NonceCapacityExhausted);
                    match self.bound_database_attempt(
                        self.commit_reservation_attempt(write, request, completion)
                            .await,
                        database_attempt,
                    ) {
                        Ok(completion) => return completion,
                        Err(RetryDatabaseAttempt) => continue 'database_attempt,
                    }
                }

                let observed_floor_ref = match mfm_journal::structured::domain_content_digest(
                    "mfm.evm.wallet-observed-floor-provenance.v1",
                    &(&request.qualified_floor, state_input_ref),
                ) {
                    Ok(value) => value.as_str().to_owned(),
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                let reservation_evidence_ref = match evidence_reference(
                    "mfm.evm.wallet-reservation-evidence.v1",
                    &(
                        request,
                        state_input_ref,
                        &self.current_lineage_head.public_lineage_head_ref,
                        nonce,
                    ),
                ) {
                    Ok(value) => value,
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                let reservation = ReservedWalletNonce {
                    nonce_domain: request.nonce_domain.clone(),
                    domain_activation_record_ref: request
                        .domain_activation_attestation
                        .activation_record_ref
                        .clone(),
                    nonce,
                    semantic_reservation_key: request.reservation_key.clone(),
                    submission_intent_id: request.submission_intent_id.clone(),
                    transaction_intent_digest: request.transaction_intent.digest().to_owned(),
                    candidate_family_ref: request.candidate_family.digest().to_owned(),
                    observed_floor_ref,
                    resource_lineage_ref: self.current_lineage_head.public_lineage_head_ref.clone(),
                    reservation_evidence_ref,
                };
                if let Err(completion) = self
                    .prepare_mutation(
                        &mut write,
                        Box::new(ProviderMutation::Reservation {
                            request: request.clone(),
                            reservation: reservation.clone(),
                            state_input_ref: state_input_ref.clone(),
                        }),
                    )
                    .await
                {
                    return self.abort_write(write, completion).await;
                }
                let persist = async {
                    if retained_domain.is_none() {
                        insert_domain_activation(&mut write.transaction, request, nonce).await?;
                    } else {
                        update_high_water(
                            &mut write.transaction,
                            request.nonce_domain.as_str(),
                            nonce,
                        )
                        .await?;
                    }
                    insert_reservation(
                        &mut write.transaction,
                        request,
                        &reservation,
                        state_input_ref,
                    )
                    .await
                }
                .await;
                if persist.is_err() {
                    let completion = EffectAdapterCompletion::SafeFailure(
                        EvmSubmissionFailure::NonceAuthorityUnavailable,
                    );
                    return self.abort_write(write, completion).await;
                }
                let current_incarnation_ref =
                    match canonical_wallet_reference(&self.store_incarnation) {
                        Ok(reference) => reference,
                        Err(_) => {
                            let completion = EffectAdapterCompletion::IntegrityFault(
                                self.integrity_fault.clone(),
                            );
                            return self.abort_write(write, completion).await;
                        }
                    };
                if update_domain_projection(
                    &mut write.transaction,
                    request.nonce_domain.as_str(),
                    Some(request.reservation_key.as_str()),
                    &self.current_lineage_head.public_lineage_head_ref,
                    &current_incarnation_ref,
                )
                .await
                .is_err()
                {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                }
                let completion =
                    self.returned_reservation(ReserveWalletNonceResponse::Reserved { reservation });
                match self.bound_database_attempt(
                    self.commit_reservation_attempt(write, request, completion)
                        .await,
                    database_attempt,
                ) {
                    Ok(completion) => return completion,
                    Err(RetryDatabaseAttempt) => continue 'database_attempt,
                }
            }
        })
    }

    fn activate_candidate<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        request: &'a ActivateEvmCandidateRequest,
    ) -> mfm_capabilities::ComponentFuture<
        'a,
        EffectAdapterCompletion<
            ActivateCandidateResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    > {
        Box::pin(async move {
            if EffectCapabilityImplementation::<ActivateWalletCandidateCapability>::validate_request(
                &self.validator,
                request,
            )
            .is_err()
            {
                return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
            }
            let mut database_attempt = 0;
            'database_attempt: loop {
                database_attempt += 1;
                let mut write = match self
                    .begin_write(state_input_ref, request.candidate_operation_key.as_str())
                    .await
                {
                    Ok(write) => write,
                    Err(failure) => return self.write_start_completion(failure),
                };
                match load_candidate_by_key(
                    &mut write.transaction,
                    request.candidate_operation_key.as_str(),
                )
                .await
                {
                    Ok(Some(existing)) if candidate_request_matches(request, &existing) => {}
                    Ok(Some(_))
                    | Err(
                        PostgresEvmWalletError::InvalidAuthority
                        | PostgresEvmWalletError::PermanentConflict
                        | PostgresEvmWalletError::FenceRejected,
                    ) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Ok(None) => {}
                }
                if lock_domain(&mut write.transaction, request.nonce_domain.as_str())
                    .await
                    .is_err()
                {
                    let completion = EffectAdapterCompletion::SafeFailure(
                        EvmSubmissionFailure::NonceAuthorityUnavailable,
                    );
                    return self.abort_write(write, completion).await;
                }
                if let Err(completion) = self.revalidate_write(&mut write).await {
                    return self.abort_write(write, completion).await;
                }
                match self
                    .require_retained_activation(
                        &mut write.transaction,
                        request.nonce_domain.as_str(),
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                }
                match load_candidate_by_key(
                    &mut write.transaction,
                    request.candidate_operation_key.as_str(),
                )
                .await
                {
                    Ok(Some(existing)) => {
                        match self
                            .candidate_replay_is_valid(&mut write.transaction, request, &existing)
                            .await
                        {
                            Ok(true) => {
                                let completion = self.returned_activation(
                                    ActivateCandidateResponse::Activated {
                                        candidate: existing.candidate,
                                    },
                                );
                                match self.bound_database_attempt(
                                    self.commit_activation_attempt(write, request, completion)
                                        .await,
                                    database_attempt,
                                ) {
                                    Ok(completion) => return completion,
                                    Err(RetryDatabaseAttempt) => continue 'database_attempt,
                                }
                            }
                            Ok(false)
                            | Err(
                                PostgresEvmWalletError::InvalidAuthority
                                | PostgresEvmWalletError::PermanentConflict
                                | PostgresEvmWalletError::FenceRejected,
                            ) => {
                                let completion = EffectAdapterCompletion::IntegrityFault(
                                    self.integrity_fault.clone(),
                                );
                                return self.abort_write(write, completion).await;
                            }
                            Err(PostgresEvmWalletError::Unavailable) => {
                                let completion = EffectAdapterCompletion::SafeFailure(
                                    EvmSubmissionFailure::NonceAuthorityUnavailable,
                                );
                                return self.abort_write(write, completion).await;
                            }
                        }
                    }
                    Err(
                        PostgresEvmWalletError::InvalidAuthority
                        | PostgresEvmWalletError::PermanentConflict
                        | PostgresEvmWalletError::FenceRejected,
                    ) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Ok(None) => {}
                }
                let Some(reservation) = (match self
                    .load_validated_reservation(
                        &mut write.transaction,
                        request.next_candidate.semantic_reservation_key.as_str(),
                    )
                    .await
                {
                    Ok(value) => value,
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                }) else {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                };
                if reservation.reservation.nonce_domain != request.nonce_domain {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                }
                if self
                    .validate_activation(&reservation.request.domain_activation_attestation)
                    .is_err()
                    || reservation.request.domain_activation_attestation
                        != *self.activation_verifier.exact_attestation()
                {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                }
                let candidates = match self
                    .load_validated_candidates(&mut write.transaction, &reservation)
                    .await
                {
                    Ok(value) => value,
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                match self
                    .load_validated_completion(&mut write.transaction, &reservation, &candidates)
                    .await
                {
                    Ok(Some(_)) => {
                        let completion = self.returned_activation(
                            ActivateCandidateResponse::CandidateProgressionConflict,
                        );
                        match self.bound_database_attempt(
                            self.commit_activation_attempt(write, request, completion)
                                .await,
                            database_attempt,
                        ) {
                            Ok(completion) => return completion,
                            Err(RetryDatabaseAttempt) => continue 'database_attempt,
                        }
                    }
                    Ok(None) => {}
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                }
                if !candidate_progression_matches(request, &reservation, &candidates) {
                    let completion = self.returned_activation(
                        ActivateCandidateResponse::CandidateProgressionConflict,
                    );
                    match self.bound_database_attempt(
                        self.commit_activation_attempt(write, request, completion)
                            .await,
                        database_attempt,
                    ) {
                        Ok(completion) => return completion,
                        Err(RetryDatabaseAttempt) => continue 'database_attempt,
                    }
                }
                if !attested_candidate_matches(request, &reservation, &candidates) {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                }
                let activation_evidence_ref = match evidence_reference(
                    "mfm.evm.wallet-candidate-activation-evidence.v1",
                    &(
                        request,
                        state_input_ref,
                        &reservation.reservation.resource_lineage_ref,
                    ),
                ) {
                    Ok(value) => value,
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                let candidate = ActiveWalletCandidate {
                    attested_candidate: request.next_candidate.clone(),
                    activation_evidence_ref,
                };
                if let Err(completion) = self
                    .prepare_mutation(
                        &mut write,
                        Box::new(ProviderMutation::CandidateActivation {
                            request: request.clone(),
                            candidate: candidate.clone(),
                            state_input_ref: state_input_ref.clone(),
                        }),
                    )
                    .await
                {
                    return self.abort_write(write, completion).await;
                }
                if insert_candidate(&mut write.transaction, request, &candidate, state_input_ref)
                    .await
                    .is_err()
                {
                    let completion = EffectAdapterCompletion::SafeFailure(
                        EvmSubmissionFailure::NonceAuthorityUnavailable,
                    );
                    return self.abort_write(write, completion).await;
                }
                let completion =
                    self.returned_activation(ActivateCandidateResponse::Activated { candidate });
                match self.bound_database_attempt(
                    self.commit_activation_attempt(write, request, completion)
                        .await,
                    database_attempt,
                ) {
                    Ok(completion) => return completion,
                    Err(RetryDatabaseAttempt) => continue 'database_attempt,
                }
            }
        })
    }

    fn complete<'a>(
        &'a self,
        state_input_ref: &'a LexicalValueRef,
        request: &'a CompleteEvmNonceRequest,
    ) -> mfm_capabilities::ComponentFuture<
        'a,
        EffectAdapterCompletion<
            CompleteWalletNonceResponse,
            EvmSubmissionFailure,
            WalletNonceStoreLineageHead,
        >,
    > {
        Box::pin(async move {
            if EffectCapabilityImplementation::<CompleteWalletNonceCapability>::validate_request(
                &self.validator,
                request,
            )
            .is_err()
            {
                return EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
            }
            let mut database_attempt = 0;
            'database_attempt: loop {
                database_attempt += 1;
                let mut write = match self
                    .begin_write(state_input_ref, request.completion_key.as_str())
                    .await
                {
                    Ok(write) => write,
                    Err(failure) => return self.write_start_completion(failure),
                };
                match load_completion_by_key(
                    &mut write.transaction,
                    request.completion_key.as_str(),
                )
                .await
                {
                    Ok(Some(existing)) if completion_request_matches(request, &existing) => {}
                    Ok(Some(_))
                    | Err(
                        PostgresEvmWalletError::InvalidAuthority
                        | PostgresEvmWalletError::PermanentConflict
                        | PostgresEvmWalletError::FenceRejected,
                    ) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Ok(None) => {}
                }
                if lock_domain(&mut write.transaction, request.nonce_domain.as_str())
                    .await
                    .is_err()
                {
                    let completion = EffectAdapterCompletion::SafeFailure(
                        EvmSubmissionFailure::NonceAuthorityUnavailable,
                    );
                    return self.abort_write(write, completion).await;
                }
                if let Err(completion) = self.revalidate_write(&mut write).await {
                    return self.abort_write(write, completion).await;
                }
                match self
                    .require_retained_activation(
                        &mut write.transaction,
                        request.nonce_domain.as_str(),
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                }
                let Some(reservation) = (match self
                    .load_validated_reservation(
                        &mut write.transaction,
                        request
                            .current_reservation
                            .semantic_reservation_key
                            .as_str(),
                    )
                    .await
                {
                    Ok(value) => value,
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                }) else {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                };
                if self
                    .validate_activation(&reservation.request.domain_activation_attestation)
                    .is_err()
                    || reservation.request.domain_activation_attestation
                        != *self.activation_verifier.exact_attestation()
                {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                }
                let candidates = match self
                    .load_validated_candidates(&mut write.transaction, &reservation)
                    .await
                {
                    Ok(value) => value,
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                if !completion_closure_matches(request, &reservation, &candidates) {
                    let completion =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, completion).await;
                }
                match self
                    .load_validated_completion(&mut write.transaction, &reservation, &candidates)
                    .await
                {
                    Ok(Some(existing)) if completion_request_matches(request, &existing) => {
                        let completion =
                            self.returned_completion(CompleteWalletNonceResponse::Completed {
                                completion: existing.completion,
                            });
                        match self.bound_database_attempt(
                            self.commit_completion_attempt(write, request, completion)
                                .await,
                            database_attempt,
                        ) {
                            Ok(completion) => return completion,
                            Err(RetryDatabaseAttempt) => continue 'database_attempt,
                        }
                    }
                    Ok(Some(_))
                    | Err(
                        PostgresEvmWalletError::InvalidAuthority
                        | PostgresEvmWalletError::PermanentConflict
                        | PostgresEvmWalletError::FenceRejected,
                    ) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                    Err(PostgresEvmWalletError::Unavailable) => {
                        let completion = EffectAdapterCompletion::SafeFailure(
                            EvmSubmissionFailure::NonceAuthorityUnavailable,
                        );
                        return self.abort_write(write, completion).await;
                    }
                    Ok(None) => {}
                }
                let original_terminal_witnesses_ref =
                    match canonical_wallet_reference(&request.terminal_witnesses) {
                        Ok(reference) => reference.content_digest().to_owned(),
                        Err(_) => {
                            let completion = EffectAdapterCompletion::IntegrityFault(
                                self.integrity_fault.clone(),
                            );
                            return self.abort_write(write, completion).await;
                        }
                    };
                let completion_evidence_ref = match evidence_reference(
                    "mfm.evm.wallet-completion-evidence.v1",
                    &(
                        request,
                        state_input_ref,
                        &reservation.reservation.resource_lineage_ref,
                    ),
                ) {
                    Ok(value) => value,
                    Err(_) => {
                        let completion =
                            EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                        return self.abort_write(write, completion).await;
                    }
                };
                let completed = CompletedWalletNonce {
                    nonce_domain: request.nonce_domain.clone(),
                    nonce: reservation.reservation.nonce,
                    semantic_reservation_key: reservation
                        .reservation
                        .semantic_reservation_key
                        .clone(),
                    semantic_completion_key: request.completion_key.clone(),
                    canonical_terminal_outcome: request.canonical_terminal_outcome.clone(),
                    terminal_witnesses: request.terminal_witnesses.clone(),
                    sealed_activated_candidates: candidates.clone(),
                    original_terminal_witnesses_ref,
                    completion_evidence_ref,
                };
                if completed.validate().is_err() {
                    let failure =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, failure).await;
                }
                if let Err(failure) = self
                    .prepare_mutation(
                        &mut write,
                        Box::new(ProviderMutation::Completion {
                            request: request.clone(),
                            completion: completed.clone(),
                            state_input_ref: state_input_ref.clone(),
                        }),
                    )
                    .await
                {
                    return self.abort_write(write, failure).await;
                }
                if insert_completion(&mut write.transaction, request, &completed, state_input_ref)
                    .await
                    .is_err()
                {
                    let failure = EffectAdapterCompletion::SafeFailure(
                        EvmSubmissionFailure::NonceAuthorityUnavailable,
                    );
                    return self.abort_write(write, failure).await;
                }
                if clear_active_reservation(&mut write.transaction, request.nonce_domain.as_str())
                    .await
                    .is_err()
                {
                    let failure =
                        EffectAdapterCompletion::IntegrityFault(self.integrity_fault.clone());
                    return self.abort_write(write, failure).await;
                }
                let response = self.returned_completion(CompleteWalletNonceResponse::Completed {
                    completion: completed,
                });
                match self.bound_database_attempt(
                    self.commit_completion_attempt(write, request, response)
                        .await,
                    database_attempt,
                ) {
                    Ok(completion) => return completion,
                    Err(RetryDatabaseAttempt) => continue 'database_attempt,
                }
            }
        })
    }

    fn supersession_head<'a>(
        &'a self,
        evidence: &'a WalletNonceStoreLineageHead,
    ) -> mfm_capabilities::ComponentFuture<'a, Option<HistoryObject>> {
        Box::pin(async move {
            if evidence == &self.current_lineage_head {
                return Some(self.current_public_lineage_head.clone());
            }
            self.supersession_heads.lock().ok().and_then(|heads| {
                heads
                    .get(evidence.public_lineage_head_ref.content_digest())
                    .cloned()
            })
        })
    }
}

struct ReadTransaction<'a> {
    transaction: sqlx::Transaction<'a, sqlx::Postgres>,
    lease: ReadSnapshotLease,
}

struct WriteTransaction<'a> {
    transaction: sqlx::Transaction<'a, sqlx::Postgres>,
    lease: WriteTransactionLease,
}

const MAX_DATABASE_ATTEMPTS_PER_INVOCATION: usize = 2;

type WalletMutationCompletion<R> =
    EffectAdapterCompletion<R, EvmSubmissionFailure, WalletNonceStoreLineageHead>;

enum DatabaseAttempt<R> {
    Return(WalletMutationCompletion<R>),
    Retry,
}

struct RetryDatabaseAttempt;

enum WriteStartFailure {
    Safe,
    Superseded(WalletNonceStoreLineageHead),
    EntryUnknown,
    Integrity,
}

fn write_provider_error(error: PostgresEvmWalletError) -> WriteStartFailure {
    match error {
        PostgresEvmWalletError::Unavailable => WriteStartFailure::Safe,
        PostgresEvmWalletError::FenceRejected => WriteStartFailure::Integrity,
        PostgresEvmWalletError::InvalidAuthority | PostgresEvmWalletError::PermanentConflict => {
            WriteStartFailure::Integrity
        }
    }
}

struct ReservationClosure {
    request: Box<ReserveEvmNonceRequest>,
    transaction_intent: Box<EvmTransactionIntent>,
    candidate_family: Box<EvmCandidateFamily>,
    reservation: Box<ReservedWalletNonce>,
    state_input_ref: LexicalValueRef,
}

struct ValidatedDomainAggregate {
    high_water: Option<u64>,
    incomplete_reservation_key: Option<String>,
    current_resource_frontier_ref: Option<String>,
    current_incarnation_ref: Option<String>,
}

struct RetainedCandidate {
    request: ActivateEvmCandidateRequest,
    candidate: ActiveWalletCandidate,
}

struct RetainedCompletion {
    request: CompleteEvmNonceRequest,
    terminal_outcome: CanonicalTerminalOutcome,
    completion: CompletedWalletNonce,
    state_input_ref: LexicalValueRef,
}

async fn acquire_target_session_marker(connection: &mut PgConnection, marker: &str) -> Result<()> {
    let (first, second) = target_session_marker_lock_keys(marker)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(first)
        .bind(second)
        .execute(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}

async fn lock_domain(connection: &mut PgConnection, domain_id: &str) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1))")
        .bind(domain_id)
        .execute(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}

async fn load_domain_activation(
    connection: &mut PgConnection,
    domain_id: &str,
    for_update: bool,
) -> Result<Option<WalletNonceDomainActivationAttestation>> {
    let sql = if for_update {
        "SELECT wallet_nonce_domain_id, activation_record_ref, \
                wallet_nonce_store_lineage_id, activation_record_json, \
                activation_attestation_json FROM wallet_nonce_domains \
         WHERE wallet_nonce_domain_id = $1 FOR UPDATE"
    } else {
        "SELECT wallet_nonce_domain_id, activation_record_ref, \
                wallet_nonce_store_lineage_id, activation_record_json, \
                activation_attestation_json FROM wallet_nonce_domains \
         WHERE wallet_nonce_domain_id = $1"
    };
    let row = sqlx::query(sql)
        .bind(domain_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    row.map(|row| {
        let record: WalletNonceDomainActivationRecord =
            decode_row_json(&row, "activation_record_json")?;
        let activation: WalletNonceDomainActivationAttestation =
            decode_row_json(&row, "activation_attestation_json")?;
        activation
            .validate()
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let retained_domain_id: &str = required_row_text(&row, "wallet_nonce_domain_id")?;
        let retained_activation_ref: &str = required_row_text(&row, "activation_record_ref")?;
        let retained_lineage_id: &str = required_row_text(&row, "wallet_nonce_store_lineage_id")?;
        if retained_domain_id != domain_id
            || retained_domain_id != record.wallet_nonce_domain.as_str()
            || retained_activation_ref != reference_text(&activation.activation_record_ref)?
            || retained_lineage_id != record.wallet_nonce_store_lineage_id
            || activation.current_schema_record != record
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(activation)
    })
    .transpose()
}

async fn load_reservation(
    connection: &mut PgConnection,
    reservation_key: &str,
) -> Result<Option<ReservationClosure>> {
    let row = sqlx::query(
        "SELECT semantic_reservation_key, wallet_nonce_domain_id, submission_intent_id, \
                nonce::text AS nonce, transaction_intent_digest, candidate_family_ref, \
                submission_semantics_digest, \
                request_json, transaction_intent_json, candidate_family_json, reservation_json, \
                state_input_json \
         FROM wallet_nonce_reservations WHERE semantic_reservation_key = $1",
    )
    .bind(reservation_key)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    row.map(|row| {
        let request: Box<ReserveEvmNonceRequest> = decode_row_json(&row, "request_json")?;
        let transaction_intent: Box<EvmTransactionIntent> =
            decode_row_json(&row, "transaction_intent_json")?;
        let candidate_family: Box<EvmCandidateFamily> =
            decode_row_json(&row, "candidate_family_json")?;
        let reservation: Box<ReservedWalletNonce> = decode_row_json(&row, "reservation_json")?;
        let state_input_ref: LexicalValueRef = decode_row_json(&row, "state_input_json")?;
        let retained_key = required_row_text(&row, "semantic_reservation_key")?;
        let retained_domain_id = required_row_text(&row, "wallet_nonce_domain_id")?;
        let retained_submission_intent_id = required_row_text(&row, "submission_intent_id")?;
        let retained_nonce = required_row_text(&row, "nonce")?;
        let retained_transaction_intent_digest =
            required_row_text(&row, "transaction_intent_digest")?;
        let retained_candidate_family_ref = required_row_text(&row, "candidate_family_ref")?;
        let retained_submission_semantics_digest =
            required_row_text(&row, "submission_semantics_digest")?;
        if retained_key != reservation_key
            || retained_key != reservation.semantic_reservation_key.as_str()
            || retained_domain_id != reservation.nonce_domain.as_str()
            || retained_submission_intent_id != reservation.submission_intent_id.as_str()
            || retained_nonce != reservation.nonce.to_string()
            || retained_transaction_intent_digest != reservation.transaction_intent_digest
            || retained_candidate_family_ref != reservation.candidate_family_ref
            || retained_submission_semantics_digest != request.submission_semantics_digest.as_str()
            || request.transaction_intent != *transaction_intent
            || request.candidate_family != *candidate_family
            || request.reservation_key != reservation.semantic_reservation_key
            || request.nonce_domain != reservation.nonce_domain
            || request.submission_intent_id != reservation.submission_intent_id
            || transaction_intent.digest() != reservation.transaction_intent_digest
            || candidate_family.digest() != reservation.candidate_family_ref
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(ReservationClosure {
            request,
            transaction_intent,
            candidate_family,
            reservation,
            state_input_ref,
        })
    })
    .transpose()
}

async fn load_candidate_by_key(
    connection: &mut PgConnection,
    candidate_key: &str,
) -> Result<Option<RetainedCandidate>> {
    let row = sqlx::query(
        "SELECT semantic_candidate_operation_key, semantic_reservation_key, candidate_ordinal, \
                request_json, active_candidate_json, state_input_json \
         FROM wallet_nonce_candidates \
         WHERE semantic_candidate_operation_key = $1",
    )
    .bind(candidate_key)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    row.map(|row| {
        let request: ActivateEvmCandidateRequest = decode_row_json(&row, "request_json")?;
        let candidate: ActiveWalletCandidate = decode_row_json(&row, "active_candidate_json")?;
        let _state_input_ref: LexicalValueRef = decode_row_json(&row, "state_input_json")?;
        let retained_key = required_row_text(&row, "semantic_candidate_operation_key")?;
        let retained_reservation_key = required_row_text(&row, "semantic_reservation_key")?;
        let retained_ordinal: i32 = row
            .try_get("candidate_ordinal")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if request.next_candidate != candidate.attested_candidate
            || retained_key != candidate_key
            || request.candidate_operation_key.as_str() != candidate_key
            || retained_reservation_key
                != candidate
                    .attested_candidate
                    .semantic_reservation_key
                    .as_str()
            || retained_ordinal != i32::from(candidate.attested_candidate.candidate_ordinal)
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(RetainedCandidate { request, candidate })
    })
    .transpose()
}

async fn load_candidates(
    connection: &mut PgConnection,
    reservation: &ReservationClosure,
    validator: &EvmSubmissionCapabilityImplementation,
) -> Result<Vec<ActiveWalletCandidate>> {
    let rows = sqlx::query(
        "SELECT semantic_candidate_operation_key, semantic_reservation_key, candidate_ordinal, \
                request_json, active_candidate_json, state_input_json \
         FROM wallet_nonce_candidates WHERE semantic_reservation_key = $1 \
         ORDER BY candidate_ordinal",
    )
    .bind(reservation.reservation.semantic_reservation_key.as_str())
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    let mut candidates = Vec::with_capacity(rows.len());
    for (ordinal, row) in rows.into_iter().enumerate() {
        let retained_ordinal: i32 = row
            .try_get("candidate_ordinal")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let request: ActivateEvmCandidateRequest = decode_row_json(&row, "request_json")?;
        let candidate: ActiveWalletCandidate = decode_row_json(&row, "active_candidate_json")?;
        let state_input_ref: LexicalValueRef = decode_row_json(&row, "state_input_json")?;
        let expected_key = derive_evm_candidate_operation_key(
            &candidate.attested_candidate.semantic_reservation_key,
            candidate.attested_candidate.candidate_ordinal,
        )
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let retained_key: &str = row
            .try_get("semantic_candidate_operation_key")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let retained_reservation_key: &str = row
            .try_get("semantic_reservation_key")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let expected_activation_evidence_ref = evidence_reference(
            "mfm.evm.wallet-candidate-activation-evidence.v1",
            &(
                &request,
                &state_input_ref,
                &reservation.reservation.resource_lineage_ref,
            ),
        )?;
        if retained_ordinal
            != i32::try_from(ordinal).map_err(|_| PostgresEvmWalletError::InvalidAuthority)?
            || usize::from(candidate.attested_candidate.candidate_ordinal) != ordinal
            || request.next_candidate != candidate.attested_candidate
            || request.candidate_operation_key != expected_key
            || retained_key != expected_key.as_str()
            || retained_reservation_key
                != reservation.reservation.semantic_reservation_key.as_str()
            || request.nonce_domain != reservation.reservation.nonce_domain
            || EffectCapabilityImplementation::<ActivateWalletCandidateCapability>::validate_request(
                validator,
                &request,
            )
            .is_err()
            || !candidate_progression_matches(&request, reservation, &candidates)
            || !attested_candidate_matches(&request, reservation, &candidates)
            || candidate.activation_evidence_ref != expected_activation_evidence_ref
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        candidates.push(candidate);
    }
    Ok(candidates)
}

async fn load_completion(
    connection: &mut PgConnection,
    reservation_key: &str,
) -> Result<Option<RetainedCompletion>> {
    let row = sqlx::query(
        "SELECT semantic_completion_key, semantic_reservation_key, request_json, \
                canonical_terminal_outcome_json, completion_json, state_input_json \
         FROM wallet_nonce_completions \
         WHERE semantic_reservation_key = $1",
    )
    .bind(reservation_key)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    decode_completion_row(row)
}

async fn load_completion_by_key(
    connection: &mut PgConnection,
    completion_key: &str,
) -> Result<Option<RetainedCompletion>> {
    let row = sqlx::query(
        "SELECT semantic_completion_key, semantic_reservation_key, request_json, \
                canonical_terminal_outcome_json, completion_json, state_input_json \
         FROM wallet_nonce_completions \
         WHERE semantic_completion_key = $1",
    )
    .bind(completion_key)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    decode_completion_row(row)
}

fn decode_completion_row(row: Option<sqlx::postgres::PgRow>) -> Result<Option<RetainedCompletion>> {
    row.map(|row| {
        let request: CompleteEvmNonceRequest = decode_row_json(&row, "request_json")?;
        let terminal_outcome: CanonicalTerminalOutcome =
            decode_row_json(&row, "canonical_terminal_outcome_json")?;
        let completion: CompletedWalletNonce = decode_row_json(&row, "completion_json")?;
        let state_input_ref: LexicalValueRef = decode_row_json(&row, "state_input_json")?;
        let retained_completion_key = required_row_text(&row, "semantic_completion_key")?;
        let retained_reservation_key = required_row_text(&row, "semantic_reservation_key")?;
        let terminal_witnesses_ref = canonical_wallet_reference(&request.terminal_witnesses)
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if retained_completion_key != completion.semantic_completion_key.as_str()
            || retained_reservation_key != completion.semantic_reservation_key.as_str()
            || request.completion_key != completion.semantic_completion_key
            || request.current_reservation.semantic_reservation_key
                != completion.semantic_reservation_key
            || request.canonical_terminal_outcome != terminal_outcome
            || request.canonical_terminal_outcome != completion.canonical_terminal_outcome
            || request.terminal_witnesses != completion.terminal_witnesses
            || completion.original_terminal_witnesses_ref != terminal_witnesses_ref.content_digest()
            || completion.validate().is_err()
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
        Ok(RetainedCompletion {
            request,
            terminal_outcome,
            completion,
            state_input_ref,
        })
    })
    .transpose()
}

fn decode_row_json<T: serde::de::DeserializeOwned>(
    row: &sqlx::postgres::PgRow,
    column: &str,
) -> Result<T> {
    let json: &str = row
        .try_get(column)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    decode_canonical(json)
}

fn required_row_text<'a>(row: &'a sqlx::postgres::PgRow, column: &str) -> Result<&'a str> {
    row.try_get(column)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

fn status_request_matches(
    request: &ReadEvmWalletNonceStatusRequest,
    closure: &ReservationClosure,
) -> bool {
    request.nonce_domain == closure.reservation.nonce_domain
        && request.semantic_reservation_key == closure.reservation.semantic_reservation_key
        && request.submission_intent_id == closure.reservation.submission_intent_id
        && request.submission_semantics_digest == closure.request.submission_semantics_digest
        && request.transaction_intent_digest == closure.reservation.transaction_intent_digest
        && request.candidate_family_ref == closure.reservation.candidate_family_ref
        && request.domain_activation_attestation.activation_record_ref
            == closure.reservation.domain_activation_record_ref
}

fn reservation_request_matches(
    request: &ReserveEvmNonceRequest,
    closure: &ReservationClosure,
) -> bool {
    request.nonce_domain == closure.request.nonce_domain
        && request.domain_activation_attestation == closure.request.domain_activation_attestation
        && request.submission_intent_id == closure.request.submission_intent_id
        && request.submission_semantics_digest == closure.request.submission_semantics_digest
        && request.transaction_intent == closure.request.transaction_intent
        && request.candidate_family == closure.request.candidate_family
        && request.observation_rounds == closure.request.observation_rounds
        && request.reservation_key == closure.request.reservation_key
}

fn candidate_progression_matches(
    request: &ActivateEvmCandidateRequest,
    reservation: &ReservationClosure,
    candidates: &[ActiveWalletCandidate],
) -> bool {
    // Authority re-derives the permit under the certified-order observation
    // frontier equal to the requested ordinal. Expansion only produces that
    // frontier after independent observation of every earlier activated
    // candidate; the permit eligibility preimage binds that fact (EVM-04).
    derive_exact_candidate_activation_permit(
        &reservation.reservation,
        candidates,
        request.next_candidate.candidate_ordinal,
        request.next_candidate.candidate_ordinal,
    )
    .is_ok_and(|expected| expected == request.activation_permit)
        && request.next_candidate.semantic_reservation_key
            == reservation.reservation.semantic_reservation_key
}

fn candidate_request_matches(
    request: &ActivateEvmCandidateRequest,
    retained: &RetainedCandidate,
) -> bool {
    request.next_candidate.candidate_descriptor_ref
        == retained.request.next_candidate.candidate_descriptor_ref
        && request.next_candidate.semantic_signer_id
            == retained.request.next_candidate.semantic_signer_id
        && request.next_candidate.transaction_hash
            == retained.request.next_candidate.transaction_hash
}

fn attested_candidate_semantics_match(
    request: &ActivateEvmCandidateRequest,
    reservation: &ReservationClosure,
) -> bool {
    let ordinal = usize::from(request.next_candidate.candidate_ordinal);
    let Some(fee) = reservation.candidate_family.candidates().get(ordinal) else {
        return false;
    };
    let Ok(envelope) = reservation
        .transaction_intent
        .unsigned_candidate(reservation.reservation.nonce, fee)
    else {
        return false;
    };
    let unsigned = UnsignedWalletCandidate {
        transaction_intent: (*reservation.transaction_intent).clone(),
        semantic_reservation_key: reservation.reservation.semantic_reservation_key.clone(),
        nonce: reservation.reservation.nonce,
        candidate_ordinal: request.next_candidate.candidate_ordinal,
        fee: fee.clone(),
        unsigned_candidate_digest: format!("{:#x}", envelope.signing_digest()),
    };
    let Ok(descriptor_ref) = canonical_wallet_reference(&unsigned) else {
        return false;
    };
    request.next_candidate.candidate_descriptor_ref == descriptor_ref
        && request.next_candidate.unsigned_candidate_digest == unsigned.unsigned_candidate_digest
        && request.next_candidate.semantic_signer_id
            == reservation.transaction_intent.semantic_signer_id()
        && request.next_candidate.signing_profile_contract_ref
            == *reservation
                .transaction_intent
                .signing_profile_contract_ref()
}

fn attested_candidate_matches(
    request: &ActivateEvmCandidateRequest,
    reservation: &ReservationClosure,
    candidates: &[ActiveWalletCandidate],
) -> bool {
    if !attested_candidate_semantics_match(request, reservation) {
        return false;
    }
    let descriptors: BTreeSet<_> = candidates
        .iter()
        .map(|candidate| &candidate.attested_candidate.candidate_descriptor_ref)
        .collect();
    let unsigned_digests: BTreeSet<_> = candidates
        .iter()
        .map(|candidate| {
            candidate
                .attested_candidate
                .unsigned_candidate_digest
                .as_str()
        })
        .collect();
    let transaction_hashes: BTreeSet<_> = candidates
        .iter()
        .map(|candidate| candidate.attested_candidate.transaction_hash.as_str())
        .collect();
    descriptors.len() == candidates.len()
        && unsigned_digests.len() == candidates.len()
        && transaction_hashes.len() == candidates.len()
        && !descriptors.contains(&request.next_candidate.candidate_descriptor_ref)
        && !unsigned_digests.contains(request.next_candidate.unsigned_candidate_digest.as_str())
        && !transaction_hashes.contains(request.next_candidate.transaction_hash.as_str())
}

// This deliberately reconstructs the domain cross-links inside the storage
// trust boundary instead of treating capability validation as authority.
fn completion_closure_matches(
    request: &CompleteEvmNonceRequest,
    reservation: &ReservationClosure,
    candidates: &[ActiveWalletCandidate],
) -> bool {
    let outcome = &request.canonical_terminal_outcome;
    request.nonce_domain.validate().is_ok()
        && request.completion_key.validate().is_ok()
        && derive_evm_nonce_completion_key(&reservation.reservation.semantic_reservation_key)
            .is_ok_and(|key| key == request.completion_key)
        && outcome.nonce_domain.validate().is_ok()
        && outcome.semantic_reservation_key.validate().is_ok()
        && outcome.submission_intent_id.validate().is_ok()
        && ContentDigest::from_str(&outcome.transaction_intent_digest).is_ok()
        && usize::from(outcome.winning_candidate_ordinal) < mfm_evm::EVM_WALLET_REPLACEMENT_LIMIT
        && outcome
            .winning_activation_evidence_ref
            .to_content_ref()
            .is_ok()
        && valid_evm_hash(&outcome.transaction_hash)
        && valid_evm_quantity(&outcome.inclusion_block_number)
        && valid_evm_hash(&outcome.inclusion_block_hash)
        && outcome
            .terminal_assurance_contract_ref
            .to_content_ref()
            .is_ok()
        && outcome.canonical_public_result == canonical_public_result(outcome.execution_disposition)
        && terminal_witnesses_match(&request.terminal_witnesses, outcome)
        && request.current_reservation == *reservation.reservation
        && request.nonce_domain == reservation.reservation.nonce_domain
        && outcome.nonce_domain == reservation.reservation.nonce_domain
        && outcome.semantic_reservation_key == reservation.reservation.semantic_reservation_key
        && outcome.submission_intent_id == reservation.reservation.submission_intent_id
        && outcome.transaction_intent_digest == reservation.reservation.transaction_intent_digest
        && outcome.nonce == reservation.reservation.nonce
        && outcome.terminal_assurance_contract_ref
            == *reservation
                .transaction_intent
                .terminal_assurance_contract_ref()
        && candidates.iter().any(|candidate| {
            candidate.attested_candidate.candidate_ordinal == outcome.winning_candidate_ordinal
                && candidate.attested_candidate.transaction_hash == outcome.transaction_hash
                && candidate.activation_evidence_ref == outcome.winning_activation_evidence_ref
        })
}

fn terminal_witnesses_match(
    witnesses: &TerminalWitnesses,
    outcome: &CanonicalTerminalOutcome,
) -> bool {
    let EvmTransactionLookupObservation::Found {
        transaction_hash,
        block_number: Some(transaction_block_number),
        block_hash: Some(transaction_block_hash),
    } = &witnesses.transaction
    else {
        return false;
    };
    let EvmReceiptLookupObservation::Found {
        transaction_hash: receipt_transaction_hash,
        block_number: receipt_block_number,
        block_hash: receipt_block_hash,
        status,
    } = &witnesses.receipt
    else {
        return false;
    };
    let Ok(finalized_number) = U256::from_str(&witnesses.finalized_head.block_number) else {
        return false;
    };
    let Ok(inclusion_number) = U256::from_str(&witnesses.inclusion_block.block_number) else {
        return false;
    };
    let expected_status = match outcome.execution_disposition {
        ExecutionDisposition::Succeeded => 1,
        ExecutionDisposition::Reverted => 0,
    };
    valid_evm_hash(transaction_hash)
        && valid_evm_quantity(transaction_block_number)
        && valid_evm_hash(transaction_block_hash)
        && valid_evm_hash(receipt_transaction_hash)
        && valid_evm_quantity(receipt_block_number)
        && valid_evm_hash(receipt_block_hash)
        && matches!(status, 0 | 1)
        && valid_evm_quantity(&witnesses.finalized_head.block_number)
        && valid_evm_hash(&witnesses.finalized_head.block_hash)
        && valid_evm_quantity(&witnesses.inclusion_block.block_number)
        && valid_evm_hash(&witnesses.inclusion_block.block_hash)
        && witnesses
            .terminal_assurance_contract_ref
            .to_content_ref()
            .is_ok()
        && transaction_hash == receipt_transaction_hash
        && transaction_hash == &outcome.transaction_hash
        && transaction_block_number == receipt_block_number
        && transaction_block_hash == receipt_block_hash
        && receipt_block_number == &witnesses.inclusion_block.block_number
        && receipt_block_hash == &witnesses.inclusion_block.block_hash
        && receipt_block_number == &outcome.inclusion_block_number
        && receipt_block_hash == &outcome.inclusion_block_hash
        && finalized_number >= inclusion_number
        && (finalized_number != inclusion_number
            || witnesses.finalized_head.block_hash == witnesses.inclusion_block.block_hash)
        && *status == expected_status
        && witnesses.terminal_assurance_contract_ref == outcome.terminal_assurance_contract_ref
        && witnesses.canonical_public_result == outcome.canonical_public_result
        && witnesses.canonical_public_result
            == canonical_public_result(outcome.execution_disposition)
        && canonical_wallet_reference(witnesses).is_ok()
}

fn canonical_public_result(disposition: ExecutionDisposition) -> &'static str {
    match disposition {
        ExecutionDisposition::Succeeded => "{\"execution_disposition\":\"succeeded\"}",
        ExecutionDisposition::Reverted => "{\"execution_disposition\":\"reverted\"}",
    }
}

fn valid_evm_hash(value: &str) -> bool {
    B256::from_str(value).is_ok_and(|hash| hash != B256::ZERO && value == format!("{hash:#x}"))
}

fn valid_evm_quantity(value: &str) -> bool {
    U256::from_str(value).is_ok_and(|quantity| value == quantity.to_string())
}

fn completion_request_matches(
    request: &CompleteEvmNonceRequest,
    retained: &RetainedCompletion,
) -> bool {
    request.nonce_domain == retained.request.nonce_domain
        && request.completion_key == retained.request.completion_key
        && request.current_reservation == retained.request.current_reservation
        && request.canonical_terminal_outcome == retained.request.canonical_terminal_outcome
}

async fn insert_domain_activation(
    connection: &mut PgConnection,
    request: &ReserveEvmNonceRequest,
    high_water: u64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO wallet_nonce_domains ( \
             wallet_nonce_domain_id, activation_record_ref, wallet_nonce_store_lineage_id, \
             activation_record_json, activation_attestation_json, local_high_water_nonce \
         ) VALUES ($1, $2, $3, $4, $5, $6::numeric)",
    )
    .bind(request.nonce_domain.as_str())
    .bind(reference_text(
        &request.domain_activation_attestation.activation_record_ref,
    )?)
    .bind(
        &request
            .domain_activation_attestation
            .current_schema_record
            .wallet_nonce_store_lineage_id,
    )
    .bind(canonical_json(
        &request.domain_activation_attestation.current_schema_record,
    )?)
    .bind(canonical_json(&request.domain_activation_attestation)?)
    .bind(high_water.to_string())
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}

async fn update_domain_projection(
    connection: &mut PgConnection,
    domain_id: &str,
    active_reservation_key: Option<&str>,
    current_resource_frontier_ref: &EvmWalletReference,
    current_incarnation_ref: &EvmWalletReference,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE wallet_nonce_domains SET active_reservation_key = $2, \
                current_resource_frontier_ref = $3, current_incarnation_ref = $4 \
         WHERE wallet_nonce_domain_id = $1",
    )
    .bind(domain_id)
    .bind(active_reservation_key)
    .bind(reference_text(current_resource_frontier_ref)?)
    .bind(reference_text(current_incarnation_ref)?)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    if result.rows_affected() != 1 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

async fn clear_active_reservation(connection: &mut PgConnection, domain_id: &str) -> Result<()> {
    let result = sqlx::query(
        "UPDATE wallet_nonce_domains SET active_reservation_key = NULL \
         WHERE wallet_nonce_domain_id = $1",
    )
    .bind(domain_id)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    if result.rows_affected() != 1 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

async fn update_high_water(
    connection: &mut PgConnection,
    domain_id: &str,
    high_water: u64,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE wallet_nonce_domains SET local_high_water_nonce = $2::numeric \
         WHERE wallet_nonce_domain_id = $1",
    )
    .bind(domain_id)
    .bind(high_water.to_string())
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    if result.rows_affected() != 1 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

async fn insert_reservation(
    connection: &mut PgConnection,
    request: &ReserveEvmNonceRequest,
    reservation: &ReservedWalletNonce,
    state_input_ref: &LexicalValueRef,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO wallet_nonce_reservations ( \
             semantic_reservation_key, wallet_nonce_domain_id, submission_intent_id, nonce, \
             transaction_intent_digest, candidate_family_ref, submission_semantics_digest, request_json, \
             transaction_intent_json, candidate_family_json, reservation_json, state_input_json \
         ) VALUES ($1, $2, $3, $4::numeric, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(request.reservation_key.as_str())
    .bind(request.nonce_domain.as_str())
    .bind(request.submission_intent_id.as_str())
    .bind(reservation.nonce.to_string())
    .bind(request.transaction_intent.digest())
    .bind(request.candidate_family.digest())
    .bind(request.submission_semantics_digest.as_str())
    .bind(canonical_json(request)?)
    .bind(canonical_json(&request.transaction_intent)?)
    .bind(canonical_json(&request.candidate_family)?)
    .bind(canonical_json(reservation)?)
    .bind(canonical_json(state_input_ref)?)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}

async fn insert_candidate(
    connection: &mut PgConnection,
    request: &ActivateEvmCandidateRequest,
    candidate: &ActiveWalletCandidate,
    state_input_ref: &LexicalValueRef,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO wallet_nonce_candidates ( \
             semantic_candidate_operation_key, semantic_reservation_key, candidate_ordinal, \
             request_json, active_candidate_json, state_input_json \
         ) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(request.candidate_operation_key.as_str())
    .bind(request.next_candidate.semantic_reservation_key.as_str())
    .bind(i32::from(request.next_candidate.candidate_ordinal))
    .bind(canonical_json(request)?)
    .bind(canonical_json(candidate)?)
    .bind(canonical_json(state_input_ref)?)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}

async fn insert_completion(
    connection: &mut PgConnection,
    request: &CompleteEvmNonceRequest,
    completion: &CompletedWalletNonce,
    state_input_ref: &LexicalValueRef,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO wallet_nonce_completions ( \
             semantic_completion_key, semantic_reservation_key, request_json, \
             canonical_terminal_outcome_json, completion_json, state_input_json \
         ) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(request.completion_key.as_str())
    .bind(
        request
            .current_reservation
            .semantic_reservation_key
            .as_str(),
    )
    .bind(canonical_json(request)?)
    .bind(canonical_json(&request.canonical_terminal_outcome)?)
    .bind(canonical_json(completion)?)
    .bind(canonical_json(state_input_ref)?)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::Unavailable)?;
    Ok(())
}
