use std::marker::PhantomData;

use mfm_capabilities::ReadCapabilityContract;
use mfm_ids::{ContentRef, EntryPointId, SchemaId, SemanticTypeId, StableId};
use mfm_values::{EnumTagging, MfmValue, SchemaShape};

use crate::{
    capability_contract_ref, never_ref, nominal_contract_ref, state_implementation_ref,
    Declaration, Execution, MatchDeclaration, MatchVariant, Program, ProgramError, PureState,
    ReadState, Result, State, StateDeclaration, MAX_DECLARATIONS, MAX_MATCH_ARMS, MAX_STATES,
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
    let input = nominal_contract_ref::<O::Input>()?;
    let output = nominal_contract_ref::<O::Output>()?;
    let failure = nominal_contract_ref::<O::Failure>()?;
    let mut expansion = OperationExpansion::new(
        input.clone(),
        output.clone(),
        failure.clone(),
        Vec::new(),
        1,
    );
    root.expand(&mut expansion).map_err(authoring_error)?;
    if expansion.input_contract_ref != input {
        return Err(ProgramError::InvalidContract);
    }

    if expansion.draft.declarations.is_empty() {
        if input != output || failure != never_ref()? {
            return Err(ProgramError::InvalidContract);
        }
        expansion.draft.open_success = None;
    } else {
        expansion.draft.seal_root_success(&output)?;
        expansion.draft.require_failures(&failure)?;
        expansion.draft.seal_open_failures()?;
    }
    expansion.draft.require_resolved()?;
    let declarations = expansion.draft.into_declarations()?;
    Program::new(entry_point_id, input, output, failure, declarations)
}

