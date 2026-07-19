use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeriveKind {
    Value,
    Config,
    StateInput,
    OperationOutput,
    PublicOutputs,
}

impl DeriveKind {
    pub(super) fn schema_kind_tokens(self) -> proc_macro2::TokenStream {
        match self {
            Self::Value => quote!(::mfm_values::SchemaKind::Value),
            Self::Config => quote!(::mfm_values::SchemaKind::PlanningConfig),
            Self::StateInput => quote!(::mfm_values::SchemaKind::StateInput),
            Self::OperationOutput => quote!(::mfm_values::SchemaKind::OperationOutput),
            Self::PublicOutputs => quote!(::mfm_values::SchemaKind::PublicOutput),
        }
    }

    pub(super) fn schema_method(self) -> &'static str {
        match self {
            Self::Value | Self::Config => "schema_descriptor",
            Self::StateInput => "input_schema_descriptor",
            Self::OperationOutput => "output_schema_descriptor",
            Self::PublicOutputs => "public_schema_descriptor",
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
) -> syn::Result<SchemaShapeOutput> {
    if attrs.transparent_string {
        return transparent_string_shape_tokens(data);
    }
    if attrs.transparent_map {
        return transparent_map_shape_tokens(data, kind);
    }

    match data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => {
            let field_output = field_descriptor_tokens(&fields.named, rename_all, kind)?;
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
        Data::Enum(data) => enum_shape_tokens(data, rename_all, kind, attrs),
        Data::Union(data) => Err(syn::Error::new(
            data.union_token.span,
            "MFM derives do not support unions",
        )),
    }
}

fn transparent_map_shape_tokens(data: &Data, kind: DeriveKind) -> syn::Result<SchemaShapeOutput> {
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
    let value_shape = shape_tokens(value, kind)?;

    Ok(SchemaShapeOutput {
        shape: quote!(::mfm_values::SchemaShape::BTreeMapString {
            value: Box::new(#value_shape)
        }),
        default_bounds: Vec::new(),
    })
}

fn transparent_string_shape_tokens(data: &Data) -> syn::Result<SchemaShapeOutput> {
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
        shape: quote!(::mfm_values::SchemaShape::String),
        default_bounds: Vec::new(),
    })
}

fn enum_shape_tokens(
    data: &DataEnum,
    rename_all: Option<&str>,
    kind: DeriveKind,
    attrs: &ContainerAttrs,
) -> syn::Result<SchemaShapeOutput> {
    if kind == DeriveKind::StateInput {
        return Err(syn::Error::new(
            data.enum_token.span,
            "StateInput derive supports named structs only in v1",
        ));
    }

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
        let shape_output = variant_shape_tokens(variant, rename_all, kind)?;
        default_bounds.extend(shape_output.default_bounds);
        let shape = shape_output.shape;
        variants.push(quote!(::mfm_values::EnumVariantDescriptor::new(#wire_name, #shape)));
    }

    let tagging = match (attrs.enum_tag.as_deref(), attrs.enum_content.as_deref()) {
        (None, None) => quote!(::mfm_values::EnumTagging::External),
        (Some(tag), None) => quote!(::mfm_values::EnumTagging::Internal { tag: #tag }),
        (Some(tag), Some(content)) => {
            quote!(::mfm_values::EnumTagging::Adjacent { tag: #tag, content: #content })
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
    rename_all: Option<&str>,
    kind: DeriveKind,
) -> syn::Result<SchemaShapeOutput> {
    match &variant.fields {
        Fields::Unit => Ok(SchemaShapeOutput {
            shape: quote!(::mfm_values::SchemaShape::Unit),
            default_bounds: Vec::new(),
        }),
        Fields::Named(FieldsNamed { named, .. }) => {
            let field_output = field_descriptor_tokens(named, rename_all, kind)?;
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
                .map(|field| shape_tokens(&field.ty, kind))
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
        let shape = shape_tokens(&field.ty, kind)?;
        let constructor = if attrs.default {
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

fn shape_tokens(ty: &Type, kind: DeriveKind) -> syn::Result<proc_macro2::TokenStream> {
    reject_known_secret_type(ty)?;
    match ty {
        Type::Path(type_path) => shape_tokens_for_path(type_path, kind),
        Type::Tuple(tuple) => {
            let elements = tuple
                .elems
                .iter()
                .map(|element| shape_tokens(element, kind))
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
) -> syn::Result<proc_macro2::TokenStream> {
    let Some(segment) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            type_path,
            "unsupported empty type path",
        ));
    };
    let ident = segment.ident.to_string();

    match ident.as_str() {
        "bool" => Ok(quote!(::mfm_values::SchemaShape::Bool)),
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
        "Option" => {
            let element = one_generic_type(segment, "Option")?;
            let shape = shape_tokens(element, kind)?;
            Ok(quote!(::mfm_values::SchemaShape::Option(Box::new(#shape))))
        }
        "Vec" => {
            let element = one_generic_type(segment, "Vec")?;
            let shape = shape_tokens(element, kind)?;
            Ok(quote!(::mfm_values::SchemaShape::Vec(Box::new(#shape))))
        }
        "NonEmpty" => {
            let element = one_generic_type(segment, "NonEmpty")?;
            let shape = shape_tokens(element, kind)?;
            Ok(quote!(::mfm_values::SchemaShape::NonEmptyVec(Box::new(#shape))))
        }
        "BTreeMap" => {
            let (key, value) = two_generic_types(segment, "BTreeMap")?;
            if !is_string_type(key) {
                return Err(syn::Error::new_spanned(
                    key,
                    "BTreeMap keys must be String for MFM descriptors",
                ));
            }
            let value_shape = shape_tokens(value, kind)?;
            Ok(quote!(::mfm_values::SchemaShape::BTreeMapString {
                value: Box::new(#value_shape)
            }))
        }
        _ if kind == DeriveKind::StateInput && ident.ends_with("Input") => {
            let ty = quote!(#type_path);
            Ok(quote!({
                let descriptor = <#ty as ::mfm_values::StateInput>::input_schema_descriptor()?;
                descriptor.identity.shape
            }))
        }
        _ => {
            let ty = quote!(#type_path);
            Ok(quote!(::mfm_values::SchemaShape::ValueRef {
                schema_id: <#ty as ::mfm_values::MfmValue>::schema_id()?,
                semantic_type_id: <#ty as ::mfm_values::MfmValue>::semantic_id()?,
            }))
        }
    }
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
