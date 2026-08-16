use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeriveKind {
    Value,
    PersistedContract,
}

impl DeriveKind {
    pub(super) fn schema_kind_tokens(self) -> proc_macro2::TokenStream {
        match self {
            Self::Value => quote!(::mfm_values::SchemaKind::Value),
            Self::PersistedContract => quote!(::mfm_values::SchemaKind::PersistedContract),
        }
    }
}

struct FieldDescriptorOutput {
    descriptors: Vec<proc_macro2::TokenStream>,
    default_bounds: Vec<proc_macro2::TokenStream>,
}

pub(super) struct SchemaShapeOutput {
    pub(super) shape: proc_macro2::TokenStream,
    pub(super) default_bounds: Vec<proc_macro2::TokenStream>,
}

pub(super) fn schema_shape_tokens(
    data: &Data,
    rename_all: Option<&str>,
    kind: DeriveKind,
    attrs: &ContainerAttrs,
    generic_params: &[Ident],
) -> syn::Result<SchemaShapeOutput> {
    if attrs.unsigned_minimum.is_some()
        && (kind != DeriveKind::PersistedContract || !attrs.serde_transparent)
    {
        return Err(syn::Error::new(
            Span::call_site(),
            "unsigned bounds require PersistedSchema on a serde-transparent newtype",
        ));
    }
    if attrs.transparent_string {
        return transparent_string_shape_tokens(data, kind, generic_params);
    }
    if attrs.transparent_map {
        return transparent_map_shape_tokens(data, kind, generic_params);
    }
    if kind == DeriveKind::PersistedContract && attrs.serde_transparent {
        return transparent_newtype_shape_tokens(data, kind, attrs, generic_params);
    }

    match data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => {
            let field_output =
                field_descriptor_tokens(&fields.named, rename_all, kind, generic_params)?;
            let descriptors = field_output.descriptors;
            Ok(SchemaShapeOutput {
                shape: quote!(::mfm_values::SchemaShape::named_struct(
                    vec![#(#descriptors),*]
                )?),
                default_bounds: field_output.default_bounds,
            })
        }
        Data::Struct(other) => Err(syn::Error::new(
            other.fields.span(),
            "MFM derives support named structs only in v1",
        )),
        Data::Enum(data) => enum_shape_tokens(data, rename_all, kind, attrs, generic_params),
        Data::Union(data) => Err(syn::Error::new(
            data.union_token.span,
            "MFM derives do not support unions",
        )),
    }
}

/// Builds the shape of a `#[serde(transparent)]` newtype from its one field.
///
/// The retained bytes are exactly the inner value's bytes, so the contract's
/// shape is the inner shape rather than a wrapper object.
fn transparent_newtype_shape_tokens(
    data: &Data,
    kind: DeriveKind,
    attrs: &ContainerAttrs,
    generic_params: &[Ident],
) -> syn::Result<SchemaShapeOutput> {
    let Data::Struct(DataStruct { fields, .. }) = data else {
        return Err(syn::Error::new(
            Span::call_site(),
            "serde(transparent) requires a one-field struct",
        ));
    };
    let mut iter = fields.iter();
    let (Some(field), None) = (iter.next(), iter.next()) else {
        return Err(syn::Error::new(
            fields.span(),
            "serde(transparent) requires exactly one field",
        ));
    };
    let shape = if let Some((minimum, maximum)) = attrs.unsigned_minimum.zip(attrs.unsigned_maximum)
    {
        let Type::Path(path) = &field.ty else {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "unsigned bounds require an unsigned integer newtype",
            ));
        };
        let supported = path.path.segments.last().is_some_and(|segment| {
            matches!(
                segment.ident.to_string().as_str(),
                "u8" | "u16" | "u32" | "u64"
            )
        });
        if !supported {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "unsigned bounds require an unsigned integer newtype",
            ));
        }
        quote!(::mfm_values::SchemaShape::UnsignedRange {
            minimum: #minimum,
            maximum: #maximum,
        })
    } else {
        shape_tokens(&field.ty, kind, generic_params)?
    };
    Ok(SchemaShapeOutput {
        shape,
        default_bounds: Vec::new(),
    })
}

