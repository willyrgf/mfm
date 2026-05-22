#![warn(missing_docs)]
//! Derive macros for MFM typed program contracts.
//!
//! This crate generates descriptor implementations for the typed kernel. It
//! does not generate runtime scheduling logic.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::parse_macro_input;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Fields, GenericArgument, Ident, LitStr,
    PathArguments, Type, TypePath,
};

#[proc_macro_derive(MfmValue, attributes(mfm, serde))]
/// Derives `mfm_values::MfmValue` for a named struct.
pub fn derive_mfm_value(input: TokenStream) -> TokenStream {
    expand_schema_derive(parse_macro_input!(input as DeriveInput), DeriveKind::Value).into()
}

#[proc_macro_derive(MfmConfig, attributes(mfm, serde))]
/// Derives `mfm_values::MfmConfig` for a named struct.
pub fn derive_mfm_config(input: TokenStream) -> TokenStream {
    expand_schema_derive(parse_macro_input!(input as DeriveInput), DeriveKind::Config).into()
}

#[proc_macro_derive(StateInput, attributes(mfm, serde))]
/// Derives `mfm_values::StateInput` for a named struct.
pub fn derive_state_input(input: TokenStream) -> TokenStream {
    expand_schema_derive(
        parse_macro_input!(input as DeriveInput),
        DeriveKind::StateInput,
    )
    .into()
}

#[proc_macro_derive(OperationOutput, attributes(mfm, serde))]
/// Derives `mfm_values::OperationOutput` for a named struct.
pub fn derive_operation_output(input: TokenStream) -> TokenStream {
    expand_schema_derive(
        parse_macro_input!(input as DeriveInput),
        DeriveKind::OperationOutput,
    )
    .into()
}

#[proc_macro_derive(PublicOutputs, attributes(mfm, serde))]
/// Derives `mfm_values::PublicOutputs` for a named struct.
pub fn derive_public_outputs(input: TokenStream) -> TokenStream {
    expand_schema_derive(
        parse_macro_input!(input as DeriveInput),
        DeriveKind::PublicOutputs,
    )
    .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeriveKind {
    Value,
    Config,
    StateInput,
    OperationOutput,
    PublicOutputs,
}

impl DeriveKind {
    fn schema_kind_tokens(self) -> proc_macro2::TokenStream {
        match self {
            Self::Value => quote!(::mfm_values::SchemaKind::Value),
            Self::Config => quote!(::mfm_values::SchemaKind::PlanningConfig),
            Self::StateInput => quote!(::mfm_values::SchemaKind::StateInput),
            Self::OperationOutput => quote!(::mfm_values::SchemaKind::OperationOutput),
            Self::PublicOutputs => quote!(::mfm_values::SchemaKind::PublicOutput),
        }
    }

    fn schema_method(self) -> &'static str {
        match self {
            Self::Value | Self::Config => "schema_descriptor",
            Self::StateInput => "input_schema_descriptor",
            Self::OperationOutput => "output_schema_descriptor",
            Self::PublicOutputs => "public_schema_descriptor",
        }
    }
}

fn expand_schema_derive(input: DeriveInput, kind: DeriveKind) -> proc_macro2::TokenStream {
    match expand_schema_derive_result(input, kind) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn expand_schema_derive_result(
    input: DeriveInput,
    kind: DeriveKind,
) -> syn::Result<proc_macro2::TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "MFM derives do not support generic structs in v1",
        ));
    }

    let attrs = ContainerAttrs::parse(&input.attrs, &input.ident)?;
    let fields = named_struct_fields(&input.data)?;
    let field_output = field_descriptor_tokens(fields, attrs.rename_all.as_deref())?;
    let field_descriptors = field_output.descriptors;
    let default_bounds = field_output.default_bounds;
    let ident = &input.ident;
    let schema_name = attrs.schema_name;
    let version = attrs.version;
    let schema_kind = kind.schema_kind_tokens();
    let schema_method = format_ident!("{}", kind.schema_method());
    let derive_macro_version = concat!("mfm-program-derive/", env!("CARGO_PKG_VERSION"));
    let audit_path = quote! {
        ::mfm_values::SchemaAudit::__derive_generated(
            env!("CARGO_PKG_NAME"),
            concat!(module_path!(), "::", stringify!(#ident)),
            #derive_macro_version,
        )
    };

    let semantic_impl = if kind == DeriveKind::Value {
        let namespace = attrs.namespace;
        let semantic_name = attrs.name;
        let semantic_digest =
            digest_array_literal(&format!("semantic:{namespace}:{semantic_name}:{version}"));
        quote! {
            fn semantic_id() -> ::mfm_values::Result<::mfm_ids::SemanticTypeId> {
                ::mfm_ids::SemanticTypeId::new(
                    #namespace,
                    #semantic_name,
                    #version,
                    ::mfm_ids::DigestAlgorithm::Sha256JcsV1,
                    ::mfm_ids::DigestBytes::from_array(#semantic_digest),
                )
                .map_err(|error| ::mfm_values::ValueError::Identity(error.to_string()))
            }
        }
    } else {
        quote! {}
    };

    let semantic_identity = if kind == DeriveKind::Value {
        quote!(Some(<Self as ::mfm_values::MfmValue>::semantic_id()?))
    } else {
        quote!(None)
    };

    let descriptor_body = quote! {
        #(#default_bounds)*

        ::mfm_values::SchemaDescriptor::new(
            ::mfm_values::SchemaIdentity::new(
                #schema_kind,
                #semantic_identity,
                #schema_name,
                ::mfm_ids::SchemaVersion::new(#version)
                    .map_err(|error| ::mfm_values::ValueError::Identity(error.to_string()))?,
                ::mfm_values::SchemaShape::named_struct(vec![#(#field_descriptors),*])?,
            )?,
            #audit_path,
        )
    };

    let impl_block = match kind {
        DeriveKind::Value => quote! {
            impl ::mfm_values::MfmValue for #ident {
                #semantic_impl

                fn schema_descriptor() -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    #descriptor_body
                }
            }
        },
        DeriveKind::Config => quote! {
            impl ::mfm_values::MfmConfig for #ident {
                fn schema_descriptor() -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    #descriptor_body
                }
            }
        },
        DeriveKind::StateInput => quote! {
            impl ::mfm_values::StateInput for #ident {
                fn #schema_method() -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    #descriptor_body
                }
            }
        },
        DeriveKind::OperationOutput => quote! {
            impl ::mfm_values::OperationOutput for #ident {
                fn #schema_method() -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    #descriptor_body
                }
            }
        },
        DeriveKind::PublicOutputs => quote! {
            impl ::mfm_values::PublicOutputDescriptor for #ident {
                fn #schema_method() -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    #descriptor_body
                }
            }

            impl ::mfm_values::PublicOutputs for #ident {}
        },
    };

    Ok(impl_block)
}

