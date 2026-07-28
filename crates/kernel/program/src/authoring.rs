use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

use mfm_ids::{ContentRef, FieldPath, StableId};
use mfm_journal::v1::ValueRef;
use mfm_spec::{
    AuthoredBaseKind, AuthoredInputBinding, AuthoredNode, AuthoredPublicOutputBinding,
    AuthoredSourceSelector, CanonicalAuthoredProgram,
};

use crate::{ProgramError, Result, State};

/// Exact retained config and optional context for one authored occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateBindings {
    config_ref: ValueRef,
    context_ref: Option<ValueRef>,
}

impl StateBindings {
    /// Constructs complete config and context authority.
    pub const fn new(config_ref: ValueRef, context_ref: Option<ValueRef>) -> Self {
        Self {
            config_ref,
            context_ref,
        }
    }

    fn into_parts(self) -> (ValueRef, Option<ValueRef>) {
        (self.config_ref, self.context_ref)
    }
}

/// Typed handle for one exact authored output selection.
#[derive(Debug, PartialEq, Eq)]
pub struct AuthoredHandle<T> {
    node_key: StableId,
    output_ordinal: u32,
    source_field_path: Option<FieldPath>,
    _value: PhantomData<fn(T) -> T>,
}

impl<T> Clone for AuthoredHandle<T> {
    fn clone(&self) -> Self {
        Self {
            node_key: self.node_key.clone(),
            output_ordinal: self.output_ordinal,
            source_field_path: self.source_field_path.clone(),
            _value: PhantomData,
        }
    }
}

impl<T> AuthoredHandle<T> {
    /// Returns the globally stable authored node key.
    pub const fn node_key(&self) -> &StableId {
        &self.node_key
    }

    /// Returns the selected output slot.
    pub const fn output_ordinal(&self) -> u32 {
        self.output_ordinal
    }

    /// Returns the selected nested field, or `None` for the whole output slot.
    pub const fn source_field_path(&self) -> Option<&FieldPath> {
        self.source_field_path.as_ref()
    }

    /// Selects one nested field while preserving the producer and output slot.
    ///
    /// Certification independently proves that the selected field's exact
    /// retained-value contract matches its consumer.
    pub fn field<U>(&self, field_path: FieldPath) -> AuthoredHandle<U> {
        AuthoredHandle {
            node_key: self.node_key.clone(),
            output_ordinal: self.output_ordinal,
            source_field_path: Some(field_path),
            _value: PhantomData,
        }
    }

    fn source(&self) -> AuthoredSourceSelector {
        AuthoredSourceSelector::NodeOutput {
            producer_key: self.node_key.clone(),
            producer_output_ordinal: self.output_ordinal,
            source_field_path: self.source_field_path.clone(),
        }
    }
}

/// One authored state occurrence with typed successful output.
pub struct AuthoredState<S: State> {
    node_key: StableId,
    output: AuthoredHandle<S::Output>,
    _state: PhantomData<fn(S) -> S>,
}

impl<S: State> AuthoredState<S> {
    /// Returns the authored occurrence key.
    pub const fn node_key(&self) -> &StableId {
        &self.node_key
    }

    /// Returns output slot zero as the state's typed root output.
    pub const fn output(&self) -> &AuthoredHandle<S::Output> {
        &self.output
    }

    /// Selects another certified output slot.
    ///
    /// The state registration, planner, and certifier remain authoritative for
    /// the slot's exact type and contract.
    pub fn output_slot<T>(&self, output_ordinal: u32) -> AuthoredHandle<T> {
        AuthoredHandle {
            node_key: self.node_key.clone(),
            output_ordinal,
            source_field_path: None,
            _value: PhantomData,
        }
    }

    /// Consumes the occurrence into output slot zero.
    pub fn into_output(self) -> AuthoredHandle<S::Output> {
        self.output
    }
}

/// Deterministic, authority-free authored-program surface.
pub struct AuthoredProgramBuilder {
    entry_point_operation_id: StableId,
    child_path: Vec<StableId>,
    nodes: Vec<AuthoredNode>,
    input_bindings: Vec<AuthoredInputBinding>,
    public_output_bindings: Vec<AuthoredPublicOutputBinding>,
    required_success_node_keys: BTreeSet<StableId>,
    node_keys: BTreeSet<StableId>,
    child_paths: BTreeSet<Vec<StableId>>,
    next_source_ordinals: BTreeMap<(StableId, u32), u32>,
}

