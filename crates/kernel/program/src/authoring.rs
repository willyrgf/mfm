use std::collections::hash_map::Entry;
use std::{any::TypeId, collections::HashMap, marker::PhantomData};

use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::{ContentRef, EntryPointId, SchemaId, SemanticTypeId, StableId};
use mfm_values::{EnumTagging, MfmValue, SchemaDescriptor, SchemaShape};

use crate::{
    capability_contract_ref, derive_nominal_contract, state_implementation_ref, Declaration,
    Execution, MatchDeclaration, MatchVariant, Never, Program, ProgramError, PureState, ReadState,
    Result, State, StateDeclaration, MAX_DECLARATIONS, MAX_MATCH_ARMS, MAX_STATES,
};

const MAX_AUTHORING_CALLBACK_DEPTH: u8 = 64;

/// A reusable deterministic authoring-only composition of States and child Operations.
pub trait Operation: Sized {
    /// Complete input value expected by this Operation.
    type Input: MfmValue;
    /// Complete success value exposed by this Operation.
    type Output: MfmValue;
    /// Complete failure value propagated by this Operation.
    type Failure: MfmValue;

    /// Expands this configured occurrence into the supplied scoped compiler context.
    fn expand(
        &self,
        expansion: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> Result<()>;
}

/// Deterministically expands one root Operation and constructs Program v2.
pub fn expand_program<O: Operation>(entry_point_id: EntryPointId, root: &O) -> Result<Program> {
    let mut identities = IdentityMemo::default();
    let input = identities.value_ref::<O::Input>()?;
    let output = identities.value_ref::<O::Output>()?;
    let failure = identities.value_ref::<O::Failure>()?;
    let mut expansion = OperationExpansion::new(
        input.clone(),
        output.clone(),
        failure.clone(),
        Vec::new(),
        1,
        identities,
    );
    let root_result = root.expand(&mut expansion).map_err(authoring_error);
    root_result?;
    drop(expansion.identities);
    let mut draft = expansion.draft;
    draft.seal_open_success()?;
    draft.seal_open_failures()?;
    let declarations = draft.into_declarations()?;
    Program::new(entry_point_id, input, output, failure, declarations)
}

struct CachedValueContract {
    contract_ref: ContentRef,
    descriptor: SchemaDescriptor,
}

#[derive(Default)]
struct IdentityMemo {
    values: HashMap<TypeId, CachedValueContract>,
    states: HashMap<TypeId, ContentRef>,
    capabilities: HashMap<TypeId, ContentRef>,
}

impl IdentityMemo {
    fn value<T: MfmValue>(&mut self) -> Result<&CachedValueContract> {
        match self.values.entry(TypeId::of::<T>()) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let (contract_ref, descriptor) = derive_nominal_contract::<T>()?;
                Ok(entry.insert(CachedValueContract {
                    contract_ref,
                    descriptor,
                }))
            }
        }
    }

    fn value_ref<T: MfmValue>(&mut self) -> Result<ContentRef> {
        self.value::<T>().map(|facts| facts.contract_ref.clone())
    }

    fn state_ref<S: State>(&mut self) -> Result<ContentRef> {
        let key = TypeId::of::<S>();
        if let Some(reference) = self.states.get(&key) {
            return Ok(reference.clone());
        }
        let reference = state_implementation_ref::<S>()?;
        self.states.insert(key, reference.clone());
        Ok(reference)
    }

    fn capability_ref<C: ReadCapabilityContract>(&mut self) -> Result<ContentRef> {
        let key = TypeId::of::<C>();
        if let Some(reference) = self.capabilities.get(&key) {
            return Ok(reference.clone());
        }
        let reference = capability_contract_ref::<C>()?;
        self.capabilities.insert(key, reference.clone());
        Ok(reference)
    }
}

/// The one scoped pre-Program expansion and lowering context.
pub struct OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    admitted_failure_contract_refs: Vec<ContentRef>,
    draft: ExpansionDraft,
    identities: IdentityMemo,
    callback_depth: u8,
    marker: PhantomData<fn(I) -> (O, F)>,
}

