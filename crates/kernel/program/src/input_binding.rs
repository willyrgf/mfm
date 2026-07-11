use super::*;

/// Stable field path inside a state input binding tree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputFieldPath(CheckedFieldPath);

impl InputFieldPath {
    /// Creates the root input field path.
    pub fn root() -> Self {
        Self(
            CheckedFieldPath::new("root")
                .expect("static root input field path must satisfy field-path grammar"),
        )
    }

    /// Creates a checked input field path.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_field_path("input field path", value.as_ref()).map(Self)
    }

    /// Appends a checked child segment.
    pub fn child(&self, value: impl AsRef<str>) -> Result<Self> {
        let segment = checked_field_segment("input field segment", value.as_ref())?;
        if self.0.as_str() == "root" {
            checked_field_path("input field path", segment.as_str()).map(Self)
        } else {
            checked_field_path(
                "input field path",
                format!("{}.{}", self.0.as_str(), segment.as_str()),
            )
            .map(Self)
        }
    }

    /// Returns the stable field path string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Required terminal behavior for an input cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequiredTerminal {
    /// The input requires a produced value.
    ProducedOnly,
    /// The input accepts produced or skipped optional values.
    MaybeSkipped,
}

impl RequiredTerminal {
    fn from_value_policy(policy: ValueTerminalPolicy) -> Self {
        match policy {
            ValueTerminalPolicy::ProducedOnly => Self::ProducedOnly,
            ValueTerminalPolicy::MaybeSkipped => Self::MaybeSkipped,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ProducedOnly => "produced_only",
            Self::MaybeSkipped => "maybe_skipped",
        }
    }
}

/// Ordering evidence for vector input bindings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OrderingEvidence {
    /// Author-provided vector order.
    ExplicitAuthorOrder,
    /// Canonical order by stable domain key.
    StableDomainKey,
}

impl OrderingEvidence {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::ExplicitAuthorOrder => "explicit_author_order",
            Self::StableDomainKey => "stable_domain_key",
        }
    }
}

/// Named field binding inside a struct input binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedInputBinding {
    /// Field path for this named binding.
    pub field_path: InputFieldPath,
    /// Binding node for this field.
    pub node: InputBindingNode,
}

impl NamedInputBinding {
    /// Creates a named input binding.
    pub fn new(field_path: InputFieldPath, node: InputBindingNode) -> Self {
        Self { field_path, node }
    }
}

/// Canonical typed state-input binding tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBindingNode {
    pub(super) kind: InputBindingNodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InputBindingNodeKind {
    Unit,
    Cell(Box<InputCellBinding>),
    Tuple {
        elements: Vec<InputBindingNode>,
    },
    Struct {
        fields: Vec<NamedInputBinding>,
    },
    Vec {
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
        domain_keys: Vec<StableDomainKeyRef>,
    },
    NonEmptyVec {
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
        domain_keys: Vec<StableDomainKeyRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InputCellBinding {
    pub(super) field_path: InputFieldPath,
    pub(super) cell_id: CellId,
    pub(super) semantic_type_id: SemanticTypeId,
    pub(super) schema_id: SchemaId,
    pub(super) value_lineage: ValueLineageRef,
    pub(super) required_terminal: RequiredTerminal,
    pub(super) context: InputContextSpec,
}

/// Read-only view over a typed input cell binding.
#[derive(Debug, Clone, Copy)]
pub struct InputCellBindingRef<'a> {
    binding: &'a InputCellBinding,
}

impl<'a> InputCellBindingRef<'a> {
    /// Returns the input field path.
    pub fn field_path(&self) -> &'a InputFieldPath {
        &self.binding.field_path
    }

    /// Returns the referenced cell id.
    pub fn cell_id(&self) -> &'a CellId {
        &self.binding.cell_id
    }

    /// Returns the referenced semantic type id.
    pub fn semantic_type_id(&self) -> &'a SemanticTypeId {
        &self.binding.semantic_type_id
    }

    /// Returns the referenced schema id.
    pub fn schema_id(&self) -> &'a SchemaId {
        &self.binding.schema_id
    }

    /// Returns the referenced value-lineage ref.
    pub fn value_lineage(&self) -> &'a ValueLineageRef {
        &self.binding.value_lineage
    }

    /// Returns the required terminal policy.
    pub fn required_terminal(&self) -> RequiredTerminal {
        self.binding.required_terminal
    }

    /// Returns the certified context constraint required by this input cell.
    pub fn context(&self) -> &InputContextSpec {
        &self.binding.context
    }
}

