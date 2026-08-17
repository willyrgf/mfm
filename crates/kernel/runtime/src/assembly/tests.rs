use mfm_ids::{DigestAlgorithm, DigestBytes, SemanticTypeId};
use mfm_program::{nominal_contract_ref, Declaration, Never, Program};
use mfm_program_derive::MfmValue;
use mfm_values::{
    framework_value_descriptor, EnumVariantDescriptor, GenericArgumentDescriptor, SchemaShape,
};
use serde::{Deserialize, Deserializer, Serialize};

use super::*;

#[derive(Debug, Serialize, MfmValue)]
struct AsymmetricPayload {
    value: String,
}

impl<'de> Deserialize<'de> for AsymmetricPayload {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            value: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            value: wire.value.to_lowercase(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum GenericExternal<T> {
    Selected(T),
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum GenericAdjacent<T> {
    Selected(T),
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum BoxedGenericExternal<T> {
    Selected(Box<T>),
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::redundant_allocation)] // Exercises wire-transparent nested Box association.
enum NestedBoxedGenericExternal<T> {
    Selected(Box<Box<T>>),
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum TwoGenericExternal<T> {
    First(T),
    Second(T),
}

#[derive(Debug, Serialize, Deserialize)]
struct ManualShapeSelector<const CASE: u8> {
    unused: bool,
}

impl<const CASE: u8> MfmValue for ManualShapeSelector<CASE> {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        let payload = AsymmetricPayload::schema_descriptor()?;
        let payload_schema = payload.schema_id()?;
        let payload_semantic = AsymmetricPayload::semantic_id()?;
        let payload_shape = payload.identity().canonical_json_shape()?.clone();
        let argument = GenericArgumentDescriptor {
            schema_id: payload_schema.clone(),
            semantic_type_id: payload_semantic.clone(),
        };
        let inline = SchemaShape::InlineValue {
            schema_id: payload_schema,
            semantic_type_id: payload_semantic,
            serialized_shape: Box::new(payload_shape.clone()),
        };
        let (tagging, variant_shape) = match CASE {
            0 => (
                EnumTagging::Internal {
                    tag: "kind".to_owned(),
                },
                SchemaShape::named_struct(vec![mfm_values::FieldDescriptor::required(
                    "value",
                    SchemaShape::String,
                )])?,
            ),
            1 => (EnumTagging::External, SchemaShape::Unit),
            2 => (
                EnumTagging::External,
                SchemaShape::named_struct(vec![mfm_values::FieldDescriptor::required(
                    "payload", inline,
                )])?,
            ),
            3 => (
                EnumTagging::External,
                SchemaShape::Tuple(vec![inline.clone(), inline]),
            ),
            4 => (
                EnumTagging::External,
                SchemaShape::Tuple(vec![SchemaShape::Generic {
                    constructor: "mfm/not-generic-value".to_owned(),
                    arguments: vec![argument],
                    serialized_shape: Box::new(payload_shape),
                }]),
            ),
            5 => (
                EnumTagging::External,
                SchemaShape::Tuple(vec![SchemaShape::Generic {
                    constructor: "mfm/generic-value".to_owned(),
                    arguments: vec![argument.clone(), argument],
                    serialized_shape: Box::new(payload_shape),
                }]),
            ),
            6 => {
                let wrong_semantic = SemanticTypeId::new(
                    "mfm.test",
                    "wrong-payload",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([6; 32]),
                )
                .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))?;
                let SchemaShape::InlineValue {
                    schema_id,
                    serialized_shape,
                    ..
                } = inline
                else {
                    unreachable!("inline payload")
                };
                (
                    EnumTagging::External,
                    SchemaShape::Tuple(vec![SchemaShape::InlineValue {
                        schema_id,
                        semantic_type_id: wrong_semantic,
                        serialized_shape,
                    }]),
                )
            }
            7 => {
                let SchemaShape::InlineValue {
                    schema_id,
                    semantic_type_id,
                    ..
                } = inline
                else {
                    unreachable!("inline payload")
                };
                (
                    EnumTagging::External,
                    SchemaShape::Tuple(vec![SchemaShape::InlineValue {
                        schema_id,
                        semantic_type_id,
                        serialized_shape: Box::new(SchemaShape::String),
                    }]),
                )
            }
            _ => unreachable!("unknown manual selector case"),
        };
        framework_value_descriptor(
            "mfm-runtime",
            Self::semantic_id()?,
            &format!("mfm.test.manual-shape-selector-{CASE}"),
            SchemaShape::Enum {
                tagging,
                variants: vec![EnumVariantDescriptor::new("selected", variant_shape)],
            },
            std::any::type_name::<Self>(),
        )
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.test",
            &format!("manual-shape-selector-{CASE}"),
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([CASE; 32]),
        )
        .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

fn association_result<S: MfmValue>(
    tags: &[&str],
    target_input: ContentRef,
) -> Result<MatchProjection> {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_value::<AsymmetricPayload>()
        .expect("payload codec");
    builder.register_value::<S>().expect("selector codec");
    let assembly = builder.finish().expect("assembly");
    let program = retained_match_program::<S>(tags, &target_input);
    let Declaration::Match(match_declaration) = &program.declarations()[0] else {
        panic!("root Match");
    };
    let selector_contract = nominal_contract_ref::<S>().expect("selector contract");
    associate_match(
        assembly.codec(&selector_contract).expect("selector codec"),
        match_declaration.variants(),
        program.declarations(),
        &assembly.inner,
    )
}

fn retained_match_program<S: MfmValue>(tags: &[&str], target_input: &ContentRef) -> Program {
    let selector = nominal_contract_ref::<S>().expect("selector contract");
    let never = nominal_contract_ref::<Never>().expect("Never contract");
    let variants = tags
        .iter()
        .enumerate()
        .map(|(index, tag)| {
            serde_json::json!({
                "tag": tag,
                "entry_index": index + 1,
            })
        })
        .collect::<Vec<_>>();
    let mut declarations = vec![serde_json::json!({
        "kind": "match",
        "value": {
            "selector_contract_ref": selector,
            "variants": variants,
        }
    })];
    declarations.extend(tags.iter().map(|_| {
        serde_json::json!({
            "kind": "state",
            "value": {
                "state_implementation_ref": target_input,
                "input_contract_ref": target_input,
                "output_contract_ref": target_input,
                "failure_contract_ref": never,
                "execution": { "kind": "pure" },
                "next_index": null,
                "failure_next_index": null,
            }
        })
    }));
    let wire = serde_json::json!({
        "entry_point_id": "mfm.test.runtime/retained-match@1",
        "admitted_context_contract_ref": selector,
        "root_success_contract_ref": target_input,
        "root_failure_contract_ref": never,
        "declarations": declarations,
    });
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&wire).expect("wire"),
    )
    .expect("canonical");
    Program::decode_canonical(canonical.as_bytes()).expect("retained Match Program")
}