impl<I, O, F> OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    fn new(
        input_contract_ref: ContentRef,
        output_contract_ref: ContentRef,
        failure_contract_ref: ContentRef,
        admitted_failure_contract_refs: Vec<ContentRef>,
        callback_depth: u8,
        identities: IdentityMemo,
    ) -> Self {
        Self {
            draft: ExpansionDraft::new(input_contract_ref),
            output_contract_ref,
            failure_contract_ref,
            admitted_failure_contract_refs,
            identities,
            callback_depth,
            marker: PhantomData,
        }
    }

    /// Appends one deterministic Pure State occurrence.
    pub fn pure<S: PureState>(&mut self) -> Result<()> {
        append_pure::<S>(
            &mut self.draft,
            &mut self.identities,
            &self.failure_contract_ref,
            &self.admitted_failure_contract_refs,
        )
    }

    /// Appends one exact-pair Read occurrence with deterministic capability-owned injection.
    pub fn read<S, C>(&mut self, setup: &<C as CapabilityInjection<S>>::Setup) -> Result<()>
    where
        S: ReadState<C>,
        C: ReadCapabilityContract + CapabilityInjection<S>,
    {
        let expanded_input = self.identities.value_ref::<C::ExpandedInput>()?;
        let expanded_output = self.identities.value_ref::<C::ExpandedOutput>()?;
        let state_input = self.identities.value_ref::<S::Input>()?;
        let state_output = self.identities.value_ref::<S::Output>()?;
        let state_failure = self.identities.value_ref::<S::Failure>()?;
        let never = self.identities.value_ref::<Never>()?;
        self.draft.require_current(&expanded_input)?;
        require_legal_failure(
            &state_failure,
            &never,
            &self.failure_contract_ref,
            &self.admitted_failure_contract_refs,
        )?;
        nested_callback_depth(self.callback_depth)?;
        let (identities, suffix) = expand_read_suffix::<S, C>(
            setup,
            std::mem::take(&mut self.identities),
            expanded_input,
            expanded_output,
            state_input,
            state_output,
            state_failure,
            never,
            self.callback_depth,
        );
        self.identities = identities;
        self.draft.merge_connected(suffix?)
    }

    /// Expands one configured child Operation and flattens its checked scope atomically.
    pub fn operation<Op: Operation>(&mut self, child: &Op) -> Result<()> {
        let input = self.identities.value_ref::<Op::Input>()?;
        let output = self.identities.value_ref::<Op::Output>()?;
        let failure = self.identities.value_ref::<Op::Failure>()?;
        let never = self.identities.value_ref::<Never>()?;
        self.draft.require_current(&input)?;
        require_legal_failure(
            &failure,
            &never,
            &self.failure_contract_ref,
            &self.admitted_failure_contract_refs,
        )?;
        let depth = nested_callback_depth(self.callback_depth)?;
        let mut child_expansion = OperationExpansion::new(
            input,
            output.clone(),
            failure,
            Vec::new(),
            depth,
            std::mem::take(&mut self.identities),
        );
        let callback_result = child.expand(&mut child_expansion).map_err(authoring_error);
        self.identities = std::mem::take(&mut child_expansion.identities);
        callback_result?;
        if child_expansion.draft.declarations.is_empty() {
            return Err(ProgramError::InvalidContract);
        }
        child_expansion.draft.reopen_scope_success(&output)?;
        self.draft.merge_connected(child_expansion.draft)
    }

    /// Authors one typed closed-sum Match whose nonterminal arms rejoin at `J`.
    pub fn match_join<T, J>(
        &mut self,
        define: impl FnOnce(&mut MatchJoin<I, O, F>) -> Result<()>,
    ) -> Result<()>
    where
        T: MfmValue,
        J: MfmValue,
    {
        let selector = self.identities.value_ref::<T>()?;
        let join = self.identities.value_ref::<J>()?;
        self.draft.require_current(&selector)?;
        let variants = selector_variants(self.identities.value::<T>()?)?;
        let mut arms = MatchJoin {
            draft: ExpansionDraft::with_match(selector),
            variants,
            selector_type_id: TypeId::of::<T>(),
            join_contract_ref: join.clone(),
            output_contract_ref: self.output_contract_ref.clone(),
            failure_contract_ref: self.failure_contract_ref.clone(),
            admitted_failure_contract_refs: self.admitted_failure_contract_refs.clone(),
            callback_depth: nested_callback_depth(self.callback_depth)?,
            identities: std::mem::take(&mut self.identities),
            marker: PhantomData,
        };
        let callback_result = define(&mut arms).map_err(authoring_error);
        self.identities = std::mem::take(&mut arms.identities);
        callback_result?;
        if arms.variants.iter().any(|variant| !variant.seen) {
            return Err(ProgramError::InvalidContract);
        }
        match &arms.draft.open_success {
            None if arms.draft.scope_success.is_empty() => {
                return Err(ProgramError::InvalidContract);
            }
            Some(frontier) if frontier.contract_ref != join || frontier.tails.is_empty() => {
                return Err(ProgramError::InvalidContract);
            }
            _ => {}
        }
        self.draft.merge_connected(arms.draft)
    }

    /// Routes exact failures from one protected region into one State-first handler.
    pub fn with_failure_handler<E, J>(
        &mut self,
        protected: impl FnOnce(&mut Self) -> Result<()>,
        handler: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()>
    where
        E: MfmValue,
        J: MfmValue,
    {
        let handled = self.identities.value_ref::<E>()?;
        let join = self.identities.value_ref::<J>()?;
        if handled == self.identities.value_ref::<Never>()? {
            return Err(ProgramError::InvalidContract);
        }
        let current = self
            .draft
            .current_contract()
            .cloned()
            .ok_or(ProgramError::InvalidContract)?;
        let depth = nested_callback_depth(self.callback_depth)?;
        let mut protected_failures = self.admitted_failure_contract_refs.clone();
        if !protected_failures.contains(&handled) {
            protected_failures.push(handled.clone());
        }
        let mut protected_expansion = Self::new(
            current.clone(),
            self.output_contract_ref.clone(),
            self.failure_contract_ref.clone(),
            protected_failures,
            depth,
            std::mem::take(&mut self.identities),
        );
        let protected_result = protected(&mut protected_expansion).map_err(authoring_error);
        self.identities = std::mem::take(&mut protected_expansion.identities);
        protected_result?;
        if protected_expansion.draft.declarations.is_empty() {
            return Err(ProgramError::InvalidContract);
        }
        protected_expansion.draft.require_current_or_closed(&join)?;
        let handled_tails = protected_expansion.draft.take_failures(&handled)?;
        if handled_tails.is_empty() {
            return Err(ProgramError::InvalidContract);
        }

        let mut handler_expansion = Self::new(
            handled,
            self.output_contract_ref.clone(),
            self.failure_contract_ref.clone(),
            self.admitted_failure_contract_refs.clone(),
            depth,
            std::mem::take(&mut self.identities),
        );
        let handler_result = handler(&mut handler_expansion).map_err(authoring_error);
        self.identities = std::mem::take(&mut handler_expansion.identities);
        handler_result?;
        if !handler_expansion.draft.is_state_first() {
            return Err(ProgramError::InvalidContract);
        }
        handler_expansion
            .draft
            .classify_join_or_terminal(&join, &self.output_contract_ref)?;

        let mut composite = ExpansionDraft::new(current);
        composite.open_success = None;
        let (_, protected_open, protected_success, protected_failures) =
            composite.merge_unconnected(protected_expansion.draft)?;
        let (handler_entry, handler_open, handler_success, handler_failures) =
            composite.merge_unconnected(handler_expansion.draft)?;
        composite.patch_failure_tails(&handled_tails, handler_entry)?;
        require_frontier_contract(protected_open.as_ref(), &join)?;
        require_frontier_contract(handler_open.as_ref(), &join)?;
        composite.open_success = combine_frontiers(protected_open, handler_open);
        composite.scope_success = protected_success;
        composite.scope_success.extend(handler_success);
        composite.open_failures = protected_failures;
        composite.open_failures.extend(handler_failures);
        self.draft.merge_connected(composite)
    }
}

