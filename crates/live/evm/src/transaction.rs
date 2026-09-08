//! Explicit reservation, preparation, and execution adapters for EVM transaction States.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_capabilities::EffectCapabilityContract;
use mfm_evm::custody::{
    AuthorityError, EvmTransactionAuthority, ExactRawTransaction, NonceDomain, PreparedRecord,
    Reservation,
};
use mfm_evm::{
    Eip1559TransactionCommand, EvmAddress, EvmBlockAnchor, EvmChainInstance, EvmHash,
    EvmTransactionBinding, EvmTransactionEffect, EvmTransactionReceipt, EvmTransactionSettlement,
};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_runtime::{AdapterError, EffectAdapterOutcome, RuntimeAssemblyBuilder, RuntimeError};
use mfm_signing::{recover_public_key, Secp256k1Signer};

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

use mfm_evm::{
    EvmNonceReservationEffect, EvmTransactionPreparationEffect, PreparedEvmTransaction,
    PreparedEvmTransactionEvidence, ReservedEvmTransaction,
};

/// Registers the three transaction adapters under one public binding.
/// State implementations are registered separately by application composition.
pub fn register_evm_transaction_adapters(
    builder: &mut RuntimeAssemblyBuilder,
    binding: EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    authority: Arc<dyn EvmTransactionAuthority>,
    provider: Arc<dyn EvmTransactionProvider>,
) -> mfm_runtime::Result<()> {
    let purpose = StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID)
        .map_err(|_| RuntimeError::IncompatibleAssembly)?;
    if (&binding.authority_epoch) != authority.authority_epoch()
        || signer.purpose() != &purpose
        || ethereum_address(signer.public_key()) != binding.sender
    {
        return Err(RuntimeError::IncompatibleAssembly);
    }
    let reserve_binding = binding.clone();
    let reserve_authority = authority.clone();
    let reserve_provider = provider.clone();
    builder.register_effect_adapter::<EvmNonceReservationEffect, EvmTransactionBinding, _>(
        binding.clone(),
        move |id, reference, command| {
            let (id, reference, command) = (id.clone(), reference.clone(), command.clone());
            let (binding, authority, provider) = (
                reserve_binding.clone(),
                reserve_authority.clone(),
                reserve_provider.clone(),
            );
            Box::pin(async move {
                reserve_nonce(
                    &binding,
                    authority.as_ref(),
                    provider.as_ref(),
                    &id,
                    &reference,
                    &command,
                )
                .await
            })
        },
    )?;
    let prepare_binding = binding.clone();
    let prepare_authority = authority.clone();
    builder.register_effect_adapter::<EvmTransactionPreparationEffect, EvmTransactionBinding, _>(
        binding.clone(),
        move |id, _, command| {
            let (id, command) = (id.clone(), command.clone());
            let (binding, authority, signer) = (
                prepare_binding.clone(),
                prepare_authority.clone(),
                signer.clone(),
            );
            Box::pin(async move {
                prepare_transaction(&binding, authority.as_ref(), signer.as_ref(), &id, &command)
                    .await
            })
        },
    )?;
    builder.register_effect_adapter::<EvmTransactionEffect, EvmTransactionBinding, _>(
        binding.clone(),
        move |id, _, command| {
            let (id, command) = (id.clone(), command.clone());
            let (binding, authority, provider) =
                (binding.clone(), authority.clone(), provider.clone());
            Box::pin(async move {
                execute_transaction(
                    &binding,
                    authority.as_ref(),
                    provider.as_ref(),
                    &id,
                    &command,
                )
                .await
            })
        },
    )
}