/// Read-only view over a typed input binding node.
#[derive(Debug, Clone, Copy)]
pub enum InputBindingNodeRef<'a> {
    /// Unit input.
    Unit,
    /// Typed cell input.
    Cell(InputCellBindingRef<'a>),
    /// Tuple input.
    Tuple(&'a [InputBindingNode]),
    /// Struct input.
    Struct(&'a [NamedInputBinding]),
    /// Vector input.
    Vec {
        /// Element bindings.
        elements: &'a [InputBindingNode],
        /// Ordering evidence.
        ordering: &'a OrderingEvidence,
        /// Stable domain-key refs.
        domain_keys: &'a [StableDomainKeyRef],
    },
    /// Non-empty vector input.
    NonEmptyVec {
        /// Element bindings.
        elements: &'a [InputBindingNode],
        /// Ordering evidence.
        ordering: &'a OrderingEvidence,
        /// Stable domain-key refs.
        domain_keys: &'a [StableDomainKeyRef],
    },
}

impl InputBindingNode {
    /// Unit input binding node.
    #[allow(non_upper_case_globals)]
    pub const Unit: Self = Self {
        kind: InputBindingNodeKind::Unit,
    };

    /// Builds a struct binding from named fields, rejecting duplicate paths and sorting canonically.
    pub fn struct_fields(mut fields: Vec<NamedInputBinding>) -> Result<Self> {
        let mut seen = BTreeSet::new();
        for field in &fields {
            if !seen.insert(field.field_path.as_str().to_owned()) {
                return Err(PlanError::DuplicateInputFieldPath(
                    field.field_path.as_str().to_owned(),
                ));
            }
        }
        fields.sort_by(|left, right| left.field_path.cmp(&right.field_path));
        Ok(Self::struct_fields_unchecked(fields))
    }

    fn cell(
        field_path: InputFieldPath,
        cell_id: CellId,
        semantic_type_id: SemanticTypeId,
        schema_id: SchemaId,
        value_lineage: ValueLineageRef,
        required_terminal: RequiredTerminal,
        context: InputContextSpec,
    ) -> Self {
        Self {
            kind: InputBindingNodeKind::Cell(Box::new(InputCellBinding {
                field_path,
                cell_id,
                semantic_type_id,
                schema_id,
                value_lineage,
                required_terminal,
                context,
            })),
        }
    }

    fn tuple(elements: Vec<InputBindingNode>) -> Self {
        Self {
            kind: InputBindingNodeKind::Tuple { elements },
        }
    }

    pub(super) fn struct_fields_unchecked(fields: Vec<NamedInputBinding>) -> Self {
        Self {
            kind: InputBindingNodeKind::Struct { fields },
        }
    }

    fn vector(
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
        domain_keys: Vec<StableDomainKeyRef>,
    ) -> Self {
        Self {
            kind: InputBindingNodeKind::Vec {
                elements,
                ordering,
                domain_keys,
            },
        }
    }

    pub(super) fn non_empty_vector(
        elements: Vec<InputBindingNode>,
        ordering: OrderingEvidence,
        domain_keys: Vec<StableDomainKeyRef>,
    ) -> Self {
        Self {
            kind: InputBindingNodeKind::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            },
        }
    }

    /// Returns a read-only structural view of this binding node.
    pub fn as_ref(&self) -> InputBindingNodeRef<'_> {
        match &self.kind {
            InputBindingNodeKind::Unit => InputBindingNodeRef::Unit,
            InputBindingNodeKind::Cell(binding) => {
                InputBindingNodeRef::Cell(InputCellBindingRef { binding })
            }
            InputBindingNodeKind::Tuple { elements } => InputBindingNodeRef::Tuple(elements),
            InputBindingNodeKind::Struct { fields } => InputBindingNodeRef::Struct(fields),
            InputBindingNodeKind::Vec {
                elements,
                ordering,
                domain_keys,
            } => InputBindingNodeRef::Vec {
                elements,
                ordering,
                domain_keys,
            },
            InputBindingNodeKind::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            } => InputBindingNodeRef::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            },
        }
    }
}

