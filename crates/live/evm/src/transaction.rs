//! Durable signer/authority/provider orchestration for EVM transaction Effects.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_evm::{
    Eip1559TransactionCommand, EvmAddress, EvmBlockAnchor, EvmChainInstance, EvmHash,
    EvmTransactionBinding, EvmTransactionEffect, EvmTransactionOutcome, EvmTransactionReceipt,
    EvmTransactionSettlement,
};
use mfm_evm_transaction_authority::{
    AuthorityError, AuthorityState, EvmTransactionAuthority, ExactRawTransaction, NonceDomain,
    PreparedRecord, Reservation,
};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_runtime::{AdapterError, EffectAdapterOutcome, RuntimeAssemblyBuilder};
use mfm_signing::{recover_public_key, Secp256k1PublicKey, Secp256k1Signer};

use crate::codec::{
    create_address, signed_transaction, transaction_signing_digest, validate_signed_transaction,
};
use crate::ethereum_address;

/// Exact signing-purpose identity for fixed EIP-1559 transactions.
pub const EVM_EIP1559_SIGNING_PURPOSE_ID: &str = "mfm.evm.sign-eip1559@1";

/// Borrowing boxed transaction-provider operation.
pub type EvmTransactionProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AdapterError>> + Send + 'a>>;

/// Closed provider receipt result preserving create-or-call shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderReceiptResult {
    /// A creation transaction succeeded with this contract address.
    SuccessCreate {
        /// Receipt-returned created contract address.
        contract_address: EvmAddress,
    },
    /// An ordinary call succeeded against this target.
    SuccessCall {
        /// Receipt-returned call target.
        target: EvmAddress,
    },
    /// A creation transaction reverted.
    RevertedCreate,
    /// An ordinary call reverted against this target.
    RevertedCall {
        /// Receipt-returned call target.
        target: EvmAddress,
    },
}

/// Checked transaction receipt observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReceipt {
    transaction_hash: EvmHash,
    sender: EvmAddress,
    result: ProviderReceiptResult,
    block_anchor: EvmBlockAnchor,
}

impl ProviderReceipt {
    /// Constructs one fully checked typed receipt observation.
    pub fn new(
        transaction_hash: EvmHash,
        sender: EvmAddress,
        result: ProviderReceiptResult,
        block_anchor: EvmBlockAnchor,
    ) -> Self {
        Self {
            transaction_hash,
            sender,
            result,
            block_anchor,
        }
    }

    /// Returns the receipt transaction hash.
    pub const fn transaction_hash(&self) -> &EvmHash {
        &self.transaction_hash
    }

    /// Returns the receipt sender.
    pub const fn sender(&self) -> &EvmAddress {
        &self.sender
    }

    /// Returns the closed create-or-call result.
    pub const fn result(&self) -> &ProviderReceiptResult {
        &self.result
    }

    /// Returns the receipt block identity.
    pub const fn block_anchor(&self) -> &EvmBlockAnchor {
        &self.block_anchor
    }
}

/// Minimal transaction-only provider facet.
pub trait EvmTransactionProvider: Send + Sync + 'static {
    /// Observes the endpoint's chain ID and genesis block hash.
    fn chain_instance(&self) -> EvmTransactionProviderFuture<'_, EvmChainInstance>;

    /// Observes the sender's pending transaction count.
    fn pending_nonce<'a>(&'a self, sender: &'a EvmAddress)
        -> EvmTransactionProviderFuture<'a, u64>;

    /// Observes one exact transaction receipt, where absence is expected and retryable.
    fn receipt<'a>(
        &'a self,
        transaction_hash: &'a EvmHash,
    ) -> EvmTransactionProviderFuture<'a, Option<ProviderReceipt>>;

    /// Observes the current canonical identity of one exact block number.
    fn canonical_block<'a>(
        &'a self,
        block_number: &'a mfm_evm::EvmU256,
    ) -> EvmTransactionProviderFuture<'a, EvmBlockAnchor>;

    /// Submits exact retained signed bytes and returns the node's transaction hash.
    fn submit_raw<'a>(
        &'a self,
        raw_transaction: &'a ExactRawTransaction,
    ) -> EvmTransactionProviderFuture<'a, EvmHash>;
}