/// One scoped typed Match whose nonterminal arms share one exact success join.
pub struct MatchJoin<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    draft: ExpansionDraft,
    variants: Vec<SelectorVariant>,
    selector_type_id: TypeId,
    join_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    admitted_failure_contract_refs: Vec<ContentRef>,
    identities: IdentityMemo,
    callback_depth: u8,
    marker: PhantomData<fn(I) -> (O, F)>,
}

impl<I, O, F> MatchJoin<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    /// Adds one exact payload arm to this Match scope.
    pub fn arm<P>(
        &mut self,
        tag: StableId,
        branch: impl FnOnce(&mut OperationExpansion<I, O, F>) -> Result<()>,
    ) -> Result<()>
    where
        P: MfmValue,
    {
        let variant_index = self
            .variants
            .iter()
            .position(|variant| variant.tag == tag.as_str())
            .ok_or(ProgramError::InvalidContract)?;
        if self.variants[variant_index].seen {
            return Err(ProgramError::InvalidContract);
        }
        require_payload::<P>(
            &mut self.identities,
            self.selector_type_id,
            self.variants[variant_index].descriptor_ordinal,
        )?;
        let input = self.identities.value_ref::<P>()?;
        let depth = nested_callback_depth(self.callback_depth)?;
        let mut branch_expansion = OperationExpansion::new(
            input,
            self.output_contract_ref.clone(),
            self.failure_contract_ref.clone(),
            self.admitted_failure_contract_refs.clone(),
            depth,
            std::mem::take(&mut self.identities),
        );
        let callback_result = branch(&mut branch_expansion).map_err(authoring_error);
        self.identities = std::mem::take(&mut branch_expansion.identities);
        callback_result?;
        if !branch_expansion.draft.is_state_first() {
            return Err(ProgramError::InvalidContract);
        }
        branch_expansion
            .draft
            .classify_join_or_terminal(&self.join_contract_ref, &self.output_contract_ref)?;
        require_frontier_contract(self.draft.open_success.as_ref(), &self.join_contract_ref)?;
        require_frontier_contract(
            branch_expansion.draft.open_success.as_ref(),
            &self.join_contract_ref,
        )?;
        let (entry, open, scope_success, open_failures) =
            self.draft.merge_unconnected(branch_expansion.draft)?;
        self.draft.add_match_variant(tag, entry);
        self.draft.open_success = combine_frontiers(self.draft.open_success.take(), open);
        self.draft.scope_success.extend(scope_success);
        self.draft.open_failures.extend(open_failures);
        self.variants[variant_index].seen = true;
        Ok(())
    }
}

