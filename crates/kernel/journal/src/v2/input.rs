use mfm_canonical::CanonicalValue;
use mfm_ids::{FieldPath, RunId, SchemaId};

use super::codec::{
    array_field, cv_array, cv_string, define_schema_value, field_path_field, nullable_field,
    object, required_field, run_id_field, schema_id_field, tagged_object,
};
use super::{
    CrossRunSourceRef, FactRef, OutputRef, RecordRef, Result, RetainedValueContract, TransitionRef,
    ValueRef,
};

define_schema_value! {
    /// One exact named root in a config, seed, or context manifest.
    pub struct RootManifestEntry => "mfm.root-manifest-entry.v1";
    /// Complete canonically ordered immutable configuration roots.
    pub struct ConfigManifest => "mfm.config-manifest.v1";
    /// Complete canonically ordered immutable seed roots.
    pub struct SeedManifest => "mfm.seed-manifest.v1";
    /// Complete canonically ordered immutable context roots.
    pub struct ContextManifest => "mfm.context-manifest.v1";
    /// One exact named closed source-run root.
    pub struct CrossRunSourceManifestEntry => "mfm.cross-run-source-manifest-entry.v1";
    /// Complete canonically ordered closed source-run roots.
    pub struct CrossRunSourceManifest => "mfm.cross-run-source-manifest.v1";
    /// Closed source lineage for one materialized state input.
    pub struct InputSource => "mfm.input-source.v1";
    /// One typed state-input binding and its exact source lineage.
    pub struct InputBinding => "mfm.input-binding.v1";
    /// Complete immutable typed input tree consumed by one state callback.
    pub struct InputManifest => "mfm.input-manifest.v1";
}

const MAX_ADMISSION_MANIFEST_ENTRIES: usize = 4096;

impl RootManifestEntry {
    /// Constructs one exact named retained root.
    pub fn new(field_path: FieldPath, value_ref: ValueRef) -> Result<Self> {
        Self::from_canonical_value(object([
            ("field_path", cv_string(field_path.as_str())),
            ("value_ref", value_ref.canonical_value()?),
        ])?)
    }

    /// Returns the exact manifest field path.
    pub fn field_path(&self) -> Result<FieldPath> {
        field_path_field(self, "field_path")
    }

    /// Returns the full retained root authority.
    pub fn value_ref(&self) -> Result<ValueRef> {
        ValueRef::from_canonical_value(required_field(self, "value_ref")?)
    }
}

macro_rules! root_manifest {
    ($name:ident, $version:literal, $semantic_name:literal, $role:literal) => {
        impl $name {
            /// Returns the deterministic retained contract for this admission manifest.
            pub fn retained_contract() -> Result<RetainedValueContract> {
                super::runtime_retained_contract($version, $semantic_name, $role)
            }

            /// Constructs a canonically ordered, duplicate-free root manifest.
            pub fn new(entries: Vec<RootManifestEntry>) -> Result<Self> {
                let entries = canonical_manifest_entries(entries, RootManifestEntry::field_path)?;
                Self::from_canonical_value(object([
                    ("version", CanonicalValue::String($version.to_owned())),
                    (
                        "entries",
                        cv_array(entries.iter().map(|entry| entry.canonical_value()))?,
                    ),
                ])?)
            }

            /// Returns all exact roots in canonical field-path order.
            pub fn entries(&self) -> Result<Vec<RootManifestEntry>> {
                array_field(self, "entries")?
                    .into_iter()
                    .map(RootManifestEntry::from_canonical_value)
                    .collect()
            }
        }
    };
}

root_manifest!(
    ConfigManifest,
    "mfm.config-manifest.v1",
    "config-manifest",
    "mfm.admission.config-manifest"
);
root_manifest!(
    SeedManifest,
    "mfm.seed-manifest.v1",
    "seed-manifest",
    "mfm.admission.seed-manifest"
);
root_manifest!(
    ContextManifest,
    "mfm.context-manifest.v1",
    "context-manifest",
    "mfm.admission.context-manifest"
);