impl AuthoredProgramBuilder {
    /// Starts one entry-point authored program.
    pub fn new(entry_point_operation_id: StableId) -> Self {
        Self {
            entry_point_operation_id,
            child_path: Vec::new(),
            nodes: Vec::new(),
            input_bindings: Vec::new(),
            public_output_bindings: Vec::new(),
            required_success_node_keys: BTreeSet::new(),
            node_keys: BTreeSet::new(),
            child_paths: BTreeSet::new(),
            next_source_ordinals: BTreeMap::new(),
        }
    }

    /// Authors one deterministic nested child operation.
    pub fn child<T>(
        &mut self,
        stable_key: StableId,
        author: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.child_path.push(stable_key);
        if !self.child_paths.insert(self.child_path.clone()) {
            self.child_path.pop();
            return Err(ProgramError::Authoring(
                "duplicate child-operation path".to_owned(),
            ));
        }
        let result = author(self);
        self.child_path.pop();
        result
    }

    /// Authors one typed state occurrence with complete retained bindings.
    pub fn state<S: State>(
        &mut self,
        local_stable_key: StableId,
        bindings: StateBindings,
    ) -> Result<AuthoredState<S>> {
        let node_key = self.scoped_key(&local_stable_key)?;
        self.insert_node(
            node_key.clone(),
            local_stable_key,
            AuthoredBaseKind::Authored,
            S::state_contract_ref()?,
            bindings,
        )?;
        Ok(AuthoredState {
            node_key: node_key.clone(),
            output: AuthoredHandle {
                node_key,
                output_ordinal: 0,
                source_field_path: None,
                _value: PhantomData,
            },
            _state: PhantomData,
        })
    }

    /// Authors an ordinary same-value bridge occurrence.
    pub fn bridge<T>(
        &mut self,
        local_stable_key: StableId,
        bridge_state_contract_ref: ContentRef,
        input: &AuthoredHandle<T>,
        bindings: StateBindings,
    ) -> Result<AuthoredHandle<T>> {
        let node_key = self.scoped_key(&local_stable_key)?;
        self.insert_node(
            node_key.clone(),
            local_stable_key,
            AuthoredBaseKind::Bridge,
            bridge_state_contract_ref,
            bindings,
        )?;
        self.push_input_source(node_key.clone(), 0, input.source())?;
        Ok(AuthoredHandle {
            node_key,
            output_ordinal: 0,
            source_field_path: None,
            _value: PhantomData,
        })
    }

    /// Connects a whole typed producer output to input destination zero.
    pub fn connect<T, S>(
        &mut self,
        producer: &AuthoredHandle<T>,
        consumer: &AuthoredState<S>,
    ) -> Result<()>
    where
        S: State<Input = T>,
    {
        self.push_input_source(consumer.node_key.clone(), 0, producer.source())
    }

    /// Binds one exact source to a declared consumer input destination.
    pub fn bind_input_source<S: State>(
        &mut self,
        consumer: &AuthoredState<S>,
        consumer_input_ordinal: u32,
        source: AuthoredSourceSelector,
    ) -> Result<()> {
        self.push_input_source(consumer.node_key.clone(), consumer_input_ordinal, source)
    }

    /// Binds one same-run producer selection to an input destination.
    pub fn connect_to<T, S: State>(
        &mut self,
        producer: &AuthoredHandle<T>,
        consumer: &AuthoredState<S>,
        consumer_input_ordinal: u32,
    ) -> Result<()> {
        self.bind_input_source(consumer, consumer_input_ordinal, producer.source())
    }

    /// Binds one typed output as a public output field.
    pub fn public_output<T>(&mut self, field: StableId, output: &AuthoredHandle<T>) -> Result<()> {
        if self
            .public_output_bindings
            .iter()
            .any(|binding| binding.field() == &field)
        {
            return Err(ProgramError::Authoring(
                "duplicate public output field".to_owned(),
            ));
        }
        self.public_output_bindings
            .push(AuthoredPublicOutputBinding::new(
                field,
                output.node_key.clone(),
                output.output_ordinal,
                output.source_field_path.clone(),
            ));
        Ok(())
    }

