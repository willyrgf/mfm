//! Durable signer/authority/provider orchestration for EVM transaction Effects.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_capabilities::EffectCapabilityContract;
use mfm_evm::{
    Eip1559TransactionCommand, EvmAddress, EvmBlockAnchor, EvmChainInstance, EvmHash,
    EvmTransactionBinding, EvmTransactionEffect, EvmTransactionOutcome, EvmTransactionReceipt,
    EvmTransactionSettlement,
};
use mfm_evm_transaction_authority::{
    AuthorityError, AuthorityState, EvmTransactionAuthority, ExactRawTransaction, NonceDomain,
    PreparedRecord, Reservation, SettledRecord,
};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_runtime::{AdapterError, EffectAdapterOutcome, RuntimeAssemblyBuilder, RuntimeError};
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
    let executor = Arc::new(CheckedExecutor::new(
        binding.clone(),
        signer,
        authority,
        provider,
    )?);
    builder.register_effect_adapter::<EvmTransactionEffect, EvmTransactionBinding, _>(
        binding,
        move |effect_id, command_value_ref, command| {
            let effect_id = effect_id.clone();
            let command_value_ref = command_value_ref.clone();
            let command = command.clone();
            let executor = Arc::clone(&executor);
            Box::pin(async move {
                execute_transaction(&executor, &effect_id, &command_value_ref, &command).await
            })
        },
    )
}

struct CheckedExecutor {
    binding: EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    authority: Arc<dyn EvmTransactionAuthority>,
    provider: Arc<dyn EvmTransactionProvider>,
    domain: NonceDomain,
    public_key: Secp256k1PublicKey,
    sender: EvmAddress,
}

impl CheckedExecutor {
    fn new(
        binding: EvmTransactionBinding,
        signer: Arc<dyn Secp256k1Signer>,
        authority: Arc<dyn EvmTransactionAuthority>,
        provider: Arc<dyn EvmTransactionProvider>,
    ) -> mfm_runtime::Result<Self> {
        if binding.authority_epoch() != authority.authority_epoch() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let purpose = StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID)
            .map_err(|_| RuntimeError::IncompatibleAssembly)?;
        if signer.purpose() != &purpose {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let public_key = *signer.public_key();
        let sender = ethereum_address(&public_key);
        if &sender != binding.sender() {
            return Err(RuntimeError::IncompatibleAssembly);
        }
        let domain = NonceDomain::new(
            binding.authority_epoch().clone(),
            binding.route().chain_instance().clone(),
            sender.clone(),
        );
        Ok(Self {
            binding,
            signer,
            authority,
            provider,
            domain,
            public_key,
            sender,
        })
    }
}

enum ResumeState {
    Absent,
    Reserved(Reservation),
    Prepared(PreparedRecord),
}

async fn execute_transaction(
    executor: &CheckedExecutor,
    effect_id: &EffectId,
    command_value_ref: &ContentRef,
    command: &Eip1559TransactionCommand,
) -> Result<EffectAdapterOutcome<EvmTransactionSettlement>, AdapterError> {
    if command.binding() != &executor.binding {
        return Err(AdapterError::Internal);
    }
    let state = executor
        .authority
        .load(effect_id)
        .await
        .map_err(map_authority_error)?;

    let resume = match state {
        Some(AuthorityState::Settled(settled)) => {
            validate_settled(executor, effect_id, command_value_ref, command, &settled)?;
            return Ok(EffectAdapterOutcome::Settled(settled.evidence().clone()));
        }
        Some(AuthorityState::Prepared(prepared)) => {
            validate_reservation(
                prepared.reservation(),
                effect_id,
                command_value_ref,
                &executor.domain,
            )?;
            validate_prepared(&prepared, command, executor)?;
            ResumeState::Prepared(prepared)
        }
        Some(AuthorityState::Reserved(reservation)) => {
            validate_reservation(&reservation, effect_id, command_value_ref, &executor.domain)?;
            ResumeState::Reserved(reservation)
        }
        None => ResumeState::Absent,
    };

    verify_chain(command, executor.provider.as_ref()).await?;

    let prepared = match resume {
        ResumeState::Prepared(prepared) => prepared,
        ResumeState::Reserved(reservation) => prepare(command, executor, reservation).await?,
        ResumeState::Absent => {
            let pending = executor.provider.pending_nonce(&executor.sender).await?;
            let reservation = executor
                .authority
                .reserve_or_compare(effect_id, command_value_ref, &executor.domain, pending)
                .await
                .map_err(map_authority_error)?;
            validate_reservation(&reservation, effect_id, command_value_ref, &executor.domain)?;
            prepare(command, executor, reservation).await?
        }
    };

    reconcile(effect_id, command, executor, &prepared).await
}