fn check_binding(
    binding: &EvmTransactionBinding,
    authority: &dyn EvmTransactionAuthority,
    command: &Eip1559TransactionCommand,
) -> Result<(), AdapterError> {
    if command.binding() != binding || (&binding.authority_epoch) != authority.authority_epoch() {
        return Err(AdapterError::Internal);
    }
    Ok(())
}
async fn reserve_nonce(
    binding: &EvmTransactionBinding,
    authority: &dyn EvmTransactionAuthority,
    provider: &dyn EvmTransactionProvider,
    id: &EffectId,
    reference: &ContentRef,
    command: &Eip1559TransactionCommand,
) -> Result<EffectAdapterOutcome<Reservation>, AdapterError> {
    check_binding(binding, authority, command)?;
    let reservation = match authority.load(id).await.map_err(map_authority_error)? {
        Some(loaded) => loaded.reservation,
        None => {
            verify_chain(command, provider).await?;
            let observed = provider.pending_nonce(&binding.sender).await?;
            authority
                .reserve_or_compare(id, reference, &NonceDomain::from_binding(binding), observed)
                .await
                .map_err(map_authority_error)?
        }
    };
    validate_reservation(
        &reservation,
        id,
        reference,
        &NonceDomain::from_binding(binding),
    )?;
    EvmNonceReservationEffect::bind_evidence(id, command, &reservation)
        .map_err(|_| AdapterError::Internal)?;
    Ok(EffectAdapterOutcome::Settled(reservation))
}
async fn load_reserved(
    authority: &dyn EvmTransactionAuthority,
    command: &ReservedEvmTransaction,
) -> Result<Option<PreparedRecord>, AdapterError> {
    let loaded = authority
        .load(command.reservation().effect_id())
        .await
        .map_err(map_authority_error)?
        .ok_or(AdapterError::Internal)?;
    if &loaded.reservation != command.reservation() {
        return Err(AdapterError::Internal);
    }
    Ok(loaded.prepared)
}
async fn qualify_prepared(
    command: &ReservedEvmTransaction,
    prepared: PreparedRecord,
) -> Result<PreparedRecord, AdapterError> {
    let command = command.clone();
    tokio::task::spawn_blocking(move || {
        validate_signed_transaction(
            command.command(),
            command.reservation().nonce(),
            prepared.transaction_hash(),
            prepared.raw_transaction(),
            &command.command().binding().sender,
        )
        .map_err(|_| AdapterError::Internal)?;
        Ok(prepared)
    })
    .await
    .map_err(|_| AdapterError::Internal)?
}
async fn prepare_transaction(
    binding: &EvmTransactionBinding,
    authority: &dyn EvmTransactionAuthority,
    signer: &dyn Secp256k1Signer,
    id: &EffectId,
    command: &ReservedEvmTransaction,
) -> Result<EffectAdapterOutcome<PreparedEvmTransactionEvidence>, AdapterError> {
    check_binding(binding, authority, command.command())?;
    let prepared = match load_reserved(authority, command).await? {
        Some(prepared) => qualify_prepared(command, prepared).await?,
        None => {
            let owned = command.clone();
            let digest = tokio::task::spawn_blocking(move || {
                transaction_signing_digest(owned.command(), owned.reservation().nonce())
            })
            .await
            .map_err(|_| AdapterError::Internal)?
            .map_err(|_| AdapterError::Internal)?;
            let signature = signer
                .sign(digest)
                .await
                .map_err(|_| AdapterError::Unavailable)?;
            let owned = command.clone();
            let candidate = tokio::task::spawn_blocking(move || {
                let recovered =
                    recover_public_key(digest, &signature).map_err(|_| AdapterError::Internal)?;
                if ethereum_address(&recovered) != owned.command().binding().sender {
                    return Err(AdapterError::Internal);
                }
                let (raw, hash) =
                    signed_transaction(owned.command(), owned.reservation().nonce(), signature)
                        .map_err(|_| AdapterError::Internal)?;
                Ok(PreparedRecord::new(hash, raw))
            })
            .await
            .map_err(|_| AdapterError::Internal)??;
            let winner = authority
                .retain_prepared(command.reservation(), &candidate)
                .await
                .map_err(map_authority_error)?;
            if winner == candidate {
                winner
            } else {
                qualify_prepared(command, winner).await?
            }
        }
    };
    Ok(EffectAdapterOutcome::Settled(
        PreparedEvmTransactionEvidence {
            effect_id: id.clone(),
            transaction_hash: prepared.transaction_hash().clone(),
        },
    ))
}
async fn execute_transaction(
    binding: &EvmTransactionBinding,
    authority: &dyn EvmTransactionAuthority,
    provider: &dyn EvmTransactionProvider,
    id: &EffectId,
    command: &PreparedEvmTransaction,
) -> Result<EffectAdapterOutcome<EvmTransactionSettlement>, AdapterError> {
    check_binding(binding, authority, command.reserved().command())?;
    let prepared = load_reserved(authority, command.reserved())
        .await?
        .ok_or(AdapterError::Internal)?;
    if prepared.transaction_hash() != command.transaction_hash() {
        return Err(AdapterError::Internal);
    }
    let prepared = qualify_prepared(command.reserved(), prepared).await?;
    verify_chain(command.reserved().command(), provider).await?;
    let Some(receipt) = provider.receipt(command.transaction_hash()).await? else {
        let submitted = provider.submit_raw(prepared.raw_transaction()).await?;
        if &submitted != command.transaction_hash() {
            return Err(AdapterError::Unavailable);
        }
        return Ok(EffectAdapterOutcome::Pending);
    };
    let evidence = validate_receipt(
        id,
        &receipt,
        &prepared,
        command.reserved().command(),
        command.reserved().reservation().nonce(),
    )?;
    let canonical = provider
        .canonical_block(&receipt.block_anchor().number)
        .await?;
    if &canonical != receipt.block_anchor() {
        return Err(AdapterError::Unavailable);
    }
    EvmTransactionEffect::bind_evidence(id, command, &evidence)
        .map_err(|_| AdapterError::Internal)?;
    Ok(EffectAdapterOutcome::Settled(evidence))
}
fn validate_receipt(
    effect_id: &EffectId,
    receipt: &ProviderReceipt,
    prepared: &PreparedRecord,
    command: &Eip1559TransactionCommand,
    nonce: u64,
) -> Result<EvmTransactionSettlement, AdapterError> {
    if receipt.transaction_hash() != prepared.transaction_hash()
        || receipt.sender() != (&command.binding().sender)
    {
        return Err(AdapterError::Internal);
    }
    match (command.to(), receipt.result()) {
        (None, ProviderReceiptResult::SuccessCreate { contract_address }) => {
            if contract_address != &create_address(&command.binding().sender, nonce) {
                return Err(AdapterError::Internal);
            }
            Ok(EvmTransactionSettlement::created(
                effect_id.clone(),
                nonce,
                EvmTransactionReceipt {
                    block_anchor: receipt.block_anchor().clone(),
                    transaction_hash: prepared.transaction_hash().clone(),
                },
                contract_address.clone(),
            ))
        }
        (None, ProviderReceiptResult::RevertedCreate) => Ok(EvmTransactionSettlement::reverted(
            effect_id.clone(),
            nonce,
            EvmTransactionReceipt {
                block_anchor: receipt.block_anchor().clone(),
                transaction_hash: prepared.transaction_hash().clone(),
            },
        )),
        (Some(to), ProviderReceiptResult::SuccessCall { target }) if target == to => {
            Ok(EvmTransactionSettlement::called(
                effect_id.clone(),
                nonce,
                EvmTransactionReceipt {
                    block_anchor: receipt.block_anchor().clone(),
                    transaction_hash: prepared.transaction_hash().clone(),
                },
            ))
        }
        (Some(to), ProviderReceiptResult::RevertedCall { target }) if target == to => {
            Ok(EvmTransactionSettlement::reverted(
                effect_id.clone(),
                nonce,
                EvmTransactionReceipt {
                    block_anchor: receipt.block_anchor().clone(),
                    transaction_hash: prepared.transaction_hash().clone(),
                },
            ))
        }
        _ => Err(AdapterError::Internal),
    }
}

async fn verify_chain(
    command: &Eip1559TransactionCommand,
    provider: &dyn EvmTransactionProvider,
) -> Result<(), AdapterError> {
    let expected = &command.binding().route.chain_instance;
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

const fn map_authority_error(error: AuthorityError) -> AdapterError {
    match error {
        AuthorityError::Unavailable => AdapterError::Unavailable,
        AuthorityError::Internal => AdapterError::Internal,
    }
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;