#[derive(Debug)]
struct ContainerAttrs {
    namespace: String,
    name: String,
    version: String,
    schema_name: String,
    rename_all: Option<String>,
}

impl ContainerAttrs {
    fn parse(attrs: &[Attribute], ident: &Ident) -> syn::Result<Self> {
        let type_name = snake_case(&ident.to_string());
        let mut output = Self {
            namespace: "mfm.derived".to_owned(),
            name: type_name.clone(),
            version: "1".to_owned(),
            schema_name: format!("mfm.derived.{type_name}"),
            rename_all: None,
        };

        for attr in attrs {
            if attr.path().is_ident("mfm") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("namespace") {
                        output.namespace = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("name") {
                        output.name = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("version") {
                        output.version = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("schema") {
                        output.schema_name = meta.value()?.parse::<LitStr>()?.value();
                    } else {
                        return Err(meta.error("unsupported #[mfm(...)] container attribute"));
                    }
                    Ok(())
                })?;
            } else if attr.path().is_ident("serde") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename_all") {
                        let rename_all = meta.value()?.parse::<LitStr>()?.value();
                        match rename_all.as_str() {
                            "snake_case" | "kebab-case" | "camelCase" => {
                                output.rename_all = Some(rename_all);
                                Ok(())
                            }
                            _ => {
                                Err(meta
                                    .error("unsupported serde(rename_all) value for MFM derive"))
                            }
                        }
                    } else {
                        Err(meta
                            .error("unsupported #[serde(...)] container attribute for MFM derive"))
                    }
                })?;
            }
        }

        Ok(output)
    }
}

fn named_struct_fields(
    data: &Data,
) -> syn::Result<&syn::punctuated::Punctuated<syn::Field, syn::Token![,]>> {
    match data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => Ok(&fields.named),
        Data::Struct(other) => Err(syn::Error::new(
            other.fields.span(),
            "MFM derives support named structs only in v1",
        )),
        Data::Enum(data) => Err(syn::Error::new(
            data.enum_token.span,
            "MFM derives support named structs only in v1; enum descriptors are not derive-generated yet",
        )),
        Data::Union(data) => Err(syn::Error::new(
            data.union_token.span,
            "MFM derives do not support unions",
        )),
    }
}

struct FieldDescriptorOutput {
    descriptors: Vec<proc_macro2::TokenStream>,
    default_bounds: Vec<proc_macro2::TokenStream>,
}

fn field_descriptor_tokens(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    rename_all: Option<&str>,
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
        let shape = shape_tokens(&field.ty)?;
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

#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    default: bool,
}

impl FieldAttrs {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut output = Self::default();
        for attr in attrs {
            if attr.path().is_ident("mfm") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        output.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else {
                        Err(meta.error("unsupported #[mfm(...)] field attribute"))
                    }
                })?;
            } else if attr.path().is_ident("serde") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        output.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("default") {
                        if meta.input.peek(syn::Token![=]) {
                            return Err(
                                meta.error("custom serde default functions are not supported")
                            );
                        }
                        output.default = true;
                        Ok(())
                    } else if meta.path.is_ident("skip")
                        || meta.path.is_ident("skip_serializing")
                        || meta.path.is_ident("skip_deserializing")
                    {
                        Err(meta.error("skipped fields are not supported by MFM derives"))
                    } else if meta.path.is_ident("flatten") {
                        Err(meta.error("serde(flatten) is not supported by MFM derives"))
                    } else if meta.path.is_ident("serialize_with")
                        || meta.path.is_ident("deserialize_with")
                        || meta.path.is_ident("with")
                    {
                        Err(meta.error("custom serde serializers are not supported by MFM derives"))
                    } else {
                        Err(meta.error("unsupported #[serde(...)] field attribute for MFM derive"))
                    }
                })?;
            }
        }
        Ok(output)
    }
}