    /// Marks one occurrence as required for run success.
    pub fn required_success<T>(&mut self, output: &AuthoredHandle<T>) -> Result<()> {
        if !self.node_keys.contains(&output.node_key) {
            return Err(ProgramError::Authoring(
                "required-success handle does not belong to this program".to_owned(),
            ));
        }
        if !self
            .required_success_node_keys
            .insert(output.node_key.clone())
        {
            return Err(ProgramError::Authoring(
                "duplicate required-success node".to_owned(),
            ));
        }
        Ok(())
    }

    /// Finishes the exact canonical authored program.
    pub fn finish(self) -> Result<CanonicalAuthoredProgram> {
        CanonicalAuthoredProgram::new(
            self.entry_point_operation_id,
            self.nodes,
            self.input_bindings,
            self.public_output_bindings,
            self.required_success_node_keys.into_iter().collect(),
        )
        .map_err(|error| ProgramError::Authoring(error.to_string()))
    }

    fn insert_node(
        &mut self,
        node_key: StableId,
        local_stable_key: StableId,
        base_kind: AuthoredBaseKind,
        state_contract_ref: ContentRef,
        bindings: StateBindings,
    ) -> Result<()> {
        if !self.node_keys.insert(node_key.clone()) {
            return Err(ProgramError::Authoring(format!(
                "duplicate authored node key {node_key}"
            )));
        }
        let (config_ref, context_ref) = bindings.into_parts();
        self.nodes.push(AuthoredNode::new(
            node_key,
            local_stable_key,
            self.child_path.clone(),
            base_kind,
            state_contract_ref,
            config_ref,
            context_ref,
        )?);
        Ok(())
    }

    fn push_input_source(
        &mut self,
        consumer_key: StableId,
        consumer_input_ordinal: u32,
        source: AuthoredSourceSelector,
    ) -> Result<()> {
        if !self.node_keys.contains(&consumer_key) {
            return Err(ProgramError::Authoring(
                "input consumer does not belong to this program".to_owned(),
            ));
        }
        if source
            .producer_key()
            .is_some_and(|producer| !self.node_keys.contains(producer))
        {
            return Err(ProgramError::Authoring(
                "input producer does not belong to this program".to_owned(),
            ));
        }
        let key = (consumer_key.clone(), consumer_input_ordinal);
        let source_ordinal = *self.next_source_ordinals.get(&key).unwrap_or(&0);
        let next = source_ordinal.checked_add(1).ok_or_else(|| {
            ProgramError::Authoring("input source ordinal exceeds u32".to_owned())
        })?;
        self.next_source_ordinals.insert(key, next);
        self.input_bindings.push(AuthoredInputBinding::new(
            consumer_key,
            consumer_input_ordinal,
            source_ordinal,
            source,
        ));
        Ok(())
    }

    fn scoped_key(&self, local: &StableId) -> Result<StableId> {
        if self.child_path.is_empty() {
            return Ok(local.clone());
        }
        StableId::new(format!(
            "{}/{}",
            self.child_path
                .iter()
                .map(StableId::as_str)
                .collect::<Vec<_>>()
                .join("/"),
            local.as_str()
        ))
        .map_err(|error| ProgramError::Authoring(error.to_string()))
    }
}

/// Deterministic operation authoring contract.
pub trait Operation {
    /// Typed handles returned to the caller.
    type Output;

    /// Authors frozen topology into one authority-free builder.
    fn author(&self, builder: &mut AuthoredProgramBuilder) -> Result<Self::Output>;
}

/// Authors one operation into exact canonical retained bytes.
pub fn author_program<O: Operation>(
    entry_point_operation_id: StableId,
    operation: &O,
    bind_output: impl FnOnce(&mut AuthoredProgramBuilder, &O::Output) -> Result<()>,
) -> Result<CanonicalAuthoredProgram> {
    let mut builder = AuthoredProgramBuilder::new(entry_point_operation_id);
    let output = operation.author(&mut builder)?;
    bind_output(&mut builder, &output)?;
    builder.finish()
}