/// Registers one durable EVM transaction Effect callback under its complete binding.
pub fn register_evm_transaction_effect(
    builder: &mut RuntimeAssemblyBuilder,
    binding: EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    authority: Arc<dyn EvmTransactionAuthority>,
    provider: Arc<dyn EvmTransactionProvider>,
) -> mfm_runtime::Result<()> {
    let callback_binding = binding.clone();
    builder.register_effect_adapter::<EvmTransactionEffect, EvmTransactionBinding, _>(
        binding,
        move |effect_id, command_value_ref, command| {
            let binding = callback_binding.clone();
            let effect_id = effect_id.clone();
            let command_value_ref = command_value_ref.clone();
            let command = command.clone();
            let signer = Arc::clone(&signer);
            let authority = Arc::clone(&authority);
            let provider = Arc::clone(&provider);
            Box::pin(async move {
                execute_transaction(
                    &binding,
                    &effect_id,
                    &command_value_ref,
                    &command,
                    signer.as_ref(),
                    authority.as_ref(),
                    provider.as_ref(),
                )
                .await
            })
        },
    )
}

async fn execute_transaction(
    binding: &EvmTransactionBinding,
    effect_id: &EffectId,
    command_value_ref: &ContentRef,
    command: &Eip1559TransactionCommand,
    signer: &dyn Secp256k1Signer,
    authority: &dyn EvmTransactionAuthority,
    provider: &dyn EvmTransactionProvider,
) -> Result<EffectAdapterOutcome<EvmTransactionSettlement>, AdapterError> {
    let local = validate_local(binding, command_value_ref, command, signer, authority)?;
    let state = authority
        .load(effect_id, &local.command_value_ref)
        .await
        .map_err(map_authority_error)?;

    let (prepared, chain_verified) = match state {
        Some(AuthorityState::Settled(settled)) => {
            validate_reservation(
                settled.prepared().reservation(),
                effect_id,
                &local.command_value_ref,
                &local.domain,
            )?;
            validate_prepared(settled.prepared(), command, &local)?;
            validate_settlement(
                settled.evidence(),
                effect_id,
                settled.prepared(),
                command,
                &local,
            )?;
            return Ok(EffectAdapterOutcome::Settled(settled.evidence().clone()));
        }
        Some(AuthorityState::Prepared(prepared)) => {
            validate_reservation(
                prepared.reservation(),
                effect_id,
                &local.command_value_ref,
                &local.domain,
            )?;
            validate_prepared(&prepared, command, &local)?;
            (prepared, false)
        }
        Some(AuthorityState::Reserved(reservation)) => {
            validate_reservation(
                &reservation,
                effect_id,
                &local.command_value_ref,
                &local.domain,
            )?;
            (
                prepare(effect_id, command, &local, reservation, signer, authority).await?,
                false,
            )
        }
        None => {
            verify_chain(command, provider).await?;
            let pending = provider.pending_nonce(&local.sender).await?;
            let reservation = authority
                .reserve_or_compare(effect_id, &local.command_value_ref, &local.domain, pending)
                .await
                .map_err(map_authority_error)?;
            validate_reservation(
                &reservation,
                effect_id,
                &local.command_value_ref,
                &local.domain,
            )?;
            (
                prepare(effect_id, command, &local, reservation, signer, authority).await?,
                true,
            )
        }
    };

    reconcile(
        effect_id,
        command,
        &local,
        &prepared,
        chain_verified,
        authority,
        provider,
    )
    .await
}

struct LocalExecution {
    command_value_ref: ContentRef,
    domain: NonceDomain,
    public_key: Secp256k1PublicKey,
    sender: EvmAddress,
}

