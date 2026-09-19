//! Native supporting-State contract evidence; the request fixture is not a product lifecycle caller.
use mfm_capabilities::EffectImplementation;
use mfm_chain::transaction::{
    ConfigurationApplied, PreparedTransaction, TransactionEffect, TransactionRequest,
};
use mfm_evm::*;
use mfm_ids::{DigestBytes, EffectId};
use mfm_program::{EffectState, ProposedStateOutcome};
use mfm_program_derive::MfmValue;
use mfm_values::Object;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Request {
    command: Eip1559TransactionCommand,
}
impl TransactionRequest for Request {
    type Applied = ConfigurationApplied;
}
impl EvmTransactionRecipe for Request {
    fn command(&self) -> Result<Eip1559TransactionCommand, mfm_capabilities::CallbackFailure> {
        Ok(self.command.clone())
    }
    fn applied(
        &self,
        _: &EvmTransactionSettlement,
    ) -> Result<ConfigurationApplied, mfm_capabilities::CallbackFailure> {
        Ok(ConfigurationApplied)
    }
}

#[test]
fn native_preparation_keeps_request_and_checks_command_binding_and_implementation() {
    let endpoint = Object::from_value(&EvmU256::new("1").unwrap()).unwrap();
    let binding = EvmTransactionBinding {
        authority_epoch: EvmAuthorityEpoch::new([1; 32]),
        route: EvmTransactionRoute {
            chain_instance: EvmChainInstance {
                chain_id: NonZeroU64::new(1).unwrap(),
                expected_genesis_hash: EvmHash::from_bytes([2; 32]),
            },
            endpoint_ref: endpoint.value_ref().clone(),
        },
        sender: EvmAddress::from_bytes([3; 20]),
    };
    let command = Eip1559TransactionCommand::call(
        binding.clone(),
        EvmAddress::from_bytes([4; 20]),
        vec![0, 0, 0, 84],
        EvmU256::new("0").unwrap(),
        NonZeroU64::new(100_000).unwrap(),
        1,
        2,
    )
    .unwrap();
    let request = Request {
        command: command.clone(),
    };
    let prepared_command = ReserveEvmNonce::<Request>::prepare(&request).unwrap();
    assert_eq!(prepared_command, command);
    let command_object = Object::from_value(&command).unwrap();
    let reservation = Reservation::new(
        EffectId::from_digest(DigestBytes::from_array([5; 32])),
        command_object.value_ref().clone(),
        NonceDomain {
            authority_epoch: binding.authority_epoch.clone(),
            chain_instance: binding.route.chain_instance.clone(),
            sender: binding.sender.clone(),
        },
        7,
    )
    .unwrap();
    let ProposedStateOutcome::Success { output: reserved } =
        ReserveEvmNonce::<Request>::interpret(request.clone(), &reservation).unwrap();
    let reserved = Object::from_value(&reserved)
        .unwrap()
        .decode::<ReservedRequest<Request>>()
        .unwrap();
    assert_eq!(reserved.request(), &request);
    let mut forged = serde_json::to_value(&reserved).unwrap();
    forged["request"]["command"]["action"]["value"]["calldata"] = serde_json::json!("AAAAKg");
    assert!(serde_json::from_str::<ReservedRequest<Request>>(&forged.to_string()).is_err());
    assert_eq!(
        PrepareEvmTransaction::<Request>::prepare(&reserved).unwrap(),
        *reserved.reserved()
    );
    let preparation = PreparedEvmTransactionEvidence {
        effect_id: EffectId::from_digest(DigestBytes::from_array([6; 32])),
        transaction_hash: EvmHash::from_bytes([7; 32]),
    };
    let ProposedStateOutcome::Success { output: prepared } =
        PrepareEvmTransaction::<Request>::interpret(reserved, &preparation).unwrap();
    let prepared = Object::from_value(&prepared)
        .unwrap()
        .decode::<PreparedTransaction<Request>>()
        .unwrap();
    let implementation = mfm_program::effect_implementation_ref::<
        TransactionEffect<Request>,
        EvmTransactionImplementation,
    >()
    .unwrap();
    let binding_object = Object::from_value(&binding).unwrap();
    assert_eq!(prepared.implementation_ref(), &implementation);
    assert_eq!(prepared.binding_ref(), binding_object.value_ref());
    assert_eq!(prepared.request(), &request);
    let semantic = Object::from_value(&prepared).unwrap();
    let (native_ref, native) = <EvmTransactionImplementation as EffectImplementation<
        TransactionEffect<Request>,
    >>::decode_command(
        &implementation,
        binding_object.value_ref(),
        &binding,
        semantic.value_ref(),
        &prepared,
    )
    .unwrap();
    assert_eq!(&native_ref, prepared.native().value_ref());
    assert_eq!(native.reserved().command(), &command);
    assert_eq!(native.transaction_hash(), &preparation.transaction_hash);
    for (implementation, binding_ref) in [
        (endpoint.value_ref(), binding_object.value_ref()),
        (&implementation, endpoint.value_ref()),
    ] {
        assert!(<EvmTransactionImplementation as EffectImplementation<
            TransactionEffect<Request>,
        >>::decode_command(
            implementation,
            binding_ref,
            &binding,
            semantic.value_ref(),
            &prepared
        )
        .is_err());
    }
    let mut other_binding = binding.clone();
    other_binding.sender = EvmAddress::from_bytes([8; 20]);
    assert!(<EvmTransactionImplementation as EffectImplementation<TransactionEffect<Request>>>::decode_command(
        &implementation, binding_object.value_ref(), &other_binding, semantic.value_ref(), &prepared).is_err());

    let effect = EffectId::from_digest(DigestBytes::from_array([9; 32]));
    let receipt = EvmTransactionReceipt {
        block_anchor: EvmBlockAnchor {
            hash: EvmHash::from_bytes([10; 32]),
            number: EvmU256::new("11").unwrap(),
        },
        transaction_hash: preparation.transaction_hash.clone(),
    };
    for (evidence, rejected) in [
        (
            EvmTransactionSettlement::called(effect.clone(), 7, receipt.clone()),
            false,
        ),
        (
            EvmTransactionSettlement::reverted(effect.clone(), 7, receipt),
            true,
        ),
    ] {
        let original = Object::from_value(&evidence).unwrap();
        let projected = <EvmTransactionImplementation as EffectImplementation<
            TransactionEffect<Request>,
        >>::project_evidence(
            &implementation,
            binding_object.value_ref(),
            &binding,
            &effect,
            semantic.value_ref(),
            &prepared,
            &native,
            &evidence,
            &original,
        )
        .unwrap();
        let projected = Object::from_value(&projected)
            .unwrap()
            .decode::<mfm_chain::transaction::TransactionEvidence<ConfigurationApplied>>()
            .unwrap();
        assert_eq!(projected.original().value_ref(), original.value_ref());
        assert_eq!(
            projected.original().canonical_bytes(),
            original.canonical_bytes()
        );
        assert_eq!(
            projected
                .observed_at()
                .native()
                .decode::<EvmBlockPoint>()
                .unwrap()
                .number(),
            &EvmU256::new("11").unwrap()
        );
        assert_eq!(
            matches!(
                projected.result(),
                mfm_chain::transaction::TransactionResult::Rejected { reason: None }
            ),
            rejected
        );
        assert!(<EvmTransactionImplementation as EffectImplementation<
            TransactionEffect<Request>,
        >>::project_evidence(
            &implementation,
            binding_object.value_ref(),
            &binding,
            &preparation.effect_id,
            semantic.value_ref(),
            &prepared,
            &native,
            &evidence,
            &original
        )
        .is_err());
    }
}