impl CrossRunSourceManifestEntry {
    /// Constructs one exact named closed source-run root.
    pub fn new(field_path: FieldPath, source: CrossRunSourceRef) -> Result<Self> {
        Self::from_canonical_value(object([
            ("field_path", cv_string(field_path.as_str())),
            ("source", source.canonical_value()?),
        ])?)
    }

    /// Returns the exact manifest field path.
    pub fn field_path(&self) -> Result<FieldPath> {
        field_path_field(self, "field_path")
    }

    /// Returns the complete closed source-run authority.
    pub fn source(&self) -> Result<CrossRunSourceRef> {
        CrossRunSourceRef::from_canonical_value(required_field(self, "source")?)
    }
}

impl CrossRunSourceManifest {
    /// Returns the deterministic retained contract for cross-run source admission.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        super::runtime_retained_contract(
            "mfm.cross-run-source-manifest.v1",
            "cross-run-source-manifest",
            "mfm.admission.cross-run-source-manifest",
        )
    }

    /// Constructs a canonically ordered, duplicate-free source-run manifest.
    pub fn new(entries: Vec<CrossRunSourceManifestEntry>) -> Result<Self> {
        let entries = canonical_manifest_entries(entries, CrossRunSourceManifestEntry::field_path)?;
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.cross-run-source-manifest.v1".to_owned()),
            ),
            (
                "entries",
                cv_array(entries.iter().map(|entry| entry.canonical_value()))?,
            ),
        ])?)
    }

    /// Returns all exact source roots in canonical field-path order.
    pub fn entries(&self) -> Result<Vec<CrossRunSourceManifestEntry>> {
        array_field(self, "entries")?
            .into_iter()
            .map(CrossRunSourceManifestEntry::from_canonical_value)
            .collect()
    }
}

fn canonical_manifest_entries<T>(
    entries: Vec<T>,
    field_path: fn(&T) -> Result<FieldPath>,
) -> Result<Vec<T>> {
    if entries.len() > MAX_ADMISSION_MANIFEST_ENTRIES {
        return Err(super::JournalError::Projection);
    }
    let mut entries = entries
        .into_iter()
        .map(|entry| Ok((field_path(&entry)?, entry)))
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(super::JournalError::Projection);
    }
    Ok(entries.into_iter().map(|(_, entry)| entry).collect())
}

/// Closed input-source variant and all of its typed fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSourceFields {
    /// One field retained by the immutable run admission.
    RunAdmission {
        /// Exact admission record.
        record_ref: RecordRef,
    },
    /// One output produced by a transition in this run.
    TransitionOutput {
        /// Explicit same-run binding.
        this_run_id: RunId,
        /// Producing transition.
        transition_ref: TransitionRef,
        /// Exact produced output.
        output_ref: OutputRef,
    },
    /// One fact emitted by a transition in this run.
    TransitionFact {
        /// Explicit same-run binding.
        this_run_id: RunId,
        /// Producing transition.
        transition_ref: TransitionRef,
        /// Exact emitted fact.
        fact_ref: FactRef,
    },
    /// One object authorized by a closed source run.
    CrossRun {
        /// Complete cross-run source authority.
        source_ref: CrossRunSourceRef,
    },
    /// One immutable admitted configuration value.
    Config {
        /// Full retained configuration authority.
        value_ref: ValueRef,
    },
    /// One exact registration-qualified support root.
    QualifiedSupport {
        /// Member coordinate within the admitted qualified support graph.
        member_path: FieldPath,
        /// Full retained qualified-support authority.
        value_ref: ValueRef,
    },
    /// One immutable admitted seed value.
    Seed {
        /// Full retained seed authority.
        value_ref: ValueRef,
    },
    /// One immutable admitted context value.
    Context {
        /// Full retained context authority.
        value_ref: ValueRef,
    },
}