fn validate_local(
    binding: &EvmTransactionBinding,
    command_value_ref: &ContentRef,
    command: &Eip1559TransactionCommand,
    signer: &dyn Secp256k1Signer,
    authority: &dyn EvmTransactionAuthority,
) -> Result<LocalExecution, AdapterError> {
    if command.binding() != binding {
        return Err(AdapterError::Internal);
    }
    if binding.authority_epoch() != authority.authority_epoch() {
        return Err(AdapterError::Internal);
    }
    let purpose =
        StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID).map_err(|_| AdapterError::Internal)?;
    if signer.purpose() != &purpose {
        return Err(AdapterError::Internal);
    }
    let public_key = *signer.public_key();
    let sender = ethereum_address(&public_key);
    if &sender != binding.sender() {
        return Err(AdapterError::Internal);
    }
    Ok(LocalExecution {
        command_value_ref: command_value_ref.clone(),
        domain: NonceDomain::new(
            binding.authority_epoch().clone(),
            binding.route().chain_instance().clone(),
            sender.clone(),
        ),
        public_key,
        sender,
    })
}

async fn prepare(
    effect_id: &EffectId,
    command: &Eip1559TransactionCommand,
    local: &LocalExecution,
    reservation: Reservation,
    signer: &dyn Secp256k1Signer,
    authority: &dyn EvmTransactionAuthority,
) -> Result<PreparedRecord, AdapterError> {
    let digest = transaction_signing_digest(command, reservation.nonce())
        .map_err(|_| AdapterError::Internal)?;
    let signature = signer
        .sign(digest)
        .await
        .map_err(|_| AdapterError::Unavailable)?;
    let recovered = recover_public_key(digest, signature).map_err(|_| AdapterError::Internal)?;
    if recovered != local.public_key || ethereum_address(&recovered) != local.sender {
        return Err(AdapterError::Internal);
    }
    let (raw, transaction_hash) = signed_transaction(command, reservation.nonce(), signature)
        .map_err(|_| AdapterError::Internal)?;
    validate_signed_transaction(
        command,
        reservation.nonce(),
        &transaction_hash,
        &raw,
        &local.public_key,
        &local.sender,
    )
    .map_err(|_| AdapterError::Internal)?;
    let prepared = authority
        .retain_prepared(effect_id, &local.command_value_ref, &transaction_hash, &raw)
        .await
        .map_err(map_authority_error)?;
    validate_reservation(
        prepared.reservation(),
        effect_id,
        &local.command_value_ref,
        &local.domain,
    )?;
    validate_prepared(&prepared, command, local)?;
    Ok(prepared)
}

async fn reconcile(
    effect_id: &EffectId,
    command: &Eip1559TransactionCommand,
    local: &LocalExecution,
    prepared: &PreparedRecord,
    chain_verified: bool,
    authority: &dyn EvmTransactionAuthority,
    provider: &dyn EvmTransactionProvider,
) -> Result<EffectAdapterOutcome<EvmTransactionSettlement>, AdapterError> {
    if !chain_verified {
        verify_chain(command, provider).await?;
    }
    let Some(receipt) = provider.receipt(prepared.transaction_hash()).await? else {
        let submitted = provider.submit_raw(prepared.raw_transaction()).await?;
        if &submitted != prepared.transaction_hash() {
            return Err(AdapterError::Unavailable);
        }
        return Ok(EffectAdapterOutcome::Pending);
    };
    let evidence = validate_receipt(effect_id, &receipt, prepared, command, local)?;
    let canonical = provider
        .canonical_block(receipt.block_anchor().number())
        .await?;
    if &canonical != receipt.block_anchor() {
        return Err(AdapterError::Unavailable);
    }

    let settled = authority
        .retain_settlement(effect_id, &local.command_value_ref, &evidence)
        .await
        .map_err(map_authority_error)?;
    validate_reservation(
        settled.prepared().reservation(),
        effect_id,
        &local.command_value_ref,
        &local.domain,
    )?;
    validate_prepared(settled.prepared(), command, local)?;
    validate_settlement(
        settled.evidence(),
        effect_id,
        settled.prepared(),
        command,
        local,
    )?;
    Ok(EffectAdapterOutcome::Settled(settled.evidence().clone()))
}

