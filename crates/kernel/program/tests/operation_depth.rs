//! The active Operation guard runs before user planning and unwinds between siblings.
#[allow(dead_code)]
#[path = "native_construction/support.rs"]
mod support;
use mfm_program::*;
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};
use support::{Add, Deployed};

static PLANNED: AtomicUsize = AtomicUsize::new(0);
struct Branch<B>(PhantomData<B>);
impl<B> Default for Branch<B> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<B: AuthoringSource> OperationDefinition for Branch<B> {
    type Body = B;
}
impl<B: AuthoringSource + Default, Parent> Plan<Parent> for Branch<B> {
    type Config = Parent;
    fn plan<'a>(&'a self, parent: &'a Parent) -> Result<(&'a Parent, B)> {
        PLANNED.fetch_add(1, Ordering::SeqCst);
        Ok((parent, B::default()))
    }
}
type Depth1<B = Pure<Add>> = Operation<Branch<B>>;
type Depth2<B = Pure<Add>> = Operation<Branch<Depth1<B>>>;
type Depth3<B = Pure<Add>> = Operation<Branch<Depth2<B>>>;
type Depth4<B = Pure<Add>> = Operation<Branch<Depth3<B>>>;
type Depth5<B = Pure<Add>> = Operation<Branch<Depth4<B>>>;
type Depth6<B = Pure<Add>> = Operation<Branch<Depth5<B>>>;
type Depth7<B = Pure<Add>> = Operation<Branch<Depth6<B>>>;
type Depth8<B = Pure<Add>> = Operation<Branch<Depth7<B>>>;
type Depth9<B = Pure<Add>> = Operation<Branch<Depth8<B>>>;
type Depth10<B = Pure<Add>> = Operation<Branch<Depth9<B>>>;
type Depth11<B = Pure<Add>> = Operation<Branch<Depth10<B>>>;
type Depth12<B = Pure<Add>> = Operation<Branch<Depth11<B>>>;
type Depth13<B = Pure<Add>> = Operation<Branch<Depth12<B>>>;
type Depth14<B = Pure<Add>> = Operation<Branch<Depth13<B>>>;
type Depth15<B = Pure<Add>> = Operation<Branch<Depth14<B>>>;
type Depth16<B = Pure<Add>> = Operation<Branch<Depth15<B>>>;
type Depth17<B = Pure<Add>> = Operation<Branch<Depth16<B>>>;

struct Resources;
impl ProgramEnvironment for Resources {
    type Sources = Pure<Add>;
}

#[test]
fn depth_limit_precedes_planning_and_counts_active_scopes_only() {
    let entry = mfm_ids::EntryPointId::new("mfm.test/depth@1").unwrap();
    let input = Deployed { value: 42 };
    let accepted = compile(
        entry.clone(),
        &Depth16::<Pure<Add>>::default(),
        &input,
        &Resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_eq!(PLANNED.swap(0, Ordering::SeqCst), 16);
    assert_eq!(accepted.declarations().len(), 1);
    let cold = load(accepted.canonical_bytes(), &Resources).unwrap();
    assert_eq!(cold.content_ref(), accepted.content_ref());
    assert_eq!(PLANNED.load(Ordering::SeqCst), 0);

    let rejected = compile(
        entry.clone(),
        &Depth17::<Pure<Add>>::default(),
        &input,
        &Resources,
        ProgramLimits::new(0),
    );
    assert!(matches!(rejected, Err(ProgramError::Capacity)));
    assert_eq!(PLANNED.swap(0, Ordering::SeqCst), 16);

    let siblings = compile(
        entry,
        &(
            Depth16::<Pure<Add>>::default(),
            Depth16::<Pure<Add>>::default(),
        ),
        &input,
        &Resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_eq!(siblings.declarations().len(), 2);
    assert_eq!(PLANNED.swap(0, Ordering::SeqCst), 32);

    // The selected native Read and its resolved support Read share the same guard.
    let bindings = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let native = support::Resources::<false> {
        family: PhantomData,
        bindings: bindings.clone(),
        primary_available: true,
        execution: support::ProviderScript::Forbidden,
    };
    type Leaf = Read<support::Observe, support::Observation>;
    let input = support::Configured { value: 1 };
    let entry = mfm_ids::EntryPointId::new("mfm.test/native-depth@1").unwrap();
    let accepted = compile(
        entry.clone(),
        &Depth14::<Leaf>::default(),
        &input,
        &native,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert_eq!(accepted.declarations().len(), 2);
    assert_eq!(PLANNED.swap(0, Ordering::SeqCst), 14);
    assert_eq!(*bindings.lock().unwrap(), [0, 1]);
    bindings.lock().unwrap().clear();
    let rejected = compile(
        entry,
        &Depth15::<Leaf>::default(),
        &input,
        &native,
        ProgramLimits::new(0),
    );
    assert!(matches!(rejected, Err(ProgramError::Capacity)));
    assert_eq!(PLANNED.swap(0, Ordering::SeqCst), 15);
    assert!(bindings.lock().unwrap().is_empty());
}