/// The one scoped pre-Program expansion and lowering context.
pub struct OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    admitted_failure_contract_refs: Vec<ContentRef>,
    draft: ExpansionDraft,
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
    ) -> Self {
        Self {
            draft: ExpansionDraft::new(input_contract_ref.clone()),
            input_contract_ref,
            output_contract_ref,
            failure_contract_ref,
            admitted_failure_contract_refs,
            callback_depth,
            marker: PhantomData,
        }
    }

    /// Appends one deterministic Pure State occurrence.
    pub fn pure<S: PureState>(&mut self) -> Result<()> {
        append_pure::<S>(
            &mut self.draft,
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
        let expanded_input = nominal_contract_ref::<C::ExpandedInput>()?;
        let expanded_output = nominal_contract_ref::<C::ExpandedOutput>()?;
        let state_input = nominal_contract_ref::<S::Input>()?;
        let state_output = nominal_contract_ref::<S::Output>()?;
        let state_failure = nominal_contract_ref::<S::Failure>()?;
        self.require_current(&expanded_input)?;
        require_legal_failure(
            &state_failure,
            &self.failure_contract_ref,
            &self.admitted_failure_contract_refs,
        )?;

        nested_callback_depth(self.callback_depth)?;
        let mut writer = InjectionWriter {
            draft: ExpansionDraft::new(expanded_input),
            required_failure_contract_ref: state_failure.clone(),
        };
        C::write_before(setup, &mut writer).map_err(authoring_error)?;
        writer.draft.require_current(&state_input)?;
        nested_callback_depth(self.callback_depth)?;
        let binding_ref = C::original_binding_ref(setup).map_err(authoring_error)?;
        append_state_core(
            &mut writer.draft,
            state_implementation_ref::<S>()?,
            state_input,
            state_output,
            state_failure.clone(),
            Execution::read(
                capability_contract_ref::<C>()?,
                nominal_contract_ref::<C::Intent>()?,
                nominal_contract_ref::<C::Evidence>()?,
                binding_ref,
            ),
            &state_failure,
            &[],
        )?;
        nested_callback_depth(self.callback_depth)?;
        C::write_after(setup, &mut writer).map_err(authoring_error)?;
        writer.draft.require_current(&expanded_output)?;
        writer.draft.require_failures(&state_failure)?;
        self.draft.merge_connected(writer.draft)
    }

    /// Expands one configured child Operation and flattens its checked scope atomically.
    pub fn operation<Op: Operation>(&mut self, child: &Op) -> Result<()> {
        let input = nominal_contract_ref::<Op::Input>()?;
        let output = nominal_contract_ref::<Op::Output>()?;
        let failure = nominal_contract_ref::<Op::Failure>()?;
        self.require_current(&input)?;
        require_legal_failure(
            &failure,
            &self.failure_contract_ref,
            &self.admitted_failure_contract_refs,
        )?;
        let depth = nested_callback_depth(self.callback_depth)?;
        let mut child_expansion =
            OperationExpansion::new(input, output.clone(), failure.clone(), Vec::new(), depth);
        child
            .expand(&mut child_expansion)
            .map_err(authoring_error)?;
        if child_expansion.draft.declarations.is_empty() {
            return Err(ProgramError::InvalidContract);
        }
        child_expansion.draft.reopen_scope_success(&output)?;
        child_expansion.draft.require_current(&output)?;
        child_expansion.draft.require_failures(&failure)?;
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
        let selector = nominal_contract_ref::<T>()?;
        let join = nominal_contract_ref::<J>()?;
        self.require_current(&selector)?;
        let variants = selector_variants::<T>()?;
        let depth = nested_callback_depth(self.callback_depth)?;
        let mut arms = MatchJoin {
            draft: ExpansionDraft::with_match(selector.clone()),
            variants,
            join_contract_ref: join.clone(),
            output_contract_ref: self.output_contract_ref.clone(),
            failure_contract_ref: self.failure_contract_ref.clone(),
            admitted_failure_contract_refs: self.admitted_failure_contract_refs.clone(),
            callback_depth: depth,
            marker: PhantomData,
        };
        define(&mut arms).map_err(authoring_error)?;
        if arms.variants.iter().any(|variant| !variant.seen) {
            return Err(ProgramError::InvalidContract);
        }
        arms.draft.finish_match_frontier(join)?;
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
        let handled = nominal_contract_ref::<E>()?;
        let join = nominal_contract_ref::<J>()?;
        if handled == never_ref()? {
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
        );
        protected(&mut protected_expansion).map_err(authoring_error)?;
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
        );
        handler(&mut handler_expansion).map_err(authoring_error)?;
        if !handler_expansion.draft.is_state_first() {
            return Err(ProgramError::InvalidContract);
        }
        handler_expansion
            .draft
            .classify_join_or_terminal(&join, &self.output_contract_ref)?;

        let mut composite = ExpansionDraft::new(current);
        composite.open_success = None;
        let (protected_entry, protected_open, protected_success, protected_failures) =
            composite.merge_unconnected(protected_expansion.draft)?;
        composite.entry = Some(protected_entry);
        let (handler_entry, handler_open, handler_success, handler_failures) =
            composite.merge_unconnected(handler_expansion.draft)?;
        composite.patch_failure_tails(&handled_tails, handler_entry)?;
        composite.open_success = merge_frontiers(protected_open, handler_open, &join)?;
        composite.scope_success = protected_success;
        composite.scope_success.extend(handler_success);
        composite.open_failures = protected_failures;
        composite.open_failures.extend(handler_failures);
        self.draft.merge_connected(composite)
    }

    fn require_current(&self, expected: &ContentRef) -> Result<()> {
        self.draft.require_current(expected)
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
    join_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    admitted_failure_contract_refs: Vec<ContentRef>,
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
        require_payload::<P>(&self.variants[variant_index])?;
        let input = nominal_contract_ref::<P>()?;
        let depth = nested_callback_depth(self.callback_depth)?;
        let mut branch_expansion = OperationExpansion::new(
            input,
            self.output_contract_ref.clone(),
            self.failure_contract_ref.clone(),
            self.admitted_failure_contract_refs.clone(),
            depth,
        );
        branch(&mut branch_expansion).map_err(authoring_error)?;
        if !branch_expansion.draft.is_state_first() {
            return Err(ProgramError::InvalidContract);
        }
        branch_expansion
            .draft
            .classify_join_or_terminal(&self.join_contract_ref, &self.output_contract_ref)?;
        self.draft.reserve_match_variant()?;
        let (entry, open, scope_success, open_failures) =
            self.draft.merge_unconnected(branch_expansion.draft)?;
        self.draft.add_match_variant(tag, entry);
        self.draft.open_success = merge_frontiers(
            self.draft.open_success.take(),
            open,
            &self.join_contract_ref,
        )?;
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
}