impl InputSource {
    /// Constructs an input sourced from the exact immutable admission root.
    pub fn run_admission(record_ref: &RecordRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "run_admission",
            [("record_ref", record_ref.canonical_value()?)],
        )?)
    }

    /// Constructs an input sourced from one transition output in this run.
    pub fn transition_output(
        this_run_id: &RunId,
        transition_ref: &TransitionRef,
        output_ref: &OutputRef,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "transition_output",
            [
                ("this_run_id", cv_string(this_run_id.as_str())),
                ("transition_ref", transition_ref.canonical_value()?),
                ("output_ref", output_ref.canonical_value()?),
            ],
        )?)
    }

    /// Constructs an input sourced from one transition fact in this run.
    pub fn transition_fact(
        this_run_id: &RunId,
        transition_ref: &TransitionRef,
        fact_ref: &FactRef,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "transition_fact",
            [
                ("this_run_id", cv_string(this_run_id.as_str())),
                ("transition_ref", transition_ref.canonical_value()?),
                ("fact_ref", fact_ref.canonical_value()?),
            ],
        )?)
    }

    /// Constructs an input sourced through exact cross-run authority.
    pub fn cross_run(source_ref: &CrossRunSourceRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "cross_run",
            [("source_ref", source_ref.canonical_value()?)],
        )?)
    }

    /// Constructs an input sourced from the immutable admitted configuration root.
    pub fn config(value_ref: &ValueRef) -> Result<Self> {
        Self::retained_value("config", value_ref)
    }

    /// Constructs an input sourced from one exact qualified-support root.
    pub fn qualified_support(member_path: &FieldPath, value_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "qualified_support",
            [
                ("member_path", cv_string(member_path.as_str())),
                ("value_ref", value_ref.canonical_value()?),
            ],
        )?)
    }

    /// Constructs an input sourced from an immutable admitted seed root.
    pub fn seed(value_ref: &ValueRef) -> Result<Self> {
        Self::retained_value("seed", value_ref)
    }

    /// Constructs an input sourced from the immutable admitted context root.
    pub fn context(value_ref: &ValueRef) -> Result<Self> {
        Self::retained_value("context", value_ref)
    }

    fn retained_value(kind: &str, value_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            kind,
            [("value_ref", value_ref.canonical_value()?)],
        )?)
    }

    /// Projects the closed input-source variant and all of its typed fields.
    pub fn fields(&self) -> Result<InputSourceFields> {
        match super::codec::tag(self)?.as_str() {
            "run_admission" => Ok(InputSourceFields::RunAdmission {
                record_ref: RecordRef::from_canonical_value(required_field(self, "record_ref")?)?,
            }),
            "transition_output" => Ok(InputSourceFields::TransitionOutput {
                this_run_id: run_id_field(self, "this_run_id")?,
                transition_ref: TransitionRef::from_canonical_value(required_field(
                    self,
                    "transition_ref",
                )?)?,
                output_ref: OutputRef::from_canonical_value(required_field(self, "output_ref")?)?,
            }),
            "transition_fact" => Ok(InputSourceFields::TransitionFact {
                this_run_id: run_id_field(self, "this_run_id")?,
                transition_ref: TransitionRef::from_canonical_value(required_field(
                    self,
                    "transition_ref",
                )?)?,
                fact_ref: FactRef::from_canonical_value(required_field(self, "fact_ref")?)?,
            }),
            "cross_run" => Ok(InputSourceFields::CrossRun {
                source_ref: CrossRunSourceRef::from_canonical_value(required_field(
                    self,
                    "source_ref",
                )?)?,
            }),
            "config" => Ok(InputSourceFields::Config {
                value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            }),
            "qualified_support" => Ok(InputSourceFields::QualifiedSupport {
                member_path: field_path_field(self, "member_path")?,
                value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            }),
            "seed" => Ok(InputSourceFields::Seed {
                value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            }),
            "context" => Ok(InputSourceFields::Context {
                value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

impl InputManifest {
    /// Returns the deterministic retained contract for materialized state inputs.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        super::runtime_retained_contract(
            "mfm.input-manifest.v1",
            "input-manifest",
            "mfm.admission.input-manifest",
        )
    }
}

/// Typed fields of one state-input binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBindingFields {
    /// Exact destination within the state input tree.
    pub field_path: FieldPath,
    /// Closed source lineage.
    pub source: InputSource,
    /// Optional field selected from the exact source root.
    pub source_field_path: Option<FieldPath>,
    /// Full retained value authority.
    pub value_ref: ValueRef,
}

impl InputBinding {
    /// Constructs and validates one exact state-input binding.
    pub fn new(
        field_path: &FieldPath,
        source: &InputSource,
        source_field_path: Option<&FieldPath>,
        value_ref: &ValueRef,
    ) -> Result<Self> {
        let binding = Self::from_canonical_value(object([
            ("field_path", cv_string(field_path.as_str())),
            ("source", source.canonical_value()?),
            (
                "source_field_path",
                source_field_path
                    .map(|path| cv_string(path.as_str()))
                    .unwrap_or(CanonicalValue::Null),
            ),
            ("value_ref", value_ref.canonical_value()?),
        ])?)?;
        binding.fields()?;
        Ok(binding)
    }

    /// Projects every typed state-input binding field.
    pub fn fields(&self) -> Result<InputBindingFields> {
        let fields = InputBindingFields {
            field_path: field_path_field(self, "field_path")?,
            source: InputSource::from_canonical_value(required_field(self, "source")?)?,
            source_field_path: nullable_field(self, "source_field_path")?
                .map(|value| match value {
                    CanonicalValue::String(value) => FieldPath::new(value).map_err(Into::into),
                    _ => Err(super::JournalError::Projection),
                })
                .transpose()?,
            value_ref: ValueRef::from_canonical_value(required_field(self, "value_ref")?)?,
        };
        if matches!(
            fields.source.fields()?,
            InputSourceFields::TransitionFact { .. }
        ) && fields.source_field_path.is_some()
        {
            return Err(super::JournalError::Projection);
        }
        Ok(fields)
    }
}

/// Typed fields of one complete immutable input manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputManifestFields {
    /// Certified input-tree schema.
    pub input_schema_id: SchemaId,
    /// Store-minted root authority for the complete input tree.
    pub root_input_ref: ValueRef,
    /// Optional exact immutable configuration authority.
    pub config_ref: Option<ValueRef>,
    /// Optional exact immutable context authority.
    pub context_ref: Option<ValueRef>,
    /// Complete canonically ordered typed bindings.
    pub bindings: Vec<InputBinding>,
}