/// Author-side conversion from handles into a binding node for a runtime input type.
pub trait IntoInputBindingNode<I> {
    /// Converts this author-side input into a binding node at `field_path`.
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode>;
}

/// Typed state-input binding with descriptor and canonical digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBinding<I: StateInput> {
    input_schema_id: SchemaId,
    input_descriptor_id: DescriptorId,
    root: InputBindingNode,
    digest: ContentDigest,
    _input: PhantomData<fn(I) -> I>,
}

impl<I: StateInput> InputBinding<I> {
    /// Creates an input binding from a canonical root binding node.
    pub fn from_root(root: InputBindingNode) -> Result<Self> {
        validate_input_binding_node(&root)?;
        let descriptor =
            I::input_schema_descriptor().map_err(|error| PlanError::Value(error.to_string()))?;
        validate_input_binding_node_shape(&root, &descriptor.identity.shape)?;
        let input_schema_id = descriptor
            .schema_id()
            .map_err(|error| PlanError::Value(error.to_string()))?;
        let input_descriptor_id = input_descriptor_id(&input_schema_id)?;
        let digest = input_binding_digest(&root)?;
        Ok(Self {
            input_schema_id,
            input_descriptor_id,
            root,
            digest,
            _input: PhantomData,
        })
    }

    /// Returns the input schema id.
    pub fn input_schema_id(&self) -> &SchemaId {
        &self.input_schema_id
    }

    /// Returns the input descriptor id.
    pub fn input_descriptor_id(&self) -> &DescriptorId {
        &self.input_descriptor_id
    }

    /// Returns the root binding node.
    pub fn root(&self) -> &InputBindingNode {
        &self.root
    }

    /// Returns the canonical binding digest.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Returns an unbranded persisted input binding spec.
    pub fn spec(&self) -> InputBindingSpec {
        InputBindingSpec {
            input_schema_id: self.input_schema_id.clone(),
            input_descriptor_id: self.input_descriptor_id.clone(),
            root: self.root.clone(),
            digest: self.digest.clone(),
        }
    }
}

/// Persisted state-input binding spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBindingSpec {
    /// Input schema id.
    pub input_schema_id: SchemaId,
    /// Input descriptor id.
    pub input_descriptor_id: DescriptorId,
    /// Root input binding node.
    pub root: InputBindingNode,
    /// Canonical digest of the root binding tree.
    pub digest: ContentDigest,
}

/// Persisted operation-input binding spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationInputBindingSpec {
    /// Input schema id.
    pub input_schema_id: SchemaId,
    /// Input descriptor id.
    pub input_descriptor_id: DescriptorId,
    /// Root input binding node.
    pub root: InputBindingNode,
    /// Canonical digest of the root binding tree.
    pub digest: ContentDigest,
}

impl OperationInputBindingSpec {
    /// Converts a validated state-input binding into an operation-input binding.
    pub fn from_binding<I: StateInput>(binding: InputBinding<I>) -> Self {
        Self::from_state_input_spec(binding.spec())
    }

    fn from_state_input_spec(spec: InputBindingSpec) -> Self {
        Self {
            input_schema_id: spec.input_schema_id,
            input_descriptor_id: spec.input_descriptor_id,
            root: spec.root,
            digest: spec.digest,
        }
    }
}

/// Typed operation input contract over framework-owned branded handle wrappers.
pub trait OperationInput<'program, 'scope>: private::OperationInputSealed {
    /// Runtime input descriptor represented by this handle-side input.
    type Runtime: StateInput;

    /// Returns the input schema id.
    fn input_schema_id() -> mfm_values::Result<SchemaId> {
        Self::Runtime::input_schema_id()
    }

    /// Returns the canonical operation input binding.
    fn input_binding(&self) -> Result<OperationInputBindingSpec>;
}

