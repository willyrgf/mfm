//! Descriptor-only fixture; no native execution or protocol validation is represented here.
use mfm_capabilities::EffectImplementation;
use mfm_chain::transaction::{
    ConfigurationValue, DeployedContract, DeploymentRequest, PreparedTransaction,
    TransactionEffect, TransactionEvidence, TransactionRequest,
};
use mfm_ids::{ContentRef, EffectId, StableId};
use mfm_program::{effect_implementation_ref, Never, NoParams};
use mfm_values::Object;

struct SharedNative;
impl<R: TransactionRequest> EffectImplementation<TransactionEffect<R>> for SharedNative {
    type Binding = NoParams;
    type NativeCommand = ConfigurationValue;
    type NativeEvidence = ConfigurationValue;
    type OperationalError = Never;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("proof.shared-native@1")?)
    }
    fn decode_command(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &ContentRef,
        _: &PreparedTransaction<R>,
    ) -> Result<(ContentRef, ConfigurationValue), mfm_capabilities::CallbackFailure> {
        panic!("descriptor derivation must not invoke native hooks")
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        _: &NoParams,
        _: &EffectId,
        _: &ContentRef,
        _: &PreparedTransaction<R>,
        _: &ConfigurationValue,
        _: &ConfigurationValue,
        _: &Object,
    ) -> Result<TransactionEvidence<R::Applied>, mfm_capabilities::CallbackFailure> {
        panic!("descriptor derivation must not invoke native hooks")
    }
}

#[test]
fn shared_native_family_has_distinct_exact_refs_for_deployment_and_configuration() {
    assert_eq!(<SharedNative as EffectImplementation<TransactionEffect<DeploymentRequest>>>::implementation_id().unwrap(),
        <SharedNative as EffectImplementation<TransactionEffect<DeployedContract>>>::implementation_id().unwrap());
    let deployment =
        effect_implementation_ref::<TransactionEffect<DeploymentRequest>, SharedNative>().unwrap();
    let configuration =
        effect_implementation_ref::<TransactionEffect<DeployedContract>, SharedNative>().unwrap();
    assert_ne!(deployment, configuration);
    assert_eq!(
        deployment,
        effect_implementation_ref::<TransactionEffect<DeploymentRequest>, SharedNative>().unwrap()
    );
}