impl InputManifest {
    /// Constructs and validates one complete immutable input manifest.
    pub fn new(
        input_schema_id: &SchemaId,
        root_input_ref: &ValueRef,
        config_ref: Option<&ValueRef>,
        context_ref: Option<&ValueRef>,
        bindings: &[InputBinding],
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.input-manifest.v1".to_owned()),
            ),
            ("input_schema_id", cv_string(input_schema_id.as_str())),
            ("root_input_ref", root_input_ref.canonical_value()?),
            (
                "config_ref",
                config_ref
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
            (
                "context_ref",
                context_ref
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
            (
                "bindings",
                cv_array(bindings.iter().map(|binding| binding.canonical_value()))?,
            ),
        ])?)
    }

    /// Projects every typed input-manifest field.
    pub fn fields(&self) -> Result<InputManifestFields> {
        Ok(InputManifestFields {
            input_schema_id: schema_id_field(self, "input_schema_id")?,
            root_input_ref: ValueRef::from_canonical_value(required_field(
                self,
                "root_input_ref",
            )?)?,
            config_ref: nullable_field(self, "config_ref")?
                .map(ValueRef::from_canonical_value)
                .transpose()?,
            context_ref: nullable_field(self, "context_ref")?
                .map(ValueRef::from_canonical_value)
                .transpose()?,
            bindings: array_field(self, "bindings")?
                .into_iter()
                .map(InputBinding::from_canonical_value)
                .collect::<Result<_>>()?,
        })
    }
}