fn validate_receipt(
    effect_id: &EffectId,
    receipt: &ProviderReceipt,
    prepared: &PreparedRecord,
    command: &Eip1559TransactionCommand,
    local: &LocalExecution,
) -> Result<EvmTransactionSettlement, AdapterError> {
    if receipt.transaction_hash() != prepared.transaction_hash()
        || receipt.sender() != &local.sender
    {
        return Err(AdapterError::Unavailable);
    }
    match (command.to(), receipt.result()) {
        (None, ProviderReceiptResult::SuccessCreate { contract_address }) => {
            if contract_address != &create_address(&local.sender, prepared.reservation().nonce()) {
                return Err(AdapterError::Unavailable);
            }
            Ok(EvmTransactionSettlement::created(
                effect_id.clone(),
                prepared.reservation().nonce(),
                EvmTransactionReceipt::new(
                    receipt.block_anchor().clone(),
                    prepared.transaction_hash().clone(),
                ),
                contract_address.clone(),
            ))
        }
        (None, ProviderReceiptResult::RevertedCreate) => Ok(EvmTransactionSettlement::reverted(
            effect_id.clone(),
            prepared.reservation().nonce(),
            EvmTransactionReceipt::new(
                receipt.block_anchor().clone(),
                prepared.transaction_hash().clone(),
            ),
        )),
        (Some(to), ProviderReceiptResult::SuccessCall { target }) if target == to => {
            Ok(EvmTransactionSettlement::called(
                effect_id.clone(),
                prepared.reservation().nonce(),
                EvmTransactionReceipt::new(
                    receipt.block_anchor().clone(),
                    prepared.transaction_hash().clone(),
                ),
            ))
        }
        (Some(to), ProviderReceiptResult::RevertedCall { target }) if target == to => {
            Ok(EvmTransactionSettlement::reverted(
                effect_id.clone(),
                prepared.reservation().nonce(),
                EvmTransactionReceipt::new(
                    receipt.block_anchor().clone(),
                    prepared.transaction_hash().clone(),
                ),
            ))
        }
        _ => Err(AdapterError::Unavailable),
    }
}

async fn verify_chain(
    command: &Eip1559TransactionCommand,
    provider: &dyn EvmTransactionProvider,
) -> Result<(), AdapterError> {
    let expected = command.binding().route().chain_instance();
    let observed = provider.chain_instance().await?;
    if &observed != expected {
        return Err(AdapterError::Internal);
    }
    Ok(())
}

fn validate_reservation(
    reservation: &Reservation,
    effect_id: &EffectId,
    command_ref: &ContentRef,
    domain: &NonceDomain,
) -> Result<(), AdapterError> {
    if reservation.effect_id() != effect_id
        || reservation.command_ref() != command_ref
        || reservation.domain() != domain
    {
        return Err(AdapterError::Internal);
    }
    Ok(())
}

fn validate_prepared(
    prepared: &PreparedRecord,
    command: &Eip1559TransactionCommand,
    local: &LocalExecution,
) -> Result<(), AdapterError> {
    validate_signed_transaction(
        command,
        prepared.reservation().nonce(),
        prepared.transaction_hash(),
        prepared.raw_transaction(),
        &local.public_key,
        &local.sender,
    )
    .map_err(|_| AdapterError::Internal)
}

fn validate_settlement(
    evidence: &EvmTransactionSettlement,
    effect_id: &EffectId,
    prepared: &PreparedRecord,
    command: &Eip1559TransactionCommand,
    local: &LocalExecution,
) -> Result<(), AdapterError> {
    if evidence.effect_id() != effect_id
        || evidence.nonce() != prepared.reservation().nonce()
        || evidence.transaction_hash() != prepared.transaction_hash()
        || !matches!(
            (command.to(), evidence.outcome()),
            (
                None,
                EvmTransactionOutcome::Created { .. } | EvmTransactionOutcome::Reverted
            ) | (
                Some(_),
                EvmTransactionOutcome::Called | EvmTransactionOutcome::Reverted
            )
        )
    {
        return Err(AdapterError::Internal);
    }
    if let EvmTransactionOutcome::Created { created_address } = evidence.outcome() {
        let expected = create_address(&local.sender, prepared.reservation().nonce());
        if created_address != &expected {
            return Err(AdapterError::Internal);
        }
    }
    Ok(())
}

const fn map_authority_error(error: AuthorityError) -> AdapterError {
    match error {
        AuthorityError::Unavailable => AdapterError::Unavailable,
        AuthorityError::Internal => AdapterError::Internal,
    }
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;