fn shape_tokens(ty: &Type) -> syn::Result<proc_macro2::TokenStream> {
    reject_known_secret_type(ty)?;
    match ty {
        Type::Path(type_path) => shape_tokens_for_path(type_path),
        Type::Tuple(tuple) => {
            let elements = tuple
                .elems
                .iter()
                .map(shape_tokens)
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

fn shape_tokens_for_path(type_path: &TypePath) -> syn::Result<proc_macro2::TokenStream> {
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
            let shape = shape_tokens(element)?;
            Ok(quote!(::mfm_values::SchemaShape::Option(Box::new(#shape))))
        }
        "Vec" => {
            let element = one_generic_type(segment, "Vec")?;
            let shape = shape_tokens(element)?;
            Ok(quote!(::mfm_values::SchemaShape::Vec(Box::new(#shape))))
        }
        "BTreeMap" => {
            let (key, value) = two_generic_types(segment, "BTreeMap")?;
            if !is_string_type(key) {
                return Err(syn::Error::new_spanned(
                    key,
                    "BTreeMap keys must be String for MFM descriptors",
                ));
            }
            let value_shape = shape_tokens(value)?;
            Ok(quote!(::mfm_values::SchemaShape::BTreeMapString {
                value: Box::new(#value_shape)
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

fn one_generic_type<'a>(segment: &'a syn::PathSegment, label: &str) -> syn::Result<&'a Type> {
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            segment,
            format!("{label} must have one generic argument"),
        ));
    };
    if args.args.len() != 1 {
        return Err(syn::Error::new_spanned(
            args,
            format!("{label} must have one generic argument"),
        ));
    }
    match args.args.first() {
        Some(GenericArgument::Type(ty)) => Ok(ty),
        _ => Err(syn::Error::new_spanned(
            args,
            format!("{label} generic argument must be a type"),
        )),
    }
}

fn two_generic_types<'a>(
    segment: &'a syn::PathSegment,
    label: &str,
) -> syn::Result<(&'a Type, &'a Type)> {
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            segment,
            format!("{label} must have two generic arguments"),
        ));
    };
    if args.args.len() != 2 {
        return Err(syn::Error::new_spanned(
            args,
            format!("{label} must have two generic arguments"),
        ));
    }
    let mut iter = args.args.iter();
    let key = match iter.next() {
        Some(GenericArgument::Type(ty)) => ty,
        _ => {
            return Err(syn::Error::new_spanned(
                args,
                format!("{label} key argument must be a type"),
            ));
        }
    };
    let value = match iter.next() {
        Some(GenericArgument::Type(ty)) => ty,
        _ => {
            return Err(syn::Error::new_spanned(
                args,
                format!("{label} value argument must be a type"),
            ));
        }
    };
    Ok((key, value))
}

fn is_string_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "String"))
}

fn reject_known_secret_type(ty: &Type) -> syn::Result<()> {
    let text = quote!(#ty).to_string();
    for marker in [
        "Secret",
        "Password",
        "PrivateKey",
        "Mnemonic",
        "SigningKey",
        "Zeroizing",
    ] {
        if text.contains(marker) {
            return Err(syn::Error::new_spanned(
                ty,
                "known secret-bearing wrappers cannot implement MFM persisted surfaces",
            ));
        }
    }
    Ok(())
}

fn path_contains(path: &syn::Path, name: &str) -> bool {
    path.segments.iter().any(|segment| segment.ident == name)
}

fn digest_array_literal(seed: &str) -> proc_macro2::TokenStream {
    let digest = mfm_canonical::sha256_digest_bytes(seed.as_bytes());
    let bytes = digest.as_bytes().iter().map(|byte| {
        let literal = syn::LitInt::new(&format!("{byte}u8"), Span::call_site());
        quote!(#literal)
    });
    quote!([#(#bytes),*])
}

fn apply_rename_all(value: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("snake_case") | None => snake_case(value),
        Some("kebab-case") => snake_case(value).replace('_', "-"),
        Some("camelCase") => {
            let snake = snake_case(value);
            let mut output = String::new();
            let mut uppercase_next = false;
            for ch in snake.chars() {
                if ch == '_' {
                    uppercase_next = true;
                } else if uppercase_next {
                    output.extend(ch.to_uppercase());
                    uppercase_next = false;
                } else {
                    output.push(ch);
                }
            }
            output
        }
        Some(other) => {
            unreachable!("unsupported rename_all value should have been rejected: {other}")
        }
    }
}

fn snake_case(value: &str) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('_');
            }
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}