fn assert_exact_projection<S: MfmValue>(selector: S) {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_value::<AsymmetricPayload>()
        .expect("payload codec");
    builder.register_value::<S>().expect("selector codec");
    let assembly = builder.finish().expect("assembly");

    let payload_contract = nominal_contract_ref::<AsymmetricPayload>().expect("payload contract");
    let selector_contract = nominal_contract_ref::<S>().expect("selector contract");
    let program = retained_match_program::<S>(&["selected"], &payload_contract);
    let Declaration::Match(match_declaration) = &program.declarations()[0] else {
        panic!("root Match");
    };
    let selector_codec = assembly.codec(&selector_contract).expect("selector codec");
    let projection = associate_match(
        selector_codec.clone(),
        match_declaration.variants(),
        program.declarations(),
        &assembly.inner,
    )
    .expect("projection");

    let hot_selector = qualify_hot(selector).expect("hot selector");
    let selector_ref = hot_selector.value_ref.clone();
    let selector_bytes = hot_selector.canonical.clone();
    let cold_selector = selector_codec
        .qualify(&selector_ref, selector_bytes.as_bytes())
        .expect("cold selector");

    let (hot_entry, hot_payload) = projection.project(hot_selector).expect("hot payload");
    let (cold_entry, cold_payload) = projection.project(cold_selector).expect("cold payload");

    assert_eq!(hot_entry, 1);
    assert_eq!(hot_entry, cold_entry);
    assert_eq!(hot_payload.contract_ref, cold_payload.contract_ref);
    assert_eq!(hot_payload.value_ref, cold_payload.value_ref);
    assert_eq!(hot_payload.canonical, cold_payload.canonical);
    assert_eq!(hot_payload.canonical.as_bytes(), br#"{"value":"MiXeD"}"#);
    assert_eq!(
        hot_payload
            .typed
            .downcast_ref::<AsymmetricPayload>()
            .expect("hot typed payload")
            .value,
        "mixed"
    );
    assert_eq!(
        cold_payload
            .typed
            .downcast_ref::<AsymmetricPayload>()
            .expect("cold typed payload")
            .value,
        "mixed"
    );
}

#[test]
fn generic_external_projection_preserves_exact_nested_bytes() {
    assert_exact_projection(GenericExternal::Selected(AsymmetricPayload {
        value: "MiXeD".to_owned(),
    }));
}

#[test]
fn generic_adjacent_projection_preserves_exact_nested_bytes() {
    assert_exact_projection(GenericAdjacent::Selected(AsymmetricPayload {
        value: "MiXeD".to_owned(),
    }));
}

#[test]
fn boxed_projection_preserves_exact_nested_bytes() {
    assert_exact_projection(BoxedGenericExternal::Selected(Box::new(
        AsymmetricPayload {
            value: "MiXeD".to_owned(),
        },
    )));
}

#[test]
fn nested_boxed_projection_preserves_exact_nested_bytes() {
    assert_exact_projection(NestedBoxedGenericExternal::Selected(Box::new(Box::new(
        AsymmetricPayload {
            value: "MiXeD".to_owned(),
        },
    ))));
}

#[test]
fn unsupported_match_shapes_fail_association() {
    let target = nominal_contract_ref::<AsymmetricPayload>().expect("target");
    for result in [
        association_result::<ManualShapeSelector<0>>(&["selected"], target.clone()),
        association_result::<ManualShapeSelector<1>>(&["selected"], target.clone()),
        association_result::<ManualShapeSelector<2>>(&["selected"], target.clone()),
        association_result::<ManualShapeSelector<3>>(&["selected"], target.clone()),
        association_result::<ManualShapeSelector<4>>(&["selected"], target.clone()),
        association_result::<ManualShapeSelector<5>>(&["selected"], target.clone()),
        association_result::<ManualShapeSelector<6>>(&["selected"], target.clone()),
        association_result::<ManualShapeSelector<7>>(&["selected"], target.clone()),
    ] {
        assert!(matches!(result, Err(RuntimeError::IncompatibleAssembly)));
    }
}

#[test]
fn match_tags_and_target_contract_must_be_exact_and_exhaustive() {
    let payload = nominal_contract_ref::<AsymmetricPayload>().expect("payload");
    assert!(matches!(
        association_result::<GenericExternal<AsymmetricPayload>>(&["unknown"], payload.clone(),),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert!(matches!(
        association_result::<TwoGenericExternal<AsymmetricPayload>>(&["first"], payload.clone(),),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert!(matches!(
        association_result::<GenericExternal<AsymmetricPayload>>(
            &["selected"],
            nominal_contract_ref::<GenericExternal<AsymmetricPayload>>().expect("wrong target"),
        ),
        Err(RuntimeError::IncompatibleAssembly)
    ));
}
