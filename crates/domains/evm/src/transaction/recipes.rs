use super::*;
use mfm_values::ContextSlot;
use std::marker::PhantomData;

/// Deterministic command policy selecting one context field and an EVM-owned success mode.
///
/// Implementations use only checked admitted data. Change `recipe_id` when command policy
/// changes; the selected slots are ordered destination first, then source dependencies.
///
/// A plan cannot serve as a successful creation dependency:
///
/// ```compile_fail
/// use mfm_evm::{CallCreatedAt, CheckedCallPlan, CheckedCreatePlan, TransactionRecipe};
/// use mfm_program_derive::{MfmContext, MfmValue};
/// use serde::{Deserialize, Serialize};
/// #[derive(Serialize, Deserialize, MfmValue, MfmContext)]
/// #[context(namespace = "mfm.example.contract")]
/// struct Workflow<D, C> { deployment: D, configuration: C }
/// type Initial = Workflow<CheckedCreatePlan, CheckedCallPlan>;
/// fn requires_recipe<R: TransactionRecipe<Initial>>() {}
/// requires_recipe::<CallCreatedAt<WorkflowConfigurationSlot, WorkflowDeploymentSlot>>();
/// ```
pub trait TransactionRecipe<C: MfmValueTrait>: Send + Sync + 'static {
    /// Destination field extended by every transaction stage.
    type Slot: ContextSlot<C>;
    /// The one supported outcome mode used by command checking and final projection.
    type Success: TransactionSuccessMode;
    /// Stable identity of this command-selection implementation.
    fn recipe_id() -> mfm_values::Result<StableId>;
    /// Ordered destination and source slot identities.
    fn source_ids() -> mfm_values::Result<Vec<StableId>>;
    /// Constructs a complete nonce-free command from checked context facts, without IO.
    fn command(context: &C) -> Eip1559TransactionCommand;
}

/// Creates from the selected checked creation plan.
pub struct CreateAt<S>(PhantomData<fn() -> S>);
impl<C: MfmValueTrait, S: ContextSlot<C, Value = CheckedCreatePlan> + 'static> TransactionRecipe<C>
    for CreateAt<S>
{
    type Slot = S;
    type Success = Created;
    fn recipe_id() -> mfm_values::Result<StableId> {
        recipe_id("mfm.evm.recipe.create-at@1")
    }
    fn source_ids() -> mfm_values::Result<Vec<StableId>> {
        Ok(vec![S::slot_id()?])
    }
    fn command(context: &C) -> Eip1559TransactionCommand {
        S::get(context).command()
    }
}

/// Calls the required target in the selected checked ordinary call plan.
pub struct CallAt<S>(PhantomData<fn() -> S>);
impl<C: MfmValueTrait, S: ContextSlot<C, Value = CheckedTargetCallPlan> + 'static>
    TransactionRecipe<C> for CallAt<S>
{
    type Slot = S;
    type Success = Called;
    fn recipe_id() -> mfm_values::Result<StableId> {
        recipe_id("mfm.evm.recipe.call-at@1")
    }
    fn source_ids() -> mfm_values::Result<Vec<StableId>> {
        Ok(vec![S::slot_id()?])
    }
    fn command(context: &C) -> Eip1559TransactionCommand {
        S::get(context).command()
    }
}

/// Combines a checked target-free call plan with a selected successful creation's address.
pub struct CallCreatedAt<S, D>(PhantomData<fn() -> (S, D)>);
impl<C, S, D> TransactionRecipe<C> for CallCreatedAt<S, D>
where
    C: MfmValueTrait,
    S: ContextSlot<C, Value = CheckedCallPlan> + 'static,
    D: ContextSlot<C, Value = CompletedTransactionFacts<Created>> + 'static,
{
    type Slot = S;
    type Success = Called;
    fn recipe_id() -> mfm_values::Result<StableId> {
        recipe_id("mfm.evm.recipe.call-created-at@1")
    }
    fn source_ids() -> mfm_values::Result<Vec<StableId>> {
        Ok(vec![S::slot_id()?, D::slot_id()?])
    }
    fn command(context: &C) -> Eip1559TransactionCommand {
        S::get(context).command_for(D::get(context).outcome().created_address().clone())
    }
}

pub(crate) fn recipe_id(id: &str) -> mfm_values::Result<StableId> {
    StableId::new(id)
        .map_err(|_| mfm_values::ValueError::Identity("invalid EVM recipe identity".to_owned()))
}

pub(crate) fn executable_id(
    stage: &str,
    recipe: StableId,
    sources: Vec<StableId>,
    mode: &str,
) -> mfm_program::Result<StableId> {
    use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
    let descriptor = CanonicalValue::object([
        (
            "domain",
            CanonicalValue::String("mfm.evm.context-executable".to_owned()),
        ),
        ("implementation_version", CanonicalValue::Unsigned(1)),
        ("stage", CanonicalValue::String(stage.to_owned())),
        ("recipe", CanonicalValue::String(recipe.to_string())),
        (
            "sources",
            CanonicalValue::Array(
                sources
                    .into_iter()
                    .map(|id| CanonicalValue::String(id.to_string()))
                    .collect(),
            ),
        ),
        ("outcome_mode", CanonicalValue::String(mode.to_owned())),
    ])
    .map_err(|_| mfm_program::ProgramError::InvalidContract)?;
    let digest = CanonicalJsonBytes::from_value(&descriptor).digest_bytes();
    StableId::new(format!("mfm.evm.state.{stage}@1/{digest}"))
        .map_err(|_| mfm_program::ProgramError::InvalidContract)
}