/// Author-side conversion into an operation's typed input value.
pub trait IntoOperationInput<'program, 'scope, I: OperationInput<'program, 'scope>> {
    /// Converts into the operation input expected by `Operation::expand`.
    fn into_operation_input(self) -> Result<I>;
}

impl<'program, 'scope, I> IntoOperationInput<'program, 'scope, I> for I
where
    I: OperationInput<'program, 'scope>,
{
    fn into_operation_input(self) -> Result<I> {
        Ok(self)
    }
}

/// Typed operation output contract over branded handles.
pub trait OperationOutput<'program, 'scope> {
    /// Returns the operation output schema id.
    fn output_schema_id() -> Result<SchemaId>
    where
        Self: Sized;

    /// Returns typed handles actually produced by this operation expansion.
    fn output_handles(&self) -> Result<Vec<TypedHandleRef>>;
}

/// Converts author-side handle values into a typed state-input binding.
pub trait IntoStateInput<'program, 'scope, I: StateInput> {
    /// Converts into a typed input binding.
    fn into_binding(self) -> Result<InputBinding<I>>;
}

impl IntoInputBindingNode<()> for () {
    fn into_binding_node(self, _field_path: InputFieldPath) -> Result<InputBindingNode> {
        Ok(InputBindingNode::Unit)
    }
}

impl<'program, 'scope> IntoStateInput<'program, 'scope, ()> for () {
    fn into_binding(self) -> Result<InputBinding<()>> {
        InputBinding::from_root(InputBindingNode::Unit)
    }
}

impl<'program, 'scope, T> IntoInputBindingNode<T> for Handle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        let typed_ref = self.typed_ref();
        Ok(InputBindingNode::cell(
            field_path,
            typed_ref.cell_id,
            typed_ref.semantic_type_id,
            typed_ref.schema_id,
            typed_ref.value_lineage,
            RequiredTerminal::from_value_policy(T::terminal_policy()),
            input_context_from_cell_context(&typed_ref.context),
        ))
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, T> for Handle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<T>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

impl<'program, 'scope, T> IntoInputBindingNode<T> for ForwardSideEffectHandle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        self.into_handle().into_binding_node(field_path)
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, T>
    for ForwardSideEffectHandle<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<T>> {
        self.into_handle().into_binding()
    }
}

impl<'program, 'scope, T> IntoInputBindingNode<Vec<T>> for Vec<Handle<'program, 'scope, T>>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        let elements = self
            .into_iter()
            .enumerate()
            .map(|(index, handle)| handle.into_binding_node(field_path.child(index.to_string())?))
            .collect::<Result<Vec<_>>>()?;
        Ok(InputBindingNode::vector(
            elements,
            OrderingEvidence::ExplicitAuthorOrder,
            Vec::new(),
        ))
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, Vec<T>>
    for Vec<Handle<'program, 'scope, T>>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<Vec<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

macro_rules! impl_tuple_input_binding {
    ($($name:ident:$index:literal),+ $(,)?) => {
        impl<'program, 'scope, $($name),+> IntoInputBindingNode<($($name,)+)>
            for ($(Handle<'program, 'scope, $name>,)+)
        where
            $($name: MfmValue,)+
        {
            fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                let elements = vec![
                    $($name.into_binding_node(field_path.child($index.to_string())?)?,)+
                ];
                Ok(InputBindingNode::tuple(elements))
            }
        }

        impl<'program, 'scope, $($name),+> IntoStateInput<'program, 'scope, ($($name,)+)>
            for ($(Handle<'program, 'scope, $name>,)+)
        where
            $($name: MfmValue,)+
        {
            fn into_binding(self) -> Result<InputBinding<($($name,)+)>> {
                InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
            }
        }
    };
}

impl_tuple_input_binding!(A:0);
impl_tuple_input_binding!(A:0, B:1);
impl_tuple_input_binding!(A:0, B:1, C:2);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
impl_tuple_input_binding!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

/// Author-side non-empty handles.
#[derive(Debug, PartialEq, Eq)]
pub struct NonEmptyHandles<'program, 'scope, T: MfmValue> {
    handles: Vec<Handle<'program, 'scope, T>>,
}