/// Deterministic authoring policy for one exact Read capability and State pairing.
pub trait CapabilityInjection<S>
where
    S: State,
{
    /// Checked authoring input for this exact pairing.
    type Setup;
    /// Complete input contract exposed by the expanded occurrence.
    type ExpandedInput: MfmValue;
    /// Complete success contract exposed by the expanded occurrence.
    type ExpandedOutput: MfmValue;

    /// Derives the original occurrence's immutable persisted binding.
    fn original_binding_ref(setup: &Self::Setup) -> Result<ContentRef>;

    /// Writes ordinary Pure States before the designated Read occurrence.
    fn write_before(_setup: &Self::Setup, _writer: &mut InjectionWriter) -> Result<()> {
        Ok(())
    }

    /// Writes ordinary Pure States after the designated Read occurrence.
    fn write_after(_setup: &Self::Setup, _writer: &mut InjectionWriter) -> Result<()> {
        Ok(())
    }
}

/// Restricted writer for one atomic capability-injection suffix.
pub struct InjectionWriter {
    draft: ExpansionDraft,
    required_failure_contract_ref: ContentRef,
    identities: IdentityMemo,
}

impl InjectionWriter {
    /// Appends one deterministic Pure support State.
    pub fn pure<S: PureState>(&mut self) -> Result<()> {
        append_pure::<S>(
            &mut self.draft,
            &mut self.identities,
            &self.required_failure_contract_ref,
            &[],
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DraftId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DraftRoute {
    Open,
    Target(DraftId),
    Terminal,
}

struct DraftState {
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    execution: Execution,
    next: DraftRoute,
    failure_next: DraftRoute,
}

struct DraftMatchVariant {
    tag: StableId,
    entry: DraftId,
}

struct DraftMatch {
    selector_contract_ref: ContentRef,
    variants: Vec<DraftMatchVariant>,
}

// Bounded source-authoring drafts are State-heavy; contiguous storage avoids one allocation per State.
#[allow(clippy::large_enum_variant)]
enum DraftDeclaration {
    State(DraftState),
    Match(DraftMatch),
}

struct Frontier {
    contract_ref: ContentRef,
    tails: Vec<DraftId>,
}

struct ExpansionDraft {
    declarations: Vec<DraftDeclaration>,
    state_count: usize,
    open_success: Option<Frontier>,
    scope_success: Vec<DraftId>,
    open_failures: Vec<DraftId>,
}

type DetachedExits = (DraftId, Option<Frontier>, Vec<DraftId>, Vec<DraftId>);

impl ExpansionDraft {
    fn new(input_contract_ref: ContentRef) -> Self {
        Self {
            declarations: Vec::new(),
            state_count: 0,
            open_success: Some(Frontier {
                contract_ref: input_contract_ref,
                tails: Vec::new(),
            }),
            scope_success: Vec::new(),
            open_failures: Vec::new(),
        }
    }

    fn with_match(selector_contract_ref: ContentRef) -> Self {
        Self {
            declarations: vec![DraftDeclaration::Match(DraftMatch {
                selector_contract_ref,
                variants: Vec::new(),
            })],
            state_count: 0,
            open_success: None,
            scope_success: Vec::new(),
            open_failures: Vec::new(),
        }
    }

    fn current_contract(&self) -> Option<&ContentRef> {
        self.open_success
            .as_ref()
            .map(|frontier| &frontier.contract_ref)
    }

    fn require_current(&self, expected: &ContentRef) -> Result<()> {
        (self.current_contract() == Some(expected))
            .then_some(())
            .ok_or(ProgramError::InvalidContract)
    }

    fn require_current_or_closed(&self, expected: &ContentRef) -> Result<()> {
        match self.current_contract() {
            Some(actual) if actual == expected => Ok(()),
            None if !self.scope_success.is_empty() => Ok(()),
            _ => Err(ProgramError::InvalidContract),
        }
    }

    fn seal_open_success(&mut self) -> Result<()> {
        if let Some(frontier) = self.open_success.take() {
            self.patch_success_tails(&frontier.tails, DraftRoute::Terminal)?;
            self.scope_success.extend(frontier.tails);
        }
        Ok(())
    }

    fn classify_join_or_terminal(&mut self, join: &ContentRef, output: &ContentRef) -> Result<()> {
        let Some(frontier) = self.open_success.take() else {
            return (!self.scope_success.is_empty())
                .then_some(())
                .ok_or(ProgramError::InvalidContract);
        };
        if frontier.contract_ref == *join {
            self.open_success = Some(frontier);
            return Ok(());
        }
        if join != output && frontier.contract_ref == *output {
            self.patch_success_tails(&frontier.tails, DraftRoute::Terminal)?;
            self.scope_success.extend(frontier.tails);
            return Ok(());
        }
        self.open_success = Some(frontier);
        Err(ProgramError::InvalidContract)
    }

    fn reopen_scope_success(&mut self, expected: &ContentRef) -> Result<()> {
        for id in &self.scope_success {
            let state = self.state(*id)?;
            if state.output_contract_ref != *expected || state.next != DraftRoute::Terminal {
                return Err(ProgramError::InvalidContract);
            }
        }
        let mut tails = match self.open_success.take() {
            Some(frontier) if frontier.contract_ref == *expected => frontier.tails,
            Some(frontier) => {
                self.open_success = Some(frontier);
                return Err(ProgramError::InvalidContract);
            }
            None => Vec::new(),
        };
        let scope_success = std::mem::take(&mut self.scope_success);
        for id in &scope_success {
            let state = self.state_mut(*id)?;
            if state.next != DraftRoute::Terminal {
                return Err(ProgramError::InvalidContract);
            }
            state.next = DraftRoute::Open;
        }
        tails.extend(scope_success);
        if tails.is_empty() {
            return Err(ProgramError::InvalidContract);
        }
        self.open_success = Some(Frontier {
            contract_ref: expected.clone(),
            tails,
        });
        Ok(())
    }

    fn take_failures(&mut self, contract: &ContentRef) -> Result<Vec<DraftId>> {
        let mut matched = Vec::new();
        let mut retained = Vec::new();
        for id in std::mem::take(&mut self.open_failures) {
            if self.state(id)?.failure_contract_ref == *contract {
                matched.push(id);
            } else {
                retained.push(id);
            }
        }
        self.open_failures = retained;
        Ok(matched)
    }

    fn seal_open_failures(&mut self) -> Result<()> {
        let failures = std::mem::take(&mut self.open_failures);
        for id in failures {
            let state = self.state_mut(id)?;
            if state.failure_next != DraftRoute::Open {
                return Err(ProgramError::InvalidContract);
            }
            state.failure_next = DraftRoute::Terminal;
        }
        Ok(())
    }

    fn patch_failure_tails(&mut self, tails: &[DraftId], target: DraftId) -> Result<()> {
        for id in tails {
            let state = self.state_mut(*id)?;
            if state.failure_next != DraftRoute::Open {
                return Err(ProgramError::InvalidContract);
            }
            state.failure_next = DraftRoute::Target(target);
        }
        Ok(())
    }

    fn patch_success_tails(&mut self, tails: &[DraftId], route: DraftRoute) -> Result<()> {
        for id in tails {
            let state = self.state_mut(*id)?;
            if state.next != DraftRoute::Open {
                return Err(ProgramError::InvalidContract);
            }
            state.next = route;
        }
        Ok(())
    }

    fn merge_connected(&mut self, scratch: Self) -> Result<()> {
        let parent_frontier = self
            .open_success
            .as_ref()
            .ok_or(ProgramError::InvalidContract)?;
        if scratch.first_contract()? != &parent_frontier.contract_ref {
            return Err(ProgramError::InvalidContract);
        }
        let parent_tails = parent_frontier.tails.clone();
        let offset = self.preflight_merge(&scratch)?;
        let scratch = scratch.rebase(offset)?;
        self.patch_success_tails(&parent_tails, DraftRoute::Target(DraftId(offset)))?;
        self.open_success = scratch.open_success;
        self.state_count += scratch.state_count;
        self.declarations.extend(scratch.declarations);
        self.scope_success.extend(scratch.scope_success);
        self.open_failures.extend(scratch.open_failures);
        Ok(())
    }

    fn merge_unconnected(&mut self, scratch: Self) -> Result<DetachedExits> {
        scratch.first_contract()?;
        let offset = self.preflight_merge(&scratch)?;
        let scratch = scratch.rebase(offset)?;
        let entry = DraftId(offset);
        let open_success = scratch.open_success;
        let scope_success = scratch.scope_success;
        let open_failures = scratch.open_failures;
        self.state_count += scratch.state_count;
        self.declarations.extend(scratch.declarations);
        Ok((entry, open_success, scope_success, open_failures))
    }

    fn preflight_merge(&self, scratch: &Self) -> Result<usize> {
        let declaration_count = self
            .declarations
            .len()
            .checked_add(scratch.declarations.len())
            .ok_or(ProgramError::Capacity)?;
        let state_count = self
            .state_count
            .checked_add(scratch.state_count)
            .ok_or(ProgramError::Capacity)?;
        if declaration_count > MAX_DECLARATIONS || state_count > MAX_STATES {
            return Err(ProgramError::Capacity);
        }
        Ok(self.declarations.len())
    }

    fn rebase(mut self, offset: usize) -> Result<Self> {
        let rebase = |id: &mut DraftId| -> Result<()> {
            id.0 = id.0.checked_add(offset).ok_or(ProgramError::Capacity)?;
            Ok(())
        };
        for declaration in &mut self.declarations {
            match declaration {
                DraftDeclaration::State(state) => {
                    for route in [&mut state.next, &mut state.failure_next] {
                        if let DraftRoute::Target(id) = route {
                            id.0 = id.0.checked_add(offset).ok_or(ProgramError::Capacity)?;
                        }
                    }
                }
                DraftDeclaration::Match(selector) => {
                    for variant in &mut selector.variants {
                        rebase(&mut variant.entry)?;
                    }
                }
            }
        }
        if let Some(frontier) = &mut self.open_success {
            for tail in &mut frontier.tails {
                rebase(tail)?;
            }
        }
        for id in &mut self.scope_success {
            rebase(id)?;
        }
        for id in &mut self.open_failures {
            rebase(id)?;
        }
        Ok(self)
    }

    fn add_match_variant(&mut self, tag: StableId, entry: DraftId) {
        if let Some(DraftDeclaration::Match(selector)) = self.declarations.first_mut() {
            selector.variants.push(DraftMatchVariant { tag, entry });
        }
    }

    fn is_state_first(&self) -> bool {
        matches!(self.declarations.first(), Some(DraftDeclaration::State(_)))
    }

    fn first_contract(&self) -> Result<&ContentRef> {
        match self.declarations.first() {
            Some(DraftDeclaration::State(state)) => Ok(&state.input_contract_ref),
            Some(DraftDeclaration::Match(selector)) => Ok(&selector.selector_contract_ref),
            None => Err(ProgramError::InvalidContract),
        }
    }

    fn state(&self, id: DraftId) -> Result<&DraftState> {
        match self
            .declarations
            .get(id.0)
            .ok_or(ProgramError::InvalidContract)?
        {
            DraftDeclaration::State(state) => Ok(state),
            DraftDeclaration::Match(_) => Err(ProgramError::InvalidContract),
        }
    }

    fn state_mut(&mut self, id: DraftId) -> Result<&mut DraftState> {
        match self
            .declarations
            .get_mut(id.0)
            .ok_or(ProgramError::InvalidContract)?
        {
            DraftDeclaration::State(state) => Ok(state),
            DraftDeclaration::Match(_) => Err(ProgramError::InvalidContract),
        }
    }

    fn into_declarations(self) -> Result<Vec<Declaration>> {
        let mut declarations = Vec::with_capacity(self.declarations.len());
        for declaration in self.declarations {
            declarations.push(match declaration {
                DraftDeclaration::State(state) => Declaration::State(StateDeclaration::new(
                    state.state_implementation_ref,
                    state.input_contract_ref,
                    state.output_contract_ref,
                    state.failure_contract_ref,
                    state.execution,
                    lower_route(state.next)?,
                    lower_route(state.failure_next)?,
                )),
                DraftDeclaration::Match(selector) => Declaration::Match(MatchDeclaration::new(
                    selector.selector_contract_ref,
                    selector
                        .variants
                        .into_iter()
                        .map(|variant| {
                            Ok(MatchVariant::new(
                                variant.tag,
                                u16::try_from(variant.entry.0)
                                    .map_err(|_| ProgramError::Capacity)?,
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?,
                )?),
            });
        }
        Ok(declarations)
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_read_suffix<S, C>(
    setup: &C::Setup,
    identities: IdentityMemo,
    expanded_input: ContentRef,
    expanded_output: ContentRef,
    state_input: ContentRef,
    state_output: ContentRef,
    state_failure: ContentRef,
    never: ContentRef,
    callback_depth: u8,
) -> (IdentityMemo, Result<ExpansionDraft>)
where
    S: ReadState<C>,
    C: ReadCapabilityContract + CapabilityInjection<S>,
{
    let mut writer = InjectionWriter {
        draft: ExpansionDraft::new(expanded_input),
        required_failure_contract_ref: state_failure.clone(),
        identities,
    };
    let result = (|| {
        <C as CapabilityInjection<S>>::write_before(setup, &mut writer).map_err(authoring_error)?;
        writer.draft.require_current(&state_input)?;
        nested_callback_depth(callback_depth)?;
        let binding_ref =
            <C as CapabilityInjection<S>>::original_binding_ref(setup).map_err(authoring_error)?;
        append_state_core(
            &mut writer.draft,
            writer.identities.state_ref::<S>()?,
            state_input,
            state_output,
            state_failure.clone(),
            Execution::read(
                writer.identities.capability_ref::<C>()?,
                writer.identities.value_ref::<C::Intent>()?,
                writer.identities.value_ref::<C::Evidence>()?,
                binding_ref,
            ),
            &never,
            &state_failure,
            &[],
        )?;
        nested_callback_depth(callback_depth)?;
        <C as CapabilityInjection<S>>::write_after(setup, &mut writer).map_err(authoring_error)?;
        writer.draft.require_current(&expanded_output)?;
        Ok(())
    })();
    let InjectionWriter {
        draft, identities, ..
    } = writer;
    (identities, result.map(|()| draft))
}

fn append_pure<S: PureState>(
    draft: &mut ExpansionDraft,
    identities: &mut IdentityMemo,
    scope_failure: &ContentRef,
    admitted_failures: &[ContentRef],
) -> Result<()> {
    let never = identities.value_ref::<Never>()?;
    append_state_core(
        draft,
        identities.state_ref::<S>()?,
        identities.value_ref::<S::Input>()?,
        identities.value_ref::<S::Output>()?,
        identities.value_ref::<S::Failure>()?,
        Execution::pure(),
        &never,
        scope_failure,
        admitted_failures,
    )
}

#[allow(clippy::too_many_arguments)]
fn append_state_core(
    draft: &mut ExpansionDraft,
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    execution: Execution,
    never: &ContentRef,
    scope_failure: &ContentRef,
    admitted_failures: &[ContentRef],
) -> Result<()> {
    if input_contract_ref == *never || output_contract_ref == *never {
        return Err(ProgramError::InvalidContract);
    }
    draft.require_current(&input_contract_ref)?;
    require_legal_failure(
        &failure_contract_ref,
        never,
        scope_failure,
        admitted_failures,
    )?;
    if draft.declarations.len() >= MAX_DECLARATIONS || draft.state_count >= MAX_STATES {
        return Err(ProgramError::Capacity);
    }
    let frontier = draft
        .open_success
        .as_ref()
        .ok_or(ProgramError::InvalidContract)?;
    for id in &frontier.tails {
        if draft.state(*id)?.next != DraftRoute::Open {
            return Err(ProgramError::InvalidContract);
        }
    }
    let Some(frontier) = draft.open_success.take() else {
        return Err(ProgramError::InvalidContract);
    };
    let id = DraftId(draft.declarations.len());
    draft.patch_success_tails(&frontier.tails, DraftRoute::Target(id))?;
    let failure_next = if failure_contract_ref == *never {
        DraftRoute::Terminal
    } else {
        draft.open_failures.push(id);
        DraftRoute::Open
    };
    draft.declarations.push(DraftDeclaration::State(DraftState {
        state_implementation_ref,
        input_contract_ref,
        output_contract_ref: output_contract_ref.clone(),
        failure_contract_ref,
        execution,
        next: DraftRoute::Open,
        failure_next,
    }));
    draft.state_count += 1;
    draft.open_success = Some(Frontier {
        contract_ref: output_contract_ref,
        tails: vec![id],
    });
    Ok(())
}

fn require_legal_failure(
    failure: &ContentRef,
    never: &ContentRef,
    scope_failure: &ContentRef,
    admitted_failures: &[ContentRef],
) -> Result<()> {
    (failure == never || failure == scope_failure || admitted_failures.contains(failure))
        .then_some(())
        .ok_or(ProgramError::InvalidContract)
}

fn nested_callback_depth(current: u8) -> Result<u8> {
    current
        .checked_add(1)
        .filter(|depth| *depth <= MAX_AUTHORING_CALLBACK_DEPTH)
        .ok_or(ProgramError::Capacity)
}

fn authoring_error(error: ProgramError) -> ProgramError {
    match error {
        ProgramError::Capacity => error,
        ProgramError::Canonical | ProgramError::InvalidContract => ProgramError::InvalidContract,
    }
}

fn require_frontier_contract(frontier: Option<&Frontier>, contract: &ContentRef) -> Result<()> {
    frontier
        .is_none_or(|frontier| frontier.contract_ref == *contract)
        .then_some(())
        .ok_or(ProgramError::InvalidContract)
}

fn combine_frontiers(left: Option<Frontier>, right: Option<Frontier>) -> Option<Frontier> {
    let Some(mut left) = left else {
        return right;
    };
    if let Some(right) = right {
        left.tails.extend(right.tails);
    }
    Some(left)
}

fn lower_route(route: DraftRoute) -> Result<Option<u16>> {
    match route {
        DraftRoute::Open => Err(ProgramError::InvalidContract),
        DraftRoute::Terminal => Ok(None),
        DraftRoute::Target(target) => u16::try_from(target.0)
            .map(Some)
            .map_err(|_| ProgramError::Capacity),
    }
}

struct SelectorVariant {
    tag: String,
    descriptor_ordinal: usize,
    seen: bool,
}

fn selector_variants(selector: &CachedValueContract) -> Result<Vec<SelectorVariant>> {
    let shape = selector
        .descriptor
        .identity()
        .canonical_json_shape()
        .map_err(|_| ProgramError::InvalidContract)?;
    let SchemaShape::Enum { tagging, variants } = shape else {
        return Err(ProgramError::InvalidContract);
    };
    if !matches!(
        tagging,
        EnumTagging::External | EnumTagging::Adjacent { .. }
    ) || variants.is_empty()
    {
        return Err(ProgramError::InvalidContract);
    }
    if variants.len() > MAX_MATCH_ARMS {
        return Err(ProgramError::Capacity);
    }
    variants
        .iter()
        .enumerate()
        .map(|(descriptor_ordinal, variant)| {
            StableId::new(&variant.name).map_err(|_| ProgramError::InvalidContract)?;
            match_payload_descriptor(&variant.shape).ok_or(ProgramError::InvalidContract)?;
            Ok(SelectorVariant {
                tag: variant.name.clone(),
                descriptor_ordinal,
                seen: false,
            })
        })
        .collect()
}

fn match_payload_descriptor(
    shape: &SchemaShape,
) -> Option<(&SchemaId, &SemanticTypeId, &SchemaShape)> {
    let payload = match shape {
        SchemaShape::Tuple(elements) if elements.len() == 1 => &elements[0],
        other => other,
    };
    match payload {
        SchemaShape::InlineValue {
            schema_id,
            semantic_type_id,
            serialized_shape,
        } => Some((schema_id, semantic_type_id, serialized_shape)),
        SchemaShape::Generic {
            constructor,
            arguments,
            serialized_shape,
        } if constructor == "mfm/generic-value" => {
            let [argument] = arguments.as_slice() else {
                return None;
            };
            Some((
                &argument.schema_id,
                &argument.semantic_type_id,
                serialized_shape,
            ))
        }
        _ => None,
    }
}

fn require_payload<P: MfmValue>(
    identities: &mut IdentityMemo,
    selector_type_id: TypeId,
    descriptor_ordinal: usize,
) -> Result<()> {
    identities.value::<P>()?;
    let get = |key| {
        identities
            .values
            .get(&key)
            .ok_or(ProgramError::InvalidContract)
    };
    let payload = get(TypeId::of::<P>())?;
    let selector = get(selector_type_id)?;
    let shape = selector
        .descriptor
        .identity()
        .canonical_json_shape()
        .map_err(|_| ProgramError::InvalidContract)?;
    let SchemaShape::Enum { variants, .. } = shape else {
        return Err(ProgramError::InvalidContract);
    };
    let variant = variants
        .get(descriptor_ordinal)
        .ok_or(ProgramError::InvalidContract)?;
    let (schema_id, semantic_type_id, serialized_shape) =
        match_payload_descriptor(&variant.shape).ok_or(ProgramError::InvalidContract)?;
    let payload_identity = payload.descriptor.identity();
    (schema_id == payload.contract_ref.schema_id()
        && Some(semantic_type_id) == payload_identity.semantic_type_id.as_ref()
        && serialized_shape
            == payload_identity
                .canonical_json_shape()
                .map_err(|_| ProgramError::InvalidContract)?)
    .then_some(())
    .ok_or(ProgramError::InvalidContract)
}
