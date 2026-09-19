use std::num::NonZeroU64;

use mfm_capabilities::ReadImplementation;
use mfm_chain::balance::{
    BalanceCollectionFailure, BalanceCollectionMetadata, BalanceContext, BalanceRead,
    BalanceRequest, BalanceSource, CandidateBalance, ConsolidateBalanceCollection, DecimalScale,
    ObserveBalance,
};
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_evm::*;
use mfm_program::{Classification, ClassifyError, ProposedStateOutcome, PureState, ReadState};
use mfm_values::{Object, Unsigned256};

#[test]
fn native_and_token_preparation_keep_typed_context_and_confirmation_precedes_amount_admission() {
    let chain = NonZeroU64::new(1).unwrap();
    let ledger = LedgerIdentity::new(Object::from_value(&EvmBalanceLedger::new(chain)).unwrap());
    let endpoint = Object::from_value(&EvmEndpoint::new("scripted-rpc").unwrap()).unwrap();
    let route = Object::from_value(&EvmPhysicalTarget {
        chain_id: chain,
        endpoint_ref: endpoint.value_ref().clone(),
    })
    .unwrap();
    let native = EvmBalanceTarget::new(EvmAddress::from_bytes([1; 20]), None);
    let token = EvmBalanceTarget::new(
        EvmAddress::from_bytes([1; 20]),
        Some(EvmAddress::from_bytes([2; 20])),
    );
    let request = BalanceRequest::new(
        vec![
            BalanceSource::new(
                "native".into(),
                BalanceTarget::new(ledger.clone(), Object::from_value(&native).unwrap()),
            )
            .unwrap(),
            BalanceSource::new(
                "token".into(),
                BalanceTarget::new(ledger, Object::from_value(&token).unwrap()),
            )
            .unwrap(),
        ],
        DecimalScale::new(0).unwrap(),
    )
    .unwrap();
    let metadata =
        BalanceCollectionMetadata::new(0, "collection".into(), route.value_ref().clone()).unwrap();
    let mut context = BalanceContext::new(request, Unsigned256::from_u64(42), metadata);
    let anchor = EvmBlockAnchor {
        number: EvmU256::new("18446744073709551616").unwrap(),
        hash: EvmHash::from_bytes([3; 32]),
    };
    let changed = EvmBlockAnchor {
        number: anchor.number.clone(),
        hash: EvmHash::from_bytes([4; 32]),
    };
    let unchanged_failure = serde_json::json!({
        "anchor_changed": {"previous": anchor, "observed": anchor}
    });
    assert!(serde_json::from_value::<EvmBalanceFailure>(unchanged_failure).is_err());
    for index in 0..2 {
        let intent = CheckEvmBalanceChain::<Unsigned256>::prepare(&context).unwrap();
        assert_eq!(intent.source_ordinal(), index);
        let identity = Object::from_value(&intent).unwrap();
        let context_object = Object::from_value(&context).unwrap();
        for (evidence, expected) in [
            (
                EvmReadEvidence::rejected(identity.value_ref().clone()),
                EvmBalanceFailure::ObservationRejected {
                    reason: EvmObservationRejection::Rejected,
                },
            ),
            (
                EvmReadEvidence::safe_failure(identity.value_ref().clone()),
                EvmBalanceFailure::ObservationRejected {
                    reason: EvmObservationRejection::SafeFailure,
                },
            ),
            (
                EvmReadEvidence::integrity_blocked(identity.value_ref().clone()),
                EvmBalanceFailure::IntegrityBlocked,
            ),
            (
                EvmReadEvidence::returned(
                    identity.value_ref().clone(),
                    EvmReadValue::ChainId(NonZeroU64::new(2).unwrap()),
                ),
                EvmBalanceFailure::ObservationRejected {
                    reason: EvmObservationRejection::ChainMismatch,
                },
            ),
        ] {
            let ProposedStateOutcome::Failure { failure } =
                CheckEvmBalanceChain::<Unsigned256>::interpret(
                    context_object.decode().unwrap(),
                    &evidence,
                )
                .unwrap()
            else {
                panic!("authenticated unsuccessful observation must reject progression")
            };
            assert_eq!(failure, expected);
            assert_eq!(failure.classify(), Classification::Permanent);
            assert_eq!(
                Object::from_value(&failure)
                    .unwrap()
                    .decode::<EvmBalanceFailure>()
                    .unwrap(),
                expected
            );
        }
        // Local identity/type mismatches grant no domain-failure or retry authority.
        for evidence in [
            EvmReadEvidence::rejected(context_object.value_ref().clone()),
            EvmReadEvidence::returned(
                identity.value_ref().clone(),
                EvmReadValue::Anchor(anchor.clone()),
            ),
        ] {
            assert!(CheckEvmBalanceChain::<Unsigned256>::interpret(
                context_object.decode().unwrap(),
                &evidence,
            )
            .is_err());
        }
        let evidence =
            EvmReadEvidence::returned(identity.value_ref().clone(), EvmReadValue::ChainId(chain));
        let ProposedStateOutcome::Success { output: checked } =
            CheckEvmBalanceChain::<Unsigned256>::interpret(context, &evidence).unwrap()
        else {
            panic!("expected checked chain")
        };
        let checked = Object::from_value(&checked)
            .unwrap()
            .decode::<EvmChainChecked<Unsigned256>>()
            .unwrap();
        assert_eq!(checked.checked_chain_id(), chain);
        let mut wire = serde_json::to_value(&checked).unwrap();
        wire["checked_chain_id"] = serde_json::json!(2);
        assert!(serde_json::from_str::<EvmChainChecked<Unsigned256>>(&wire.to_string()).is_err());
        let prepared = if index == 0 {
            assert!(
                ReadInitialEvmBalanceAnchor::<Unsigned256, EvmTokenBalance>::prepare(&checked)
                    .is_err()
            );
            let intent =
                ReadInitialEvmBalanceAnchor::<Unsigned256, EvmNativeBalance>::prepare(&checked)
                    .unwrap();
            assert!(matches!(intent.subject(), EvmReadSubject::InitialAnchor));
            let reference = Object::from_value(&intent).unwrap();
            let evidence = EvmReadEvidence::returned(
                reference.value_ref().clone(),
                EvmReadValue::Anchor(anchor.clone()),
            );
            let ProposedStateOutcome::Success { output } =
                ReadInitialEvmBalanceAnchor::<Unsigned256, EvmNativeBalance>::interpret(
                    checked, &evidence,
                )
                .unwrap()
            else {
                panic!("expected native preparation")
            };
            assert_eq!(output.source_decimals().get(), 0);
            output
        } else {
            assert!(
                ReadInitialEvmBalanceAnchor::<Unsigned256, EvmNativeBalance>::prepare(&checked)
                    .is_err()
            );
            let intent =
                ReadInitialEvmBalanceAnchor::<Unsigned256, EvmTokenBalance>::prepare(&checked)
                    .unwrap();
            let reference = Object::from_value(&intent).unwrap();
            let checked_object = Object::from_value(&checked).unwrap();
            let mismatch = EvmReadEvidence::returned(
                reference.value_ref().clone(),
                EvmReadValue::Anchor(changed.clone()),
            );
            let ProposedStateOutcome::Failure { failure } =
                ReadInitialEvmBalanceAnchor::<Unsigned256, EvmTokenBalance>::interpret(
                    checked_object.decode().unwrap(),
                    &mismatch,
                )
                .unwrap()
            else {
                panic!("expected common anchor failure")
            };
            assert_eq!(failure.classify(), Classification::InputInvalidated);
            let evidence = EvmReadEvidence::returned(
                reference.value_ref().clone(),
                EvmReadValue::Anchor(anchor.clone()),
            );
            let ProposedStateOutcome::Success { output: anchored } =
                ReadInitialEvmBalanceAnchor::<Unsigned256, EvmTokenBalance>::interpret(
                    checked, &evidence,
                )
                .unwrap()
            else {
                panic!("expected token anchor")
            };
            let anchored = Object::from_value(&anchored)
                .unwrap()
                .decode::<EvmTokenAnchored<Unsigned256>>()
                .unwrap();
            assert_eq!(anchored.anchor(), &anchor);
            let intent = ReadEvmTokenDecimals::<Unsigned256>::prepare(&anchored).unwrap();
            assert!(
                matches!(intent.subject(), EvmReadSubject::TokenDecimals { anchor: observed } if observed == &anchor)
            );
            let reference = Object::from_value(&intent).unwrap();
            let evidence = EvmReadEvidence::returned(
                reference.value_ref().clone(),
                EvmReadValue::TokenDecimals(EvmTokenDecimals::new(2).unwrap()),
            );
            let ProposedStateOutcome::Success { output } =
                ReadEvmTokenDecimals::<Unsigned256>::interpret(anchored, &evidence).unwrap()
            else {
                panic!("expected token preparation")
            };
            assert_eq!(output.source_decimals().get(), 2);
            output
        };
        // Script only the external observation; production native projection and the shared State
        // construct the candidate. This still does not execute a full Program through Runtime.
        let semantic_intent = ObserveBalance::<Unsigned256>::prepare(&prepared).unwrap();
        let semantic_object = Object::from_value(&semantic_intent).unwrap();
        let target = semantic_intent
            .target()
            .native()
            .decode::<EvmBalanceTarget>()
            .unwrap();
        let binding = EvmBalanceBinding::new(
            route.decode().unwrap(),
            index,
            target,
            prepared.context().request().decimals(),
        );
        let binding_object = Object::from_value(&binding).unwrap();
        let abi = if index == 0 {
            mfm_program::read_implementation_ref::<BalanceRead, EvmNativeBalance>().unwrap()
        } else {
            mfm_program::read_implementation_ref::<BalanceRead, EvmTokenBalance>().unwrap()
        };
        let native_intent = if index == 0 {
            EvmNativeBalance::encode_intent(
                &abi,
                binding_object.value_ref(),
                &binding,
                &semantic_intent,
            )
        } else {
            EvmTokenBalance::encode_intent(
                &abi,
                binding_object.value_ref(),
                &binding,
                &semantic_intent,
            )
        }
        .unwrap();
        assert!(match native_intent.subject() {
            EvmReadSubject::NativeBalance { anchor: observed } => index == 0 && observed == &anchor,
            EvmReadSubject::TokenBalance { anchor: observed } => index == 1 && observed == &anchor,
            _ => false,
        });
        let native_object = Object::from_value(&native_intent).unwrap();
        let evidence = EvmReadEvidence::returned(
            native_object.value_ref().clone(),
            EvmReadValue::RawUnits(EvmU256::from_u64(if index == 0 { 7 } else { 101 })),
        );
        let original = Object::from_value(&evidence).unwrap();
        let projected = if index == 0 {
            EvmNativeBalance::project_evidence(
                &abi,
                binding_object.value_ref(),
                &binding,
                semantic_object.value_ref(),
                &semantic_intent,
                native_object.value_ref(),
                &native_intent,
                &evidence,
                &original,
            )
        } else {
            EvmTokenBalance::project_evidence(
                &abi,
                binding_object.value_ref(),
                &binding,
                semantic_object.value_ref(),
                &semantic_intent,
                native_object.value_ref(),
                &native_intent,
                &evidence,
                &original,
            )
        }
        .unwrap();
        assert_eq!(projected.original(), &original);
        assert_eq!(projected.implementation_ref(), &abi);
        let projected = Object::from_value(&projected).unwrap().decode().unwrap();
        let ProposedStateOutcome::Success { output: candidate } =
            ObserveBalance::<Unsigned256>::interpret(prepared, &projected).unwrap()
        else {
            panic!("expected candidate from projected native evidence")
        };
        let candidate_object = Object::from_value(&candidate).unwrap();
        let intent = ConfirmEvmBalanceAnchor::<Unsigned256>::prepare(&candidate).unwrap();
        assert!(
            matches!(intent.subject(), EvmReadSubject::ConfirmAnchor { anchor: observed } if observed == &anchor)
        );
        let reference = Object::from_value(&intent).unwrap();
        let mismatch = EvmReadEvidence::returned(
            reference.value_ref().clone(),
            EvmReadValue::Anchor(changed.clone()),
        );
        let ProposedStateOutcome::Failure { failure } =
            ConfirmEvmBalanceAnchor::<Unsigned256>::interpret(
                candidate_object.decode().unwrap(),
                &mismatch,
            )
            .unwrap()
        else {
            panic!("expected confirmation mismatch")
        };
        assert!(
            matches!(failure, EvmBalanceFailure::AnchorChanged { ref previous, ref observed, .. } if previous == &anchor && observed == &changed)
        );
        assert_eq!(
            Object::from_value(&failure)
                .unwrap()
                .decode::<EvmBalanceFailure>()
                .unwrap(),
            failure
        );
        let evidence = EvmReadEvidence::returned(
            reference.value_ref().clone(),
            EvmReadValue::Anchor(anchor.clone()),
        );
        context = match ConfirmEvmBalanceAnchor::<Unsigned256>::interpret(candidate, &evidence)
            .unwrap()
        {
            ProposedStateOutcome::Success { output } => {
                assert_eq!(index, 0);
                output
            }
            ProposedStateOutcome::Failure { failure } => {
                assert_eq!(index, 1);
                assert!(matches!(
                    failure,
                    EvmBalanceFailure::Collection {
                        source: BalanceCollectionFailure::InexactScale { .. }
                    }
                ));
                assert_eq!(failure.classify(), Classification::Permanent);
                assert_eq!(
                    Object::from_value(&failure)
                        .unwrap()
                        .decode::<EvmBalanceFailure>()
                        .unwrap(),
                    failure
                );
                let mut wire = serde_json::to_value(
                    candidate_object
                        .decode::<CandidateBalance<Unsigned256>>()
                        .unwrap(),
                )
                .unwrap();
                wire["raw_units"] = serde_json::json!("100");
                let valid =
                    serde_json::from_str::<CandidateBalance<Unsigned256>>(&wire.to_string())
                        .unwrap();
                let ProposedStateOutcome::Success { output } =
                    ConfirmEvmBalanceAnchor::<Unsigned256>::interpret(valid, &evidence).unwrap()
                else {
                    panic!("expected exact amount confirmation")
                };
                output
            }
        };
        context = Object::from_value(&context)
            .unwrap()
            .decode::<BalanceContext<Unsigned256>>()
            .unwrap();
        assert_eq!(context.completed().len(), index as usize + 1);
        assert_eq!(context.caller().as_str(), "42");
    }
    assert_eq!(context.completed()[0].source().source_id(), "native");
    assert_eq!(context.completed()[1].source().source_id(), "token");
    let ProposedStateOutcome::Success { output } =
        ConsolidateBalanceCollection::<Unsigned256>::evaluate(context).unwrap()
    else {
        panic!("expected confirmed collection completion")
    };
    assert_eq!(output.total_scaled(), "8");
    assert_eq!(output.context().caller().as_str(), "42");
}