async fn prepare(
    command: &Eip1559TransactionCommand,
    executor: &CheckedExecutor,
    reservation: Reservation,
) -> Result<PreparedRecord, AdapterError> {
    let digest = transaction_signing_digest(command, reservation.nonce())
        .map_err(|_| AdapterError::Internal)?;
    let signature = executor
        .signer
        .sign(digest)
        .await
        .map_err(|_| AdapterError::Unavailable)?;
    let recovered = recover_public_key(digest, signature).map_err(|_| AdapterError::Internal)?;
    if recovered != executor.public_key || ethereum_address(&recovered) != executor.sender {
        return Err(AdapterError::Internal);
    }
    let (raw, transaction_hash) = signed_transaction(command, reservation.nonce(), signature)
        .map_err(|_| AdapterError::Internal)?;
    let candidate = PreparedRecord::new(reservation, transaction_hash, raw);
    executor
        .authority
        .retain_prepared(&candidate)
        .await
        .map_err(map_authority_error)?;
    Ok(candidate)
}

async fn reconcile(
    effect_id: &EffectId,
    command: &Eip1559TransactionCommand,
    executor: &CheckedExecutor,
    prepared: &PreparedRecord,
) -> Result<EffectAdapterOutcome<EvmTransactionSettlement>, AdapterError> {
    let Some(receipt) = executor
        .provider
        .receipt(prepared.transaction_hash())
        .await?
    else {
        let submitted = executor
            .provider
            .submit_raw(prepared.raw_transaction())
            .await?;
        if &submitted != prepared.transaction_hash() {
            return Err(AdapterError::Unavailable);
        }
        return Ok(EffectAdapterOutcome::Pending);
    };
    let evidence = validate_receipt(effect_id, &receipt, prepared, command, executor)?;
    let canonical = executor
        .provider
        .canonical_block(receipt.block_anchor().number())
        .await?;
    if &canonical != receipt.block_anchor() {
        return Err(AdapterError::Unavailable);
    }
    let candidate = SettledRecord::new(prepared.clone(), evidence).map_err(map_authority_error)?;
    validate_settlement(candidate.evidence(), effect_id, prepared, command, executor)?;
    executor
        .authority
        .retain_settlement(&candidate)
        .await
        .map_err(map_authority_error)?;
    Ok(EffectAdapterOutcome::Settled(candidate.evidence().clone()))
}

fn validate_receipt(
    effect_id: &EffectId,
    receipt: &ProviderReceipt,
    prepared: &PreparedRecord,
    command: &Eip1559TransactionCommand,
    executor: &CheckedExecutor,
) -> Result<EvmTransactionSettlement, AdapterError> {
    if receipt.transaction_hash() != prepared.transaction_hash()
        || receipt.sender() != &executor.sender
    {
        return Err(AdapterError::Internal);
    }
    match (command.to(), receipt.result()) {
        (None, ProviderReceiptResult::SuccessCreate { contract_address }) => {
            if contract_address != &create_address(&executor.sender, prepared.reservation().nonce())
            {
                return Err(AdapterError::Internal);
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
        _ => Err(AdapterError::Internal),
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
    command_value_ref: &ContentRef,
    domain: &NonceDomain,
) -> Result<(), AdapterError> {
    if reservation.effect_id() != effect_id
        || reservation.command_value_ref() != command_value_ref
        || reservation.domain() != domain
    {
        return Err(AdapterError::Internal);
    }
    Ok(())
}

fn validate_prepared(
    prepared: &PreparedRecord,
    command: &Eip1559TransactionCommand,
    executor: &CheckedExecutor,
) -> Result<(), AdapterError> {
    validate_signed_transaction(
        command,
        prepared.reservation().nonce(),
        prepared.transaction_hash(),
        prepared.raw_transaction(),
        &executor.public_key,
        &executor.sender,
    )
    .map_err(|_| AdapterError::Internal)
}

fn validate_settlement(
    evidence: &EvmTransactionSettlement,
    effect_id: &EffectId,
    prepared: &PreparedRecord,
    command: &Eip1559TransactionCommand,
    executor: &CheckedExecutor,
) -> Result<(), AdapterError> {
    EvmTransactionEffect::bind_evidence(effect_id, command, evidence)
        .map_err(|_| AdapterError::Internal)?;
    if evidence.nonce() != prepared.reservation().nonce()
        || evidence.transaction_hash() != prepared.transaction_hash()
    {
        return Err(AdapterError::Internal);
    }
    if let EvmTransactionOutcome::Created { created_address } = evidence.outcome() {
        let expected = create_address(&executor.sender, prepared.reservation().nonce());
        if created_address != &expected {
            return Err(AdapterError::Internal);
        }
    }
    Ok(())
}

fn validate_settled(
    executor: &CheckedExecutor,
    effect_id: &EffectId,
    command_value_ref: &ContentRef,
    command: &Eip1559TransactionCommand,
    settled: &SettledRecord,
) -> Result<(), AdapterError> {
    validate_reservation(
        settled.prepared().reservation(),
        effect_id,
        command_value_ref,
        &executor.domain,
    )?;
    validate_prepared(settled.prepared(), command, executor)?;
    validate_settlement(
        settled.evidence(),
        effect_id,
        settled.prepared(),
        command,
        executor,
    )
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