fn transparent_map_shape_tokens(
    data: &Data,
    kind: DeriveKind,
    generic_params: &[Ident],
) -> syn::Result<SchemaShapeOutput> {
    let Data::Struct(DataStruct {
        fields: Fields::Named(fields),
        ..
    }) = data
    else {
        return Err(syn::Error::new(
            Span::call_site(),
            "mfm(transparent_map) requires a named struct",
        ));
    };

    if fields.named.len() != 1 {
        return Err(syn::Error::new(
            fields.span(),
            "mfm(transparent_map) requires exactly one BTreeMap field",
        ));
    }

    let field = fields
        .named
        .first()
        .expect("field count checked before access");
    let Type::Path(type_path) = &field.ty else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "mfm(transparent_map) field must be BTreeMap<String, V>",
        ));
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "unsupported empty type path",
        ));
    };
    if segment.ident != "BTreeMap" {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "mfm(transparent_map) field must be BTreeMap<String, V>",
        ));
    }
    let (key, value) = two_generic_types(segment, "BTreeMap")?;
    if !is_string_type(key) {
        return Err(syn::Error::new_spanned(
            key,
            "mfm(transparent_map) BTreeMap keys must be String",
        ));
    }
    let value_shape = shape_tokens(value, kind, generic_params)?;

    Ok(SchemaShapeOutput {
        shape: if kind == DeriveKind::PersistedContract {
            quote!(::mfm_values::SchemaShape::BoundedStringMap {
                key_grammar: ::mfm_values::StringGrammar::UnicodeScalarText,
                key_minimum_bytes: 0,
                key_maximum_bytes: ::mfm_values::MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES as u32,
                value: Box::new(#value_shape),
                minimum_entries: 0,
                maximum_entries: ::mfm_values::MAX_OBJECT_ENTRIES as u32,
            })
        } else {
            quote!(::mfm_values::SchemaShape::BTreeMapString {
                value: Box::new(#value_shape)
            })
        },
        default_bounds: Vec::new(),
    })
}

fn transparent_string_shape_tokens(
    data: &Data,
    kind: DeriveKind,
    _generic_params: &[Ident],
) -> syn::Result<SchemaShapeOutput> {
    let Data::Struct(DataStruct {
        fields: Fields::Named(fields),
        ..
    }) = data
    else {
        return Err(syn::Error::new(
            Span::call_site(),
            "mfm(transparent_string) requires a named struct",
        ));
    };

    if fields.named.len() != 1 {
        return Err(syn::Error::new(
            fields.span(),
            "mfm(transparent_string) requires exactly one String field",
        ));
    }

    let field = fields
        .named
        .first()
        .expect("field count checked before access");
    if !is_string_type(&field.ty) {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "mfm(transparent_string) field must be String",
        ));
    }

    Ok(SchemaShapeOutput {
        shape: if kind == DeriveKind::PersistedContract {
            bounded_persisted_string()
        } else {
            quote!(::mfm_values::SchemaShape::String)
        },
        default_bounds: Vec::new(),
    })
}