impl<'program, 'scope, T: MfmValue> Clone for NonEmptyHandles<'program, 'scope, T> {
    fn clone(&self) -> Self {
        Self {
            handles: self.handles.clone(),
        }
    }
}

impl<'program, 'scope, T: MfmValue> NonEmptyHandles<'program, 'scope, T> {
    /// Creates a non-empty handle collection from a first handle and optional rest.
    pub fn new(
        first: Handle<'program, 'scope, T>,
        mut rest: Vec<Handle<'program, 'scope, T>>,
    ) -> Self {
        let mut handles = Vec::with_capacity(rest.len() + 1);
        handles.push(first);
        handles.append(&mut rest);
        Self { handles }
    }

    /// Attempts to create a non-empty handle collection from a vector.
    pub fn try_from_vec(handles: Vec<Handle<'program, 'scope, T>>) -> Result<Self> {
        if handles.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        Ok(Self { handles })
    }
}

impl<'program, 'scope, T> IntoInputBindingNode<NonEmpty<T>> for NonEmptyHandles<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        if self.handles.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        let elements = self
            .handles
            .into_iter()
            .enumerate()
            .map(|(index, handle)| handle.into_binding_node(field_path.child(index.to_string())?))
            .collect::<Result<Vec<_>>>()?;
        Ok(InputBindingNode::non_empty_vector(
            elements,
            OrderingEvidence::ExplicitAuthorOrder,
            Vec::new(),
        ))
    }
}

impl<'program, 'scope, T> IntoStateInput<'program, 'scope, NonEmpty<T>>
    for NonEmptyHandles<'program, 'scope, T>
where
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<NonEmpty<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

/// Author-side handles paired with stable domain keys.
#[derive(Debug, PartialEq, Eq)]
pub struct DomainKeyedHandles<'program, 'scope, K: StableDomainKey, T: MfmValue> {
    entries: Vec<DomainKeyedHandle<'program, 'scope, K, T>>,
}

/// Author-side non-empty handles paired with stable domain keys.
#[derive(Debug, PartialEq, Eq)]
pub struct DomainKeyedNonEmptyHandles<'program, 'scope, K: StableDomainKey, T: MfmValue> {
    entries: Vec<DomainKeyedHandle<'program, 'scope, K, T>>,
}

#[derive(Debug, PartialEq, Eq)]
struct DomainKeyedHandle<'program, 'scope, K: StableDomainKey, T: MfmValue> {
    key_ref: StableDomainKeyRef,
    handle: Handle<'program, 'scope, T>,
    _key: PhantomData<fn(K) -> K>,
}