impl InjectionWriter {
    /// Appends one deterministic Pure support State.
    pub fn pure<S: PureState>(&mut self) -> Result<()> {
        append_pure::<S>(&mut self.draft, &self.required_failure_contract_ref, &[])
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

enum DraftDeclaration {
    State(Box<DraftState>),
    Match(DraftMatch),
}

struct Frontier {
    contract_ref: ContentRef,
    tails: Vec<DraftId>,
}

struct ExpansionDraft {
    declarations: Vec<DraftDeclaration>,
    state_count: usize,
    entry: Option<DraftId>,
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
            entry: None,
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
            entry: Some(DraftId(0)),
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

    fn require_failures(&self, expected: &ContentRef) -> Result<()> {
        self.open_failures.iter().try_for_each(|id| {
            (self.state(*id)?.failure_contract_ref == *expected)
                .then_some(())
                .ok_or(ProgramError::InvalidContract)
        })
    }

    fn seal_root_success(&mut self, expected: &ContentRef) -> Result<()> {
        if let Some(frontier) = self.open_success.take() {
            if frontier.contract_ref != *expected {
                self.open_success = Some(frontier);
                return Err(ProgramError::InvalidContract);
            }
            self.patch_success_tails(&frontier.tails, DraftRoute::Terminal)?;
            self.scope_success.extend(frontier.tails);
        }
        (!self.scope_success.is_empty()
            && self.scope_success.iter().all(|id| {
                self.state(*id).is_ok_and(|state| {
                    state.output_contract_ref == *expected && state.next == DraftRoute::Terminal
                })
            }))
        .then_some(())
        .ok_or(ProgramError::InvalidContract)
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
        let scratch_entry = scratch.entry.ok_or(ProgramError::InvalidContract)?;
        if self.entry_contract(scratch_entry, &scratch)? != &parent_frontier.contract_ref {
            return Err(ProgramError::InvalidContract);
        }
        let parent_tails = parent_frontier.tails.clone();
        let parent_entry_missing = self.entry.is_none();
        let offset = self.preflight_merge(&scratch)?;
        let scratch = scratch.rebase(offset)?;
        let entry = scratch.entry.ok_or(ProgramError::InvalidContract)?;
        self.patch_success_tails(&parent_tails, DraftRoute::Target(entry))?;
        self.open_success = scratch.open_success;
        if parent_entry_missing {
            self.entry = Some(entry);
        }
        self.state_count += scratch.state_count;
        self.declarations.extend(scratch.declarations);
        self.scope_success.extend(scratch.scope_success);
        self.open_failures.extend(scratch.open_failures);
        Ok(())
    }

    fn merge_unconnected(&mut self, scratch: Self) -> Result<DetachedExits> {
        let offset = self.preflight_merge(&scratch)?;
        let scratch = scratch.rebase(offset)?;
        let entry = scratch.entry.ok_or(ProgramError::InvalidContract)?;
        let open_success = scratch.open_success;
        let scope_success = scratch.scope_success;
        let open_failures = scratch.open_failures;
        self.state_count += scratch.state_count;
        self.declarations.extend(scratch.declarations);
        Ok((entry, open_success, scope_success, open_failures))
    }

    fn preflight_merge(&mut self, scratch: &Self) -> Result<usize> {
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
        self.declarations
            .try_reserve(scratch.declarations.len())
            .map_err(|_| ProgramError::Capacity)?;
        self.scope_success
            .try_reserve(scratch.scope_success.len())
            .map_err(|_| ProgramError::Capacity)?;
        self.open_failures
            .try_reserve(scratch.open_failures.len())
            .map_err(|_| ProgramError::Capacity)?;
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
                    rebase_route(&mut state.next, offset)?;
                    rebase_route(&mut state.failure_next, offset)?;
                }
                DraftDeclaration::Match(selector) => {
                    for variant in &mut selector.variants {
                        rebase(&mut variant.entry)?;
                    }
                }
            }
        }
        if let Some(entry) = &mut self.entry {
            rebase(entry)?;
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

    fn reserve_match_variant(&mut self) -> Result<()> {
        let Some(DraftDeclaration::Match(selector)) = self.declarations.first_mut() else {
            return Err(ProgramError::InvalidContract);
        };
        selector
            .variants
            .try_reserve(1)
            .map_err(|_| ProgramError::Capacity)
    }

    fn add_match_variant(&mut self, tag: StableId, entry: DraftId) {
        if let Some(DraftDeclaration::Match(selector)) = self.declarations.first_mut() {
            selector.variants.push(DraftMatchVariant { tag, entry });
        }
    }

    fn finish_match_frontier(&mut self, join: ContentRef) -> Result<()> {
        if self.open_success.is_none() && self.scope_success.is_empty() {
            return Err(ProgramError::InvalidContract);
        }
        if let Some(frontier) = &self.open_success {
            if frontier.contract_ref != join || frontier.tails.is_empty() {
                return Err(ProgramError::InvalidContract);
            }
        }
        Ok(())
    }

    fn is_state_first(&self) -> bool {
        matches!(
            self.entry.and_then(|entry| self.declarations.get(entry.0)),
            Some(DraftDeclaration::State(_))
        )
    }

    fn entry_contract<'a>(&self, id: DraftId, draft: &'a Self) -> Result<&'a ContentRef> {
        match draft
            .declarations
            .get(id.0)
            .ok_or(ProgramError::InvalidContract)?
        {
            DraftDeclaration::State(state) => Ok(&state.input_contract_ref),
            DraftDeclaration::Match(selector) => Ok(&selector.selector_contract_ref),
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

    fn require_resolved(&self) -> Result<()> {
        if self.open_success.is_some() || !self.open_failures.is_empty() {
            return Err(ProgramError::InvalidContract);
        }
        for declaration in &self.declarations {
            if let DraftDeclaration::State(state) = declaration {
                if state.next == DraftRoute::Open || state.failure_next == DraftRoute::Open {
                    return Err(ProgramError::InvalidContract);
                }
            }
        }
        Ok(())
    }

    fn into_declarations(self) -> Result<Vec<Declaration>> {
        let mut declarations = Vec::new();
        declarations
            .try_reserve(self.declarations.len())
            .map_err(|_| ProgramError::Capacity)?;
        for (index, declaration) in self.declarations.into_iter().enumerate() {
            declarations.push(match declaration {
                DraftDeclaration::State(state) => Declaration::State(StateDeclaration::new(
                    state.state_implementation_ref,
                    state.input_contract_ref,
                    state.output_contract_ref,
                    state.failure_contract_ref,
                    state.execution,
                    lower_route(index, state.next)?,
                    lower_route(index, state.failure_next)?,
                )),
                DraftDeclaration::Match(selector) => Declaration::Match(MatchDeclaration::new(
                    selector.selector_contract_ref,
                    selector
                        .variants
                        .into_iter()
                        .map(|variant| {
                            Ok(MatchVariant::new(
                                variant.tag,
                                lower_target(index, variant.entry)?,
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?,
                )?),
            });
        }
        Ok(declarations)
    }
}

fn append_pure<S: PureState>(
    draft: &mut ExpansionDraft,
    scope_failure: &ContentRef,
    admitted_failures: &[ContentRef],
) -> Result<()> {
    append_state_core(
        draft,
        state_implementation_ref::<S>()?,
        nominal_contract_ref::<S::Input>()?,
        nominal_contract_ref::<S::Output>()?,
        nominal_contract_ref::<S::Failure>()?,
        Execution::pure(),
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
    scope_failure: &ContentRef,
    admitted_failures: &[ContentRef],
) -> Result<()> {
    let never = never_ref()?;
    if input_contract_ref == never || output_contract_ref == never {
        return Err(ProgramError::InvalidContract);
    }
    draft.require_current(&input_contract_ref)?;
    require_legal_failure(&failure_contract_ref, scope_failure, admitted_failures)?;
    if draft.declarations.len() >= MAX_DECLARATIONS || draft.state_count >= MAX_STATES {
        return Err(ProgramError::Capacity);
    }
    draft
        .declarations
        .try_reserve(1)
        .map_err(|_| ProgramError::Capacity)?;
    if failure_contract_ref != never {
        draft
            .open_failures
            .try_reserve(1)
            .map_err(|_| ProgramError::Capacity)?;
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
    if draft.entry.is_none() {
        draft.entry = Some(id);
    }
    let failure_next = if failure_contract_ref == never {
        DraftRoute::Terminal
    } else {
        draft.open_failures.push(id);
        DraftRoute::Open
    };
    draft
        .declarations
        .push(DraftDeclaration::State(Box::new(DraftState {
            state_implementation_ref,
            input_contract_ref,
            output_contract_ref: output_contract_ref.clone(),
            failure_contract_ref,
            execution,
            next: DraftRoute::Open,
            failure_next,
        })));
    draft.state_count += 1;
    draft.open_success = Some(Frontier {
        contract_ref: output_contract_ref,
        tails: vec![id],
    });
    Ok(())
}

fn require_legal_failure(
    failure: &ContentRef,
    scope_failure: &ContentRef,
    admitted_failures: &[ContentRef],
) -> Result<()> {
    if failure == &never_ref()? || failure == scope_failure || admitted_failures.contains(failure) {
        Ok(())
    } else {
        Err(ProgramError::InvalidContract)
    }
}

fn nested_callback_depth(current: u8) -> Result<u8> {
    current
        .checked_add(1)
        .filter(|depth| *depth <= MAX_AUTHORING_CALLBACK_DEPTH)
        .ok_or(ProgramError::Capacity)
}

fn authoring_error(error: ProgramError) -> ProgramError {
    match error {
        ProgramError::Capacity => ProgramError::Capacity,
        ProgramError::Canonical | ProgramError::InvalidContract => ProgramError::InvalidContract,
    }
}

fn rebase_route(route: &mut DraftRoute, offset: usize) -> Result<()> {
    if let DraftRoute::Target(id) = route {
        id.0 = id.0.checked_add(offset).ok_or(ProgramError::Capacity)?;
    }
    Ok(())
}

fn merge_frontiers(
    left: Option<Frontier>,
    right: Option<Frontier>,
    contract: &ContentRef,
) -> Result<Option<Frontier>> {
    match (left, right) {
        (None, None) => Ok(None),
        (Some(frontier), None) | (None, Some(frontier)) if frontier.contract_ref == *contract => {
            Ok(Some(frontier))
        }
        (Some(mut left), Some(right))
            if left.contract_ref == *contract && right.contract_ref == *contract =>
        {
            left.tails
                .try_reserve(right.tails.len())
                .map_err(|_| ProgramError::Capacity)?;
            left.tails.extend(right.tails);
            Ok(Some(left))
        }
        _ => Err(ProgramError::InvalidContract),
    }
}

fn lower_route(source: usize, route: DraftRoute) -> Result<Option<u16>> {
    match route {
        DraftRoute::Open => Err(ProgramError::InvalidContract),
        DraftRoute::Terminal => Ok(None),
        DraftRoute::Target(target) => lower_target(source, target).map(Some),
    }
}

fn lower_target(source: usize, target: DraftId) -> Result<u16> {
    if target.0 <= source {
        return Err(ProgramError::InvalidContract);
    }
    u16::try_from(target.0).map_err(|_| ProgramError::Capacity)
}

struct SelectorVariant {
    tag: String,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    serialized_shape: SchemaShape,
    seen: bool,
}

fn selector_variants<T: MfmValue>() -> Result<Vec<SelectorVariant>> {
    let descriptor = T::schema_descriptor().map_err(|_| ProgramError::InvalidContract)?;
    let shape = descriptor
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
        .map(|variant| {
            StableId::new(&variant.name).map_err(|_| ProgramError::InvalidContract)?;
            let (schema_id, semantic_type_id, serialized_shape) =
                match_payload_descriptor(&variant.shape).ok_or(ProgramError::InvalidContract)?;
            Ok(SelectorVariant {
                tag: variant.name.clone(),
                schema_id: schema_id.clone(),
                semantic_type_id: semantic_type_id.clone(),
                serialized_shape: serialized_shape.clone(),
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

fn require_payload<P: MfmValue>(variant: &SelectorVariant) -> Result<()> {
    let descriptor = P::schema_descriptor().map_err(|_| ProgramError::InvalidContract)?;
    let schema_id = descriptor
        .schema_id()
        .map_err(|_| ProgramError::InvalidContract)?;
    let semantic_type_id = P::semantic_id().map_err(|_| ProgramError::InvalidContract)?;
    let serialized_shape = descriptor
        .identity()
        .canonical_json_shape()
        .map_err(|_| ProgramError::InvalidContract)?;
    (variant.schema_id == schema_id
        && variant.semantic_type_id == semantic_type_id
        && variant.serialized_shape == *serialized_shape)
        .then_some(())
        .ok_or(ProgramError::InvalidContract)
}