fn enum_shape_tokens(
    data: &DataEnum,
    rename_all: Option<&str>,
    kind: DeriveKind,
    attrs: &ContainerAttrs,
    generic_params: &[Ident],
) -> syn::Result<SchemaShapeOutput> {
    let mut variants = Vec::new();
    let mut default_bounds = Vec::new();
    let mut variant_names = Vec::new();
    let internal_tagged = attrs.enum_tag.is_some() && attrs.enum_content.is_none();
    for variant in &data.variants {
        let variant_attrs = VariantAttrs::parse(&variant.attrs)?;
        let wire_name = variant_attrs
            .rename
            .unwrap_or_else(|| apply_rename_all(&variant.ident.to_string(), rename_all));
        if variant_names.iter().any(|name: &String| name == &wire_name) {
            return Err(syn::Error::new(
                variant.ident.span(),
                format!("duplicate MFM enum variant wire name '{wire_name}'"),
            ));
        }
        if internal_tagged && matches!(variant.fields, Fields::Unnamed(_)) {
            return Err(syn::Error::new(
                variant.ident.span(),
                "internally tagged MFM enum variants must be unit or named-field variants",
            ));
        }
        variant_names.push(wire_name.clone());
        let shape_output = variant_shape_tokens(variant, kind, generic_params)?;
        default_bounds.extend(shape_output.default_bounds);
        let shape = shape_output.shape;
        variants.push(quote!(::mfm_values::EnumVariantDescriptor::new(#wire_name, #shape)));
    }

    let tagging = match (attrs.enum_tag.as_deref(), attrs.enum_content.as_deref()) {
        (None, None) => quote!(::mfm_values::EnumTagging::External),
        (Some(tag), None) => {
            quote!(::mfm_values::EnumTagging::Internal { tag: #tag.to_owned() })
        }
        (Some(tag), Some(content)) => {
            quote!(::mfm_values::EnumTagging::Adjacent {
                tag: #tag.to_owned(),
                content: #content.to_owned(),
            })
        }
        (None, Some(_)) => {
            return Err(syn::Error::new(
                data.enum_token.span,
                "serde(content) requires serde(tag) for MFM enum derives",
            ));
        }
    };

    Ok(SchemaShapeOutput {
        shape: quote!(::mfm_values::SchemaShape::tagged_enum(#tagging, vec![#(#variants),*])?),
        default_bounds,
    })
}

fn variant_shape_tokens(
    variant: &Variant,
    kind: DeriveKind,
    generic_params: &[Ident],
) -> syn::Result<SchemaShapeOutput> {
    match &variant.fields {
        Fields::Unit => Ok(SchemaShapeOutput {
            shape: quote!(::mfm_values::SchemaShape::Unit),
            default_bounds: Vec::new(),
        }),
        Fields::Named(FieldsNamed { named, .. }) => {
            let field_output = field_descriptor_tokens(named, None, kind, generic_params)?;
            let descriptors = field_output.descriptors;
            Ok(SchemaShapeOutput {
                shape: quote!(::mfm_values::SchemaShape::named_struct(
                    vec![#(#descriptors),*]
                )?),
                default_bounds: field_output.default_bounds,
            })
        }
        Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => {
            let shapes = unnamed
                .iter()
                .map(|field| shape_tokens(&field.ty, kind, generic_params))
                .collect::<syn::Result<Vec<_>>>()?;
            Ok(SchemaShapeOutput {
                shape: quote!(::mfm_values::SchemaShape::Tuple(vec![#(#shapes),*])),
                default_bounds: Vec::new(),
            })
        }
    }
}

fn field_descriptor_tokens(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    rename_all: Option<&str>,
    kind: DeriveKind,
    generic_params: &[Ident],
) -> syn::Result<FieldDescriptorOutput> {
    let mut output = Vec::new();
    let mut default_bounds = Vec::new();
    let mut names = Vec::new();

    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "MFM derives require named fields"))?;
        let attrs = FieldAttrs::parse(&field.attrs)?;
        let wire_name = attrs
            .rename
            .unwrap_or_else(|| apply_rename_all(&ident.to_string(), rename_all));
        if names.iter().any(|name: &String| name == &wire_name) {
            return Err(syn::Error::new(
                ident.span(),
                format!("duplicate MFM field wire name '{wire_name}'"),
            ));
        }
        names.push(wire_name.clone());
        let shape_kind = if attrs.persisted {
            DeriveKind::PersistedContract
        } else {
            kind
        };
        let mut shape = if attrs.optional_absent {
            let Type::Path(path) = &field.ty else {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "Option::is_none requires an Option field",
                ));
            };
            let segment = path
                .path
                .segments
                .last()
                .filter(|segment| segment.ident == "Option")
                .ok_or_else(|| {
                    syn::Error::new_spanned(&field.ty, "Option::is_none requires an Option field")
                })?;
            shape_tokens(
                one_generic_type(segment, "Option")?,
                shape_kind,
                generic_params,
            )?
        } else {
            shape_tokens(&field.ty, shape_kind, generic_params)?
        };
        if let Some(literal) = attrs.literal {
            if kind != DeriveKind::PersistedContract || !is_string_type(&field.ty) {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "mfm(literal) requires a String field on PersistedSchema",
                ));
            }
            shape = quote!(::mfm_values::SchemaShape::Literal(
                ::mfm_values::LiteralValue::String(#literal.to_owned())
            ));
        }
        if attrs.minimum_items.is_some() || attrs.maximum_items.is_some() {
            let minimum_items = attrs
                .minimum_items
                .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
            let maximum_items = attrs
                .maximum_items
                .map_or_else(|| quote!(None), |value| quote!(Some(#value)));
            shape = quote!({
                let shape = #shape;
                match shape {
                    ::mfm_values::SchemaShape::BoundedSequence {
                        element,
                        minimum_items,
                        maximum_items,
                        ordering,
                        unique,
                    } => ::mfm_values::SchemaShape::BoundedSequence {
                        element,
                        minimum_items: #minimum_items.unwrap_or(minimum_items),
                        maximum_items: #maximum_items.unwrap_or(maximum_items),
                        ordering,
                        unique,
                    },
                    _ => return Err(::mfm_values::ValueError::Descriptor(
                        "sequence bounds require a sequence field".to_owned(),
                    )),
                }
            });
        }
        let constructor = if attrs.optional_absent {
            quote!(::mfm_values::FieldDescriptor::optional_absent)
        } else if attrs.default {
            let ty = &field.ty;
            default_bounds.push(quote! {
                let _ = || {
                    fn assert_mfm_default<T: ::mfm_values::MfmDefault>() {}
                    assert_mfm_default::<#ty>();
                };
            });
            quote!(::mfm_values::FieldDescriptor::with_default)
        } else {
            quote!(::mfm_values::FieldDescriptor::required)
        };
        output.push(quote!(#constructor(#wire_name, #shape)));
    }

    Ok(FieldDescriptorOutput {
        descriptors: output,
        default_bounds,
    })
}

fn shape_tokens(
    ty: &Type,
    kind: DeriveKind,
    generic_params: &[Ident],
) -> syn::Result<proc_macro2::TokenStream> {
    reject_known_secret_type(ty)?;
    match ty {
        Type::Path(type_path) => shape_tokens_for_path(type_path, kind, generic_params),
        Type::Tuple(tuple) => {
            if tuple.elems.is_empty() {
                return Ok(quote!(::mfm_values::SchemaShape::Unit));
            }
            let elements = tuple
                .elems
                .iter()
                .map(|element| shape_tokens(element, kind, generic_params))
                .collect::<syn::Result<Vec<_>>>()?;
            Ok(quote!(::mfm_values::SchemaShape::Tuple(
                vec![#(#elements),*]
            )))
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "unsupported field type for MFM derive",
        )),
    }
}

fn shape_tokens_for_path(
    type_path: &TypePath,
    kind: DeriveKind,
    generic_params: &[Ident],
) -> syn::Result<proc_macro2::TokenStream> {
    let Some(segment) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            type_path,
            "unsupported empty type path",
        ));
    };
    let ident = segment.ident.to_string();

    if type_path.path.segments.len() == 1
        && generic_params
            .iter()
            .any(|parameter| parameter == &segment.ident)
    {
        return Ok(quote!({
            let descriptor = <#type_path as ::mfm_values::MfmValue>::schema_descriptor()?;
            ::mfm_values::SchemaShape::Generic {
                constructor: "mfm/generic-value".to_owned(),
                arguments: vec![
                    ::mfm_values::GenericArgumentDescriptor::for_value::<#type_path>()?
                ],
                serialized_shape: Box::new(
                    descriptor.identity().canonical_json_shape()?.clone()
                ),
            }
        }));
    }

    if let Some(shape) = checked_identity_shape(&ident) {
        return Ok(shape);
    }

    match ident.as_str() {
        "bool" => Ok(quote!(::mfm_values::SchemaShape::Bool)),
        "String" if kind == DeriveKind::PersistedContract => Ok(bounded_persisted_string()),
        "String" => Ok(quote!(::mfm_values::SchemaShape::String)),
        "i8" => Ok(signed_integer(8)),
        "i16" => Ok(signed_integer(16)),
        "i32" => Ok(signed_integer(32)),
        "i64" => Ok(signed_integer(64)),
        "u8" => Ok(unsigned_integer(8)),
        "u16" => Ok(unsigned_integer(16)),
        "u32" => Ok(unsigned_integer(32)),
        "u64" => Ok(unsigned_integer(64)),
        "NonZeroU64" => Ok(unsigned_integer(64)),
        "NonZeroU16" => Ok(quote!(::mfm_values::SchemaShape::UnsignedRange {
            minimum: 1,
            maximum: u64::from(u16::MAX)
        })),
        "NonZeroU32" => Ok(quote!(::mfm_values::SchemaShape::UnsignedRange {
            minimum: 1,
            maximum: u64::from(u32::MAX)
        })),
        "f32" | "f64" => Err(syn::Error::new_spanned(
            type_path,
            "floating point fields are not supported by MFM persisted surfaces",
        )),
        "usize" | "isize" => Err(syn::Error::new_spanned(
            type_path,
            "usize/isize fields are not supported by MFM persisted surfaces",
        )),
        "HashMap" => Err(syn::Error::new_spanned(
            type_path,
            "HashMap is not supported; use BTreeMap<String, V>",
        )),
        "Value"
            if path_contains(&type_path.path, "serde_json")
                || type_path.path.segments.len() == 1 =>
        {
            Err(syn::Error::new_spanned(
                type_path,
                "serde_json::Value is not supported by MFM persisted surfaces",
            ))
        }
        "Box" => {
            let element = one_generic_type(segment, "Box")?;
            shape_tokens(element, kind, generic_params)
        }
        "Option" => {
            let element = one_generic_type(segment, "Option")?;
            let shape = shape_tokens(element, kind, generic_params)?;
            Ok(quote!(::mfm_values::SchemaShape::Option(Box::new(#shape))))
        }
        "Vec" => {
            let element = one_generic_type(segment, "Vec")?;
            let shape = shape_tokens(element, kind, generic_params)?;
            if kind == DeriveKind::PersistedContract {
                Ok(quote!(::mfm_values::SchemaShape::BoundedSequence {
                    element: Box::new(#shape),
                    minimum_items: 0,
                    maximum_items: ::mfm_values::MAX_ARRAY_ITEMS as u32,
                    ordering: ::mfm_values::SequenceOrdering::Preserved,
                    unique: false,
                }))
            } else {
                Ok(quote!(::mfm_values::SchemaShape::Vec(Box::new(#shape))))
            }
        }
        "NonEmpty" => {
            let element = one_generic_type(segment, "NonEmpty")?;
            let shape = shape_tokens(element, kind, generic_params)?;
            if kind == DeriveKind::PersistedContract {
                Ok(quote!(::mfm_values::SchemaShape::BoundedSequence {
                    element: Box::new(#shape),
                    minimum_items: 1,
                    maximum_items: ::mfm_values::MAX_ARRAY_ITEMS as u32,
                    ordering: ::mfm_values::SequenceOrdering::Preserved,
                    unique: false,
                }))
            } else {
                Ok(quote!(::mfm_values::SchemaShape::NonEmptyVec(Box::new(#shape))))
            }
        }
        "BTreeMap" => {
            let (key, value) = two_generic_types(segment, "BTreeMap")?;
            if !is_string_type(key) {
                return Err(syn::Error::new_spanned(
                    key,
                    "BTreeMap keys must be String for MFM descriptors",
                ));
            }
            let value_shape = shape_tokens(value, kind, generic_params)?;
            if kind == DeriveKind::PersistedContract {
                Ok(quote!(::mfm_values::SchemaShape::BoundedStringMap {
                    key_grammar: ::mfm_values::StringGrammar::UnicodeScalarText,
                    key_minimum_bytes: 0,
                    key_maximum_bytes: ::mfm_values::MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES as u32,
                    value: Box::new(#value_shape),
                    minimum_entries: 0,
                    maximum_entries: ::mfm_values::MAX_OBJECT_ENTRIES as u32,
                }))
            } else {
                Ok(quote!(::mfm_values::SchemaShape::BTreeMapString {
                    value: Box::new(#value_shape)
                }))
            }
        }
        _ if kind == DeriveKind::PersistedContract => {
            // A nested persisted owner declares its own shape; the outer
            // contract embeds it so one type never restates another's fields.
            let ty = quote!(#type_path);
            Ok(quote!(
                <#ty as ::mfm_values::PersistedSchema>::schema_shape()?
            ))
        }
        _ => {
            let ty = quote!(#type_path);
            Ok(quote!(::mfm_values::SchemaShape::inline_value::<#ty>()?))
        }
    }
}

fn bounded_persisted_string() -> proc_macro2::TokenStream {
    quote!(::mfm_values::SchemaShape::BoundedString {
        minimum_bytes: 0,
        maximum_bytes: ::mfm_values::MAX_STRING_UTF8_BYTES as u32,
        grammar: ::mfm_values::StringGrammar::UnicodeScalarText,
    })
}

fn signed_integer(bits: u16) -> proc_macro2::TokenStream {
    quote!(::mfm_values::SchemaShape::SignedInteger { bits: #bits })
}

fn unsigned_integer(bits: u16) -> proc_macro2::TokenStream {
    quote!(::mfm_values::SchemaShape::UnsignedInteger { bits: #bits })
}

fn is_string_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "String"))
}

/// Maps a checked identity type to its bounded-string shape and grammar.
///
/// The grammar is enforced by the checked Rust owner; the descriptor only names
/// it, so a persisted contract never restates an identity's regular expression.
fn checked_identity_shape(ident: &str) -> Option<proc_macro2::TokenStream> {
    let grammar_and_bound = match ident {
        "ContentRef" => {
            return Some(quote!(::mfm_values::SchemaShape::content_ref()?));
        }
        "ContentDigest" => (quote!(ContentDigest), 128_u32),
        "SchemaId" => (quote!(SchemaId), 512),
        "SemanticTypeId" => (quote!(SemanticTypeId), 512),
        "RunId" => (quote!(RunId), 128),
        "ArtifactId" => (quote!(ArtifactId), 128),
        "StableId" => (quote!(StableId), 512),
        "EntryPointId" => (quote!(EntryPointId), 512),
        _ => return None,
    };
    let (grammar, maximum_bytes) = grammar_and_bound;
    Some(quote!(::mfm_values::SchemaShape::identity_string(
        ::mfm_values::StringGrammar::#grammar,
        #maximum_bytes
    )))
}