impl<'program, 'scope, K, T> DomainKeyedHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    /// Creates domain-keyed handles, rejecting duplicate stable references and sorting by them.
    pub fn new(entries: Vec<(K, Handle<'program, 'scope, T>)>) -> Result<Self> {
        Ok(Self {
            entries: Self::sorted_entries(entries)?,
        })
    }

    fn sorted_entries(
        entries: Vec<(K, Handle<'program, 'scope, T>)>,
    ) -> Result<Vec<DomainKeyedHandle<'program, 'scope, K, T>>> {
        let mut keyed = entries
            .into_iter()
            .map(|(key, handle)| {
                let key_ref = StableDomainKeyRef::from_key(&key)?;
                Ok(DomainKeyedHandle {
                    key_ref,
                    handle,
                    _key: PhantomData,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        keyed.sort_by(|left, right| left.key_ref.cmp(&right.key_ref));
        let mut seen = BTreeSet::new();
        for entry in &keyed {
            if !seen.insert(entry.key_ref.clone()) {
                return Err(PlanError::DuplicateDomainKey(
                    entry.key_ref.stable_sort_key(),
                ));
            }
        }
        Ok(keyed)
    }

    /// Returns the number of keyed handles.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no keyed handles are present.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'program, 'scope, K, T> DomainKeyedNonEmptyHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    /// Creates non-empty domain-keyed handles, rejecting empty input and duplicate stable references.
    pub fn new(entries: Vec<(K, Handle<'program, 'scope, T>)>) -> Result<Self> {
        if entries.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        Ok(Self {
            entries: DomainKeyedHandles::<K, T>::sorted_entries(entries)?,
        })
    }

    /// Returns the number of keyed handles.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no keyed handles are present.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'program, 'scope, K, T> IntoInputBindingNode<Vec<T>>
    for DomainKeyedHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        let mut domain_keys = Vec::with_capacity(self.entries.len());
        let mut elements = Vec::with_capacity(self.entries.len());
        for (index, entry) in self.entries.into_iter().enumerate() {
            domain_keys.push(entry.key_ref);
            elements.push(
                entry
                    .handle
                    .into_binding_node(field_path.child(index.to_string())?)?,
            );
        }
        Ok(InputBindingNode::vector(
            elements,
            OrderingEvidence::StableDomainKey,
            domain_keys,
        ))
    }
}

impl<'program, 'scope, K, T> IntoStateInput<'program, 'scope, Vec<T>>
    for DomainKeyedHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<Vec<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

impl<'program, 'scope, K, T> IntoInputBindingNode<NonEmpty<T>>
    for DomainKeyedNonEmptyHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding_node(self, field_path: InputFieldPath) -> Result<InputBindingNode> {
        if self.entries.is_empty() {
            return Err(PlanError::EmptyNonEmptyInput);
        }
        let mut domain_keys = Vec::with_capacity(self.entries.len());
        let mut elements = Vec::with_capacity(self.entries.len());
        for (index, entry) in self.entries.into_iter().enumerate() {
            domain_keys.push(entry.key_ref);
            elements.push(
                entry
                    .handle
                    .into_binding_node(field_path.child(index.to_string())?)?,
            );
        }
        Ok(InputBindingNode::non_empty_vector(
            elements,
            OrderingEvidence::StableDomainKey,
            domain_keys,
        ))
    }
}

impl<'program, 'scope, K, T> IntoStateInput<'program, 'scope, NonEmpty<T>>
    for DomainKeyedNonEmptyHandles<'program, 'scope, K, T>
where
    K: StableDomainKey,
    T: MfmValue,
{
    fn into_binding(self) -> Result<InputBinding<NonEmpty<T>>> {
        InputBinding::from_root(self.into_binding_node(InputFieldPath::root())?)
    }
}

impl<'program, 'scope> OperationInput<'program, 'scope> for () {
    type Runtime = ();

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            InputBinding::<()>::from_root(InputBindingNode::Unit)?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope> for Handle<'program, 'scope, T>
where
    T: MfmValue,
{
    type Runtime = T;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope>
    for ForwardSideEffectHandle<'program, 'scope, T>
where
    T: MfmValue,
{
    type Runtime = T;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope> for Vec<Handle<'program, 'scope, T>>
where
    T: MfmValue,
{
    type Runtime = Vec<T>;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

impl<'program, 'scope, T> OperationInput<'program, 'scope> for NonEmptyHandles<'program, 'scope, T>
where
    T: MfmValue,
{
    type Runtime = NonEmpty<T>;

    fn input_binding(&self) -> Result<OperationInputBindingSpec> {
        Ok(OperationInputBindingSpec::from_state_input_spec(
            self.clone().into_binding()?.spec(),
        ))
    }
}

macro_rules! impl_tuple_operation_input {
    ($($name:ident),+ $(,)?) => {
        impl<'program, 'scope, $($name),+> OperationInput<'program, 'scope>
            for ($(Handle<'program, 'scope, $name>,)+)
        where
            $($name: MfmValue,)+
        {
            type Runtime = ($($name,)+);

            fn input_binding(&self) -> Result<OperationInputBindingSpec> {
                Ok(OperationInputBindingSpec::from_state_input_spec(
                    self.clone().into_binding()?.spec(),
                ))
            }
        }
    };
}

impl_tuple_operation_input!(A);
impl_tuple_operation_input!(A, B);
impl_tuple_operation_input!(A, B, C);
impl_tuple_operation_input!(A, B, C, D);
impl_tuple_operation_input!(A, B, C, D, E);
impl_tuple_operation_input!(A, B, C, D, E, F);
impl_tuple_operation_input!(A, B, C, D, E, F, G);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I, J);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I, J, K);
impl_tuple_operation_input!(A, B, C, D, E, F, G, H, I, J, K, L);
