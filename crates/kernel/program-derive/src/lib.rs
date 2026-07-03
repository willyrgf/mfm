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
    Attribute, Data, DataEnum, DataStruct, DeriveInput, Fields, FieldsNamed, FieldsUnnamed,
    GenericArgument, GenericParam, Ident, LitStr, Path, PathArguments, Type, TypePath, Variant,
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

#[proc_macro_derive(MfmFactType, attributes(mfm_fact, mfm, serde))]
/// Derives `mfm_program::MfmFactType` for a fact wrapper struct.
pub fn derive_mfm_fact_type(input: TokenStream) -> TokenStream {
    expand_mfm_fact_type_derive(parse_macro_input!(input as DeriveInput)).into()
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
    if kind == DeriveKind::PublicOutputs && !input.generics.params.is_empty() {
        return expand_program_public_outputs_derive_result(input);
    }
    if kind == DeriveKind::OperationOutput && !input.generics.params.is_empty() {
        return expand_program_operation_output_derive_result(input);
    }

    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "MFM derives do not support generic structs in v1",
        ));
    }

    let attrs = ContainerAttrs::parse(&input.attrs, &input.ident)?;
    let shape_output = schema_shape_tokens(&input.data, attrs.rename_all.as_deref(), kind, &attrs)?;
    let state_input_handles = if kind == DeriveKind::StateInput {
        let fields = named_struct_fields(&input.data)?;
        generated_state_input_handles_tokens(&input.ident, fields, attrs.rename_all.as_deref())?
    } else {
        quote! {}
    };
    let shape = shape_output.shape;
    let default_bounds = shape_output.default_bounds;
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
                #shape,
            )?,
            #audit_path,
        )
    };

    let config_validate_method =
        if let (DeriveKind::Config, Some(validate_path)) = (kind, &attrs.validate) {
            quote! {
                fn validate(
                    &self,
                ) -> ::std::result::Result<(), ::mfm_values::ConfigError> {
                    #validate_path(self)
                        .map_err(|error| ::mfm_values::ConfigError::new(error.to_string()))
                }
            }
        } else {
            quote! {}
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

                #config_validate_method
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

    Ok(quote! {
        #impl_block
        #state_input_handles
    })
}

fn expand_mfm_fact_type_derive(input: DeriveInput) -> proc_macro2::TokenStream {
    match expand_mfm_fact_type_derive_result(input) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn expand_mfm_fact_type_derive_result(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "MfmFactType derive does not support generic structs in v1",
        ));
    }

    let attrs = FactContainerAttrs::parse(&input.attrs)?;
    validate_fact_container_attrs(&attrs)?;
    let fields = named_struct_fields(&input.data)?;
    let subject_field = fact_wrapper_field(fields, FactWrapperFieldRole::Subject)?;
    let response_field = fact_wrapper_field(fields, FactWrapperFieldRole::Response)?;
    let subject_ident = subject_field
        .ident
        .as_ref()
        .expect("named_struct_fields returns named fields");
    let response_ident = response_field
        .ident
        .as_ref()
        .expect("named_struct_fields returns named fields");
    let subject_ty = &subject_field.ty;
    let response_ty = &response_field.ty;
    let ident = &input.ident;
    let fact_kind = attrs.kind;
    let field_tokens = attrs
        .fields
        .iter()
        .map(fact_field_descriptor_tokens)
        .collect::<syn::Result<Vec<_>>>()?;
    let ordering_tokens = attrs
        .orderings
        .iter()
        .map(fact_ordering_descriptor_tokens)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        impl ::mfm_program::MfmFactType for #ident {
            type Subject = #subject_ty;
            type Response = #response_ty;

            fn descriptor() -> ::mfm_program::facts::Result<::mfm_program::facts::FactDescriptor> {
                let subject_schema_id = <Self::Subject as ::mfm_values::MfmValue>::schema_id()
                    .map_err(|error| ::mfm_program::facts::FactDescriptorError::descriptor(error.to_string()))?;
                let response_schema_id = <Self::Response as ::mfm_values::MfmValue>::schema_id()
                    .map_err(|error| ::mfm_program::facts::FactDescriptorError::descriptor(error.to_string()))?;

                ::mfm_program::facts::FactDescriptor::new(
                    ::mfm_program::facts::FactKind::new(#fact_kind)?,
                    ::mfm_program::facts::fact_descriptor_schema_id()?,
                    subject_schema_id,
                    response_schema_id,
                    vec![#(#field_tokens),*],
                    vec![#(#ordering_tokens),*],
                )
            }

            fn subject(&self) -> &Self::Subject {
                &self.#subject_ident
            }

            fn response(&self) -> &Self::Response {
                &self.#response_ident
            }
        }
    })
}

fn expand_program_public_outputs_derive_result(
    input: DeriveInput,
) -> syn::Result<proc_macro2::TokenStream> {
    let lifetimes = input
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Lifetime(lifetime) => Some(&lifetime.lifetime),
            _ => None,
        })
        .collect::<Vec<_>>();
    if lifetimes.len() != 2 || input.generics.params.len() != 2 {
        return Err(syn::Error::new_spanned(
            input.generics,
            "program PublicOutputs derive expects exactly two lifetime parameters",
        ));
    }
    let program_lifetime = lifetimes[0];
    let scope_lifetime = lifetimes[1];

    let attrs = ContainerAttrs::parse(&input.attrs, &input.ident)?;
    let fields = named_struct_fields(&input.data)?;
    let field_output = program_public_output_field_tokens(
        fields,
        attrs.rename_all.as_deref(),
        program_lifetime,
        scope_lifetime,
    )?;
    let field_descriptors = field_output.descriptors;
    let output_cells = field_output.output_cells;
    let ident = &input.ident;
    let schema_name = attrs.schema_name;
    let version = attrs.version;
    let derive_macro_version = concat!("mfm-program-derive/", env!("CARGO_PKG_VERSION"));
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::mfm_program::PublicOutputs<#program_lifetime, #scope_lifetime>
            for #ident #ty_generics #where_clause
        {
            fn public_schema_id(&self) -> ::mfm_program::Result<::mfm_ids::SchemaId> {
                let descriptor = (|| -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    ::mfm_values::SchemaDescriptor::new(
                        ::mfm_values::SchemaIdentity::new(
                            ::mfm_values::SchemaKind::PublicOutput,
                            None,
                            #schema_name,
                            ::mfm_ids::SchemaVersion::new(#version)
                                .map_err(|error| ::mfm_values::ValueError::Identity(error.to_string()))?,
                            ::mfm_values::SchemaShape::named_struct(vec![#(#field_descriptors),*])?,
                        )?,
                        ::mfm_values::SchemaAudit::__derive_generated(
                            env!("CARGO_PKG_NAME"),
                            concat!(module_path!(), "::", stringify!(#ident)),
                            #derive_macro_version,
                        ),
                    )
                })()
                .map_err(|error| ::mfm_program::PlanError::Value(error.to_string()))?;
                descriptor
                    .schema_id()
                    .map_err(|error| ::mfm_program::PlanError::Value(error.to_string()))
            }

            fn output_cells(
                &self,
            ) -> ::mfm_program::Result<Vec<::mfm_program::PublicOutputCellSpec>> {
                Ok(vec![#(#output_cells),*])
            }
        }
    })
}

fn expand_program_operation_output_derive_result(
    input: DeriveInput,
) -> syn::Result<proc_macro2::TokenStream> {
    let lifetimes = input
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Lifetime(lifetime) => Some(&lifetime.lifetime),
            _ => None,
        })
        .collect::<Vec<_>>();
    if lifetimes.len() != 2 || input.generics.params.len() != 2 {
        return Err(syn::Error::new_spanned(
            input.generics,
            "program OperationOutput derive expects exactly two lifetime parameters",
        ));
    }
    let program_lifetime = lifetimes[0];
    let scope_lifetime = lifetimes[1];

    let attrs = ContainerAttrs::parse(&input.attrs, &input.ident)?;
    let fields = named_struct_fields(&input.data)?;
    let field_output = program_operation_output_field_tokens(
        fields,
        attrs.rename_all.as_deref(),
        program_lifetime,
        scope_lifetime,
    )?;
    let field_descriptors = field_output.descriptors;
    let output_handles = field_output.output_cells;
    let ident = &input.ident;
    let schema_name = attrs.schema_name;
    let version = attrs.version;
    let derive_macro_version = concat!("mfm-program-derive/", env!("CARGO_PKG_VERSION"));
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::mfm_program::OperationOutput<#program_lifetime, #scope_lifetime>
            for #ident #ty_generics #where_clause
        {
            fn output_schema_id() -> ::mfm_program::Result<::mfm_ids::SchemaId> {
                let descriptor = (|| -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    ::mfm_values::SchemaDescriptor::new(
                        ::mfm_values::SchemaIdentity::new(
                            ::mfm_values::SchemaKind::OperationOutput,
                            None,
                            #schema_name,
                            ::mfm_ids::SchemaVersion::new(#version)
                                .map_err(|error| ::mfm_values::ValueError::Identity(error.to_string()))?,
                            ::mfm_values::SchemaShape::named_struct(vec![#(#field_descriptors),*])?,
                        )?,
                        ::mfm_values::SchemaAudit::__derive_generated(
                            env!("CARGO_PKG_NAME"),
                            concat!(module_path!(), "::", stringify!(#ident)),
                            #derive_macro_version,
                        ),
                    )
                })()
                .map_err(|error| ::mfm_program::PlanError::Value(error.to_string()))?;
                descriptor
                    .schema_id()
                    .map_err(|error| ::mfm_program::PlanError::Value(error.to_string()))
            }

            fn output_handles(
                &self,
            ) -> ::mfm_program::Result<Vec<::mfm_program::TypedHandleRef>> {
                Ok(vec![#(#output_handles),*])
            }
        }
    })
}

#[derive(Debug)]
struct ContainerAttrs {
    namespace: String,
    name: String,
    version: String,
    schema_name: String,
    rename_all: Option<String>,
    enum_tag: Option<String>,
    enum_content: Option<String>,
    validate: Option<Path>,
    transparent_string: bool,
    transparent_map: bool,
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
            enum_tag: None,
            enum_content: None,
            validate: None,
            transparent_string: false,
            transparent_map: false,
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
                    } else if meta.path.is_ident("validate") {
                        let value = meta.value()?.parse::<LitStr>()?.value();
                        output.validate =
                            Some(syn::parse_str::<Path>(&value).map_err(|error| {
                                syn::Error::new(
                                    meta.path.span(),
                                    format!("invalid config validator path: {error}"),
                                )
                            })?);
                    } else if meta.path.is_ident("transparent_string") {
                        output.transparent_string = true;
                    } else if meta.path.is_ident("transparent_map") {
                        output.transparent_map = true;
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
                    } else if meta.path.is_ident("tag") {
                        output.enum_tag = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("content") {
                        output.enum_content = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("try_from") || meta.path.is_ident("into") {
                        let _ = meta.value()?.parse::<LitStr>()?;
                        Ok(())
                    } else if meta.path.is_ident("transparent") {
                        Ok(())
                    } else if meta.path.is_ident("untagged") {
                        Err(meta.error("serde(untagged) is not supported by MFM derives"))
                    } else {
                        Err(meta
                            .error("unsupported #[serde(...)] container attribute for MFM derive"))
                    }
                })?;
            }
        }

        if output.transparent_string && output.transparent_map {
            return Err(syn::Error::new(
                Span::call_site(),
                "MFM derives accept only one transparent container mode",
            ));
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

struct SchemaShapeOutput {
    shape: proc_macro2::TokenStream,
    default_bounds: Vec<proc_macro2::TokenStream>,
}

struct ProgramPublicOutputFieldOutput {
    descriptors: Vec<proc_macro2::TokenStream>,
    output_cells: Vec<proc_macro2::TokenStream>,
}

fn schema_shape_tokens(
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
        if internal_tagged && !matches!(variant.fields, Fields::Named(_)) {
            return Err(syn::Error::new(
                variant.ident.span(),
                "internally tagged MFM enum variants must have named fields",
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

fn program_public_output_field_tokens(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    rename_all: Option<&str>,
    program_lifetime: &syn::Lifetime,
    scope_lifetime: &syn::Lifetime,
) -> syn::Result<ProgramPublicOutputFieldOutput> {
    let mut descriptors = Vec::new();
    let mut output_cells = Vec::new();
    let mut names = Vec::new();

    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "MFM derives require named fields"))?;
        let attrs = FieldAttrs::parse(&field.attrs)?;
        if attrs.default {
            return Err(syn::Error::new(
                ident.span(),
                "program public output handle fields cannot use serde(default)",
            ));
        }
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
        let value_ty = handle_value_type(&field.ty, program_lifetime, scope_lifetime)?;
        descriptors.push(quote! {
            ::mfm_values::FieldDescriptor::required(
                #wire_name,
                ::mfm_values::SchemaShape::ValueRef {
                    schema_id: <#value_ty as ::mfm_values::MfmValue>::schema_id()?,
                    semantic_type_id: <#value_ty as ::mfm_values::MfmValue>::semantic_id()?,
                },
            )
        });
        output_cells.push(quote! {
            ::mfm_program::PublicOutputCellSpec::from_handle(
                ::mfm_program::PublicFieldPath::new(#wire_name)?,
                &self.#ident,
            )
        });
    }

    Ok(ProgramPublicOutputFieldOutput {
        descriptors,
        output_cells,
    })
}

fn program_operation_output_field_tokens(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    rename_all: Option<&str>,
    program_lifetime: &syn::Lifetime,
    scope_lifetime: &syn::Lifetime,
) -> syn::Result<ProgramPublicOutputFieldOutput> {
    let mut descriptors = Vec::new();
    let mut output_cells = Vec::new();
    let mut names = Vec::new();

    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "MFM derives require named fields"))?;
        let attrs = FieldAttrs::parse(&field.attrs)?;
        if attrs.default {
            return Err(syn::Error::new(
                ident.span(),
                "program operation output handle fields cannot use serde(default)",
            ));
        }
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
        let value_ty = handle_value_type(&field.ty, program_lifetime, scope_lifetime)?;
        descriptors.push(quote! {
            ::mfm_values::FieldDescriptor::required(
                #wire_name,
                ::mfm_values::SchemaShape::ValueRef {
                    schema_id: <#value_ty as ::mfm_values::MfmValue>::schema_id()?,
                    semantic_type_id: <#value_ty as ::mfm_values::MfmValue>::semantic_id()?,
                },
            )
        });
        output_cells.push(quote! {
            self.#ident.typed_ref()
        });
    }

    Ok(ProgramPublicOutputFieldOutput {
        descriptors,
        output_cells,
    })
}

fn generated_state_input_handles_tokens(
    input_ident: &Ident,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    rename_all: Option<&str>,
) -> syn::Result<proc_macro2::TokenStream> {
    let handle_ident = format_ident!("{}Handles", input_ident);
    let program_lifetime = syn::Lifetime::new("'program", Span::call_site());
    let scope_lifetime = syn::Lifetime::new("'scope", Span::call_site());
    let mut handle_fields = Vec::new();
    let mut input_bindings = Vec::new();
    let mut names = Vec::new();

    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "MFM derives require named fields"))?;
        let attrs = FieldAttrs::parse(&field.attrs)?;
        if attrs.default {
            return Err(syn::Error::new(
                ident.span(),
                "state input fields cannot use serde(default)",
            ));
        }
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

        let runtime_input_ty = &field.ty;
        let handle_ty = generated_state_input_handle_type_tokens(
            runtime_input_ty,
            &program_lifetime,
            &scope_lifetime,
        )?;
        handle_fields.push(quote! {
            pub #ident: #handle_ty
        });
        input_bindings.push(quote! {
            {
                let field_path = field_path.child(#wire_name)?;
                ::mfm_program::NamedInputBinding::new(
                    field_path.clone(),
                    ::mfm_program::IntoInputBindingNode::<#runtime_input_ty>::into_binding_node(
                        self.#ident,
                        field_path,
                    )?,
                )
            }
        });
    }

    Ok(quote! {
        #[derive(Clone)]
        #[allow(missing_docs)]
        pub struct #handle_ident<#program_lifetime, #scope_lifetime> {
            #(#handle_fields,)*
        }

        impl<#program_lifetime, #scope_lifetime> ::mfm_program::IntoInputBindingNode<#input_ident>
            for #handle_ident<#program_lifetime, #scope_lifetime>
        {
            fn into_binding_node(
                self,
                field_path: ::mfm_program::InputFieldPath,
            ) -> ::mfm_program::Result<::mfm_program::InputBindingNode> {
                ::mfm_program::InputBindingNode::struct_fields(vec![#(#input_bindings),*])
            }
        }

        impl<#program_lifetime, #scope_lifetime> ::mfm_program::IntoStateInput<#program_lifetime, #scope_lifetime, #input_ident>
            for #handle_ident<#program_lifetime, #scope_lifetime>
        {
            fn into_binding(self) -> ::mfm_program::Result<::mfm_program::InputBinding<#input_ident>> {
                ::mfm_program::InputBinding::from_root(
                    <Self as ::mfm_program::IntoInputBindingNode<#input_ident>>::into_binding_node(
                        self,
                        ::mfm_program::InputFieldPath::root(),
                    )?,
                )
            }
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactWrapperFieldRole {
    Subject,
    Response,
}

#[derive(Debug)]
struct FactContainerAttrs {
    kind: String,
    fields: Vec<FactFieldAttr>,
    orderings: Vec<FactOrderingAttr>,
}

impl FactContainerAttrs {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut kind = None;
        let mut fields = Vec::new();
        let mut orderings = Vec::new();

        for attr in attrs {
            if !attr.path().is_ident("mfm_fact") {
                continue;
            }

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("kind") {
                    kind = Some(meta.value()?.parse::<LitStr>()?.value());
                    Ok(())
                } else if meta.path.is_ident("field") {
                    fields.push(FactFieldAttr::parse(meta)?);
                    Ok(())
                } else if meta.path.is_ident("ordering") {
                    orderings.push(FactOrderingAttr::parse(meta)?);
                    Ok(())
                } else {
                    Err(meta.error("unsupported #[mfm_fact(...)] container attribute"))
                }
            })?;
        }

        let kind = kind.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "MfmFactType derive requires #[mfm_fact(kind = \"...\")]",
            )
        })?;
        if fields.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "MfmFactType derive requires at least one #[mfm_fact(field(...))] descriptor field",
            ));
        }

        Ok(Self {
            kind,
            fields,
            orderings,
        })
    }
}

fn validate_fact_container_attrs(attrs: &FactContainerAttrs) -> syn::Result<()> {
    let mut seen_fields = Vec::new();
    for field in &attrs.fields {
        if seen_fields.contains(&field.id.as_str()) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "MfmFactType derive found duplicate fact field id {:?}",
                    field.id
                ),
            ));
        }
        seen_fields.push(field.id.as_str());

        if field.source == FactFieldAttrSource::Subject && !field.required {
            return Err(syn::Error::new(
                Span::call_site(),
                "MfmFactType derive does not allow optional subject fields",
            ));
        }

        validate_fact_field_attr(field)?;
    }

    if !attrs
        .fields
        .iter()
        .any(|field| field.source == FactFieldAttrSource::Subject)
    {
        return Err(syn::Error::new(
            Span::call_site(),
            "MfmFactType derive requires at least one subject fact field",
        ));
    }

    for ordering in &attrs.orderings {
        for term in &ordering.terms {
            let Some(field) = attrs.fields.iter().find(|field| field.id == term.field) else {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "MfmFactType ordering {:?} references unknown fact field {:?}",
                        ordering.name, term.field
                    ),
                ));
            };
            if !field.sortable {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "MfmFactType ordering {:?} references non-sortable fact field {:?}",
                        ordering.name, term.field
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn validate_fact_field_attr(field: &FactFieldAttr) -> syn::Result<()> {
    let mut seen_operators = Vec::new();
    for operator in &field.operators {
        if seen_operators.contains(&operator.as_str()) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "MfmFactType field {:?} repeats operator {:?}",
                    field.id, operator
                ),
            ));
        }
        seen_operators.push(operator.as_str());

        if !fact_value_type_supports_operator(&field.value_type, operator) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "MfmFactType field {:?} uses operator {:?} incompatible with value_type {:?}",
                    field.id, operator, field.value_type
                ),
            ));
        }
    }

    if field.sortable && !fact_value_type_is_sortable(&field.value_type) {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "MfmFactType field {:?} marks non-sortable value_type {:?} sortable",
                field.id, field.value_type
            ),
        ));
    }

    Ok(())
}

fn fact_value_type_supports_operator(value_type: &str, operator: &str) -> bool {
    if operator == "equal" {
        return true;
    }
    matches!(
        (value_type, operator),
        (
            "signed_integer" | "unsigned_integer" | "timestamp" | "decimal_string",
            "less_than" | "less_than_or_equal" | "greater_than" | "greater_than_or_equal"
        )
    )
}

fn fact_value_type_is_sortable(value_type: &str) -> bool {
    matches!(
        value_type,
        "signed_integer" | "unsigned_integer" | "timestamp" | "decimal_string"
    )
}

#[derive(Debug)]
struct FactFieldAttr {
    id: String,
    source: FactFieldAttrSource,
    path: Option<String>,
    metadata: Option<String>,
    value_type: String,
    operators: Vec<String>,
    exposure: String,
    unit: Option<String>,
    scale: Option<i16>,
    sortable: bool,
    required: bool,
}

impl FactFieldAttr {
    fn parse(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<Self> {
        let mut id = None;
        let mut source = None;
        let mut path = None;
        let mut metadata = None;
        let mut value_type = None;
        let mut operators = Vec::new();
        let mut exposure = None;
        let mut unit = None;
        let mut scale = None;
        let mut sortable = false;
        let mut required = true;

        meta.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                id = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("source") {
                source = Some(FactFieldAttrSource::parse(
                    &meta.value()?.parse::<LitStr>()?.value(),
                    meta.path.span(),
                )?);
            } else if meta.path.is_ident("path") {
                path = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("metadata") {
                metadata = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("value_type") {
                value_type = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("operator") {
                operators.push(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("operators") {
                meta.parse_nested_meta(|operator| {
                    operators.push(operator.path.require_ident()?.to_string());
                    Ok(())
                })?;
            } else if meta.path.is_ident("exposure") {
                exposure = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("unit") {
                unit = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("scale") {
                scale = Some(
                    meta.value()?
                        .parse::<syn::LitInt>()?
                        .base10_parse::<i16>()?,
                );
            } else if meta.path.is_ident("sortable") {
                sortable = true;
            } else if meta.path.is_ident("optional") {
                required = false;
            } else if meta.path.is_ident("required") {
                required = true;
            } else {
                return Err(meta.error("unsupported mfm_fact field attribute"));
            }
            Ok(())
        })?;

        let id = id.ok_or_else(|| meta.error("mfm_fact field requires id = \"...\""))?;
        let source =
            source.ok_or_else(|| meta.error("mfm_fact field requires source = \"...\""))?;
        let value_type =
            value_type.ok_or_else(|| meta.error("mfm_fact field requires value_type = \"...\""))?;
        let exposure =
            exposure.ok_or_else(|| meta.error("mfm_fact field requires exposure = \"...\""))?;
        if operators.is_empty() {
            operators.push("equal".to_owned());
        }

        match source {
            FactFieldAttrSource::Subject | FactFieldAttrSource::Result => {
                if path.is_none() {
                    return Err(meta.error("subject/result fact fields require path = \"...\""));
                }
                if metadata.is_some() {
                    return Err(meta.error("subject/result fact fields cannot use metadata"));
                }
            }
            FactFieldAttrSource::Metadata => {
                if metadata.is_none() {
                    return Err(meta.error("metadata fact fields require metadata = \"...\""));
                }
                if path.is_some() {
                    return Err(meta.error("metadata fact fields cannot use path"));
                }
            }
        }

        Ok(Self {
            id,
            source,
            path,
            metadata,
            value_type,
            operators,
            exposure,
            unit,
            scale,
            sortable,
            required,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactFieldAttrSource {
    Subject,
    Result,
    Metadata,
}

impl FactFieldAttrSource {
    fn parse(value: &str, span: Span) -> syn::Result<Self> {
        match value {
            "subject" => Ok(Self::Subject),
            "result" => Ok(Self::Result),
            "metadata" => Ok(Self::Metadata),
            _ => Err(syn::Error::new(
                span,
                "fact field source must be subject, result, or metadata",
            )),
        }
    }
}

#[derive(Debug)]
struct FactOrderingAttr {
    name: String,
    terms: Vec<FactOrderingTermAttr>,
}

impl FactOrderingAttr {
    fn parse(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<Self> {
        let mut name = None;
        let mut terms = Vec::new();

        meta.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                name = Some(meta.value()?.parse::<LitStr>()?.value());
                Ok(())
            } else if meta.path.is_ident("term") {
                terms.push(FactOrderingTermAttr::parse(meta)?);
                Ok(())
            } else {
                Err(meta.error("unsupported mfm_fact ordering attribute"))
            }
        })?;

        let name = name.ok_or_else(|| meta.error("mfm_fact ordering requires name = \"...\""))?;
        if terms.is_empty() {
            return Err(meta.error("mfm_fact ordering requires at least one term(...)"));
        }
        Ok(Self { name, terms })
    }
}

#[derive(Debug)]
struct FactOrderingTermAttr {
    field: String,
    direction: String,
    nulls: String,
    tie_breaker: bool,
}

impl FactOrderingTermAttr {
    fn parse(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<Self> {
        let mut field = None;
        let mut direction = None;
        let mut nulls = None;
        let mut tie_breaker = false;

        meta.parse_nested_meta(|meta| {
            if meta.path.is_ident("field") {
                field = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("direction") {
                direction = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("nulls") {
                nulls = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("tie_breaker") {
                tie_breaker = true;
            } else {
                return Err(meta.error("unsupported mfm_fact ordering term attribute"));
            }
            Ok(())
        })?;

        Ok(Self {
            field: field.ok_or_else(|| meta.error("ordering term requires field = \"...\""))?,
            direction: direction
                .ok_or_else(|| meta.error("ordering term requires direction = \"...\""))?,
            nulls: nulls.ok_or_else(|| meta.error("ordering term requires nulls = \"...\""))?,
            tie_breaker,
        })
    }
}

fn fact_wrapper_field(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    role: FactWrapperFieldRole,
) -> syn::Result<&syn::Field> {
    let mut matches = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "MfmFactType requires named fields"))?;
        let explicit_role = fact_wrapper_field_role(&field.attrs)?;
        let default_role = match ident.to_string().as_str() {
            "subject" => Some(FactWrapperFieldRole::Subject),
            "response" => Some(FactWrapperFieldRole::Response),
            _ => None,
        };
        if explicit_role.or(default_role) == Some(role) {
            matches.push(field);
        }
    }

    match matches.as_slice() {
        [field] => Ok(*field),
        [] => Err(syn::Error::new(
            Span::call_site(),
            match role {
                FactWrapperFieldRole::Subject => {
                    "MfmFactType derive requires one subject field named subject or marked #[mfm_fact(subject)]"
                }
                FactWrapperFieldRole::Response => {
                    "MfmFactType derive requires one response field named response or marked #[mfm_fact(response)]"
                }
            },
        )),
        [first, ..] => Err(syn::Error::new(
            first.span(),
            match role {
                FactWrapperFieldRole::Subject => "MfmFactType derive found multiple subject fields",
                FactWrapperFieldRole::Response => {
                    "MfmFactType derive found multiple response fields"
                }
            },
        )),
    }
}

fn fact_wrapper_field_role(attrs: &[Attribute]) -> syn::Result<Option<FactWrapperFieldRole>> {
    let mut role = None;
    for attr in attrs {
        if !attr.path().is_ident("mfm_fact") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            let next = if meta.path.is_ident("subject") {
                FactWrapperFieldRole::Subject
            } else if meta.path.is_ident("response") {
                FactWrapperFieldRole::Response
            } else {
                return Err(meta.error("fact wrapper fields support only #[mfm_fact(subject)] or #[mfm_fact(response)]"));
            };
            if role.replace(next).is_some() {
                return Err(meta.error("duplicate fact wrapper field role"));
            }
            Ok(())
        })?;
    }
    Ok(role)
}

fn fact_field_descriptor_tokens(field: &FactFieldAttr) -> syn::Result<proc_macro2::TokenStream> {
    let id = &field.id;
    let accessor = fact_field_accessor_tokens(field)?;
    let value_type = fact_value_type_tokens(&field.value_type, Span::call_site())?;
    let operators = field
        .operators
        .iter()
        .map(|operator| fact_operator_tokens(operator, Span::call_site()))
        .collect::<syn::Result<Vec<_>>>()?;
    let exposure = fact_exposure_tokens(&field.exposure, Span::call_site())?;
    let unit = field
        .unit
        .as_ref()
        .map(|unit| quote!(Some(::mfm_program::facts::FactUnit::new(#unit)?)))
        .unwrap_or_else(|| quote!(None));
    let scale = field
        .scale
        .map(|scale| quote!(Some(::mfm_program::facts::FactScale::new(#scale)?)))
        .unwrap_or_else(|| quote!(None));
    let sortable = field.sortable;
    let required = field.required;

    Ok(quote! {
        ::mfm_program::facts::FactFieldDescriptor::new(
            ::mfm_program::facts::FactFieldId::new(#id)?,
            ::mfm_program::facts::FactFieldPath::new(#id)?,
            #value_type,
            #accessor,
            vec![#(#operators),*],
            #exposure,
            #unit,
            #scale,
            #sortable,
            #required,
        )?
    })
}

fn fact_field_accessor_tokens(field: &FactFieldAttr) -> syn::Result<proc_macro2::TokenStream> {
    match field.source {
        FactFieldAttrSource::Subject => {
            let path = field.path.as_ref().expect("validated subject path");
            Ok(quote!(::mfm_program::facts::FactFieldAccessor::SubjectPath(
                ::mfm_program::facts::CanonicalValuePath::new(#path)?
            )))
        }
        FactFieldAttrSource::Result => {
            let path = field.path.as_ref().expect("validated result path");
            Ok(
                quote!(::mfm_program::facts::FactFieldAccessor::ResponsePath(
                    ::mfm_program::facts::CanonicalValuePath::new(#path)?
                )),
            )
        }
        FactFieldAttrSource::Metadata => {
            let metadata = field.metadata.as_ref().expect("validated metadata field");
            let metadata = fact_metadata_field_tokens(metadata, Span::call_site())?;
            Ok(quote!(::mfm_program::facts::FactFieldAccessor::Metadata(#metadata)))
        }
    }
}

fn fact_ordering_descriptor_tokens(
    ordering: &FactOrderingAttr,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = &ordering.name;
    let terms = ordering
        .terms
        .iter()
        .map(fact_ordering_term_tokens)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote! {
        ::mfm_program::facts::FactOrderingDescriptor::new(
            ::mfm_program::facts::FactOrderingName::new(#name)?,
            vec![#(#terms),*],
        )?
    })
}

fn fact_ordering_term_tokens(term: &FactOrderingTermAttr) -> syn::Result<proc_macro2::TokenStream> {
    let field = &term.field;
    let direction = fact_sort_direction_tokens(&term.direction, Span::call_site())?;
    let nulls = fact_null_ordering_tokens(&term.nulls, Span::call_site())?;
    let tie_breaker = term.tie_breaker;
    Ok(quote! {
        ::mfm_program::facts::FactOrderingTerm::new(
            ::mfm_program::facts::FactFieldId::new(#field)?,
            #direction,
            #nulls,
            #tie_breaker,
        )
    })
}

fn fact_value_type_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "string" => Ok(quote!(::mfm_program::facts::FactFieldValueType::String)),
        "boolean" => Ok(quote!(::mfm_program::facts::FactFieldValueType::Boolean)),
        "signed_integer" => Ok(quote!(::mfm_program::facts::FactFieldValueType::SignedInteger)),
        "unsigned_integer" => Ok(quote!(::mfm_program::facts::FactFieldValueType::UnsignedInteger)),
        "timestamp" => Ok(quote!(::mfm_program::facts::FactFieldValueType::Timestamp)),
        "decimal_string" => Ok(quote!(::mfm_program::facts::FactFieldValueType::DecimalString)),
        "digest" => Ok(quote!(::mfm_program::facts::FactFieldValueType::Digest)),
        _ => Err(syn::Error::new(
            span,
            "fact value_type must be string, boolean, signed_integer, unsigned_integer, timestamp, decimal_string, or digest",
        )),
    }
}

fn fact_operator_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "equal" => Ok(quote!(::mfm_program::facts::FactQueryOperator::Equal)),
        "less_than" => Ok(quote!(::mfm_program::facts::FactQueryOperator::LessThan)),
        "less_than_or_equal" => Ok(quote!(::mfm_program::facts::FactQueryOperator::LessThanOrEqual)),
        "greater_than" => Ok(quote!(::mfm_program::facts::FactQueryOperator::GreaterThan)),
        "greater_than_or_equal" => Ok(quote!(::mfm_program::facts::FactQueryOperator::GreaterThanOrEqual)),
        _ => Err(syn::Error::new(
            span,
            "fact operator must be equal, less_than, less_than_or_equal, greater_than, or greater_than_or_equal",
        )),
    }
}

fn fact_exposure_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "returnable" => Ok(quote!(::mfm_program::facts::FactFieldExposure::Returnable)),
        "query_only" => Ok(quote!(::mfm_program::facts::FactFieldExposure::QueryOnly)),
        "hidden" => Ok(quote!(::mfm_program::facts::FactFieldExposure::Hidden)),
        _ => Err(syn::Error::new(
            span,
            "fact exposure must be returnable, query_only, or hidden",
        )),
    }
}

fn fact_metadata_field_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "recorded_at" => Ok(quote!(::mfm_program::facts::FactMetadataField::RecordedAt)),
        "observed_at" => Ok(quote!(::mfm_program::facts::FactMetadataField::ObservedAt)),
        "store_commit_order" => Ok(quote!(
            ::mfm_program::facts::FactMetadataField::StoreCommitOrder
        )),
        _ => Err(syn::Error::new(
            span,
            "fact metadata must be recorded_at, observed_at, or store_commit_order",
        )),
    }
}

fn fact_sort_direction_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "ascending" => Ok(quote!(::mfm_program::facts::SortDirection::Ascending)),
        "descending" => Ok(quote!(::mfm_program::facts::SortDirection::Descending)),
        _ => Err(syn::Error::new(
            span,
            "fact ordering direction must be ascending or descending",
        )),
    }
}

fn fact_null_ordering_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "first" => Ok(quote!(::mfm_program::facts::NullOrdering::First)),
        "last" => Ok(quote!(::mfm_program::facts::NullOrdering::Last)),
        _ => Err(syn::Error::new(
            span,
            "fact ordering nulls must be first or last",
        )),
    }
}

#[derive(Debug, Default)]
struct VariantAttrs {
    rename: Option<String>,
}

impl VariantAttrs {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut output = Self::default();
        for attr in attrs {
            if attr.path().is_ident("serde") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        output.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("alias") {
                        Err(meta.error("serde(alias) is not supported by MFM derives"))
                    } else {
                        Err(meta
                            .error("unsupported #[serde(...)] variant attribute for MFM derive"))
                    }
                })?;
            }
        }
        Ok(output)
    }
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

fn handle_value_type<'a>(
    ty: &'a Type,
    program_lifetime: &syn::Lifetime,
    scope_lifetime: &syn::Lifetime,
) -> syn::Result<&'a Type> {
    let Type::Path(type_path) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "handle fields must be mfm_program::Handle<'p, 's, T>",
        ));
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            ty,
            "handle fields must be mfm_program::Handle<'p, 's, T>",
        ));
    };
    if segment.ident != "Handle" {
        return Err(syn::Error::new_spanned(
            ty,
            "handle fields must be mfm_program::Handle<'p, 's, T>",
        ));
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            ty,
            "Handle fields must include program lifetime, scope lifetime, and value type",
        ));
    };
    if args.args.len() != 3 {
        return Err(syn::Error::new_spanned(
            args,
            "Handle fields must include program lifetime, scope lifetime, and value type",
        ));
    }
    match args.args.first() {
        Some(GenericArgument::Lifetime(lifetime)) if lifetime.ident == program_lifetime.ident => {}
        Some(arg) => {
            return Err(syn::Error::new_spanned(
                arg,
                "Handle program lifetime must match the first PublicOutputs lifetime",
            ));
        }
        None => unreachable!("checked argument length"),
    }
    match args.args.iter().nth(1) {
        Some(GenericArgument::Lifetime(lifetime)) if lifetime.ident == scope_lifetime.ident => {}
        Some(arg) => {
            return Err(syn::Error::new_spanned(
                arg,
                "Handle scope lifetime must match the second PublicOutputs lifetime",
            ));
        }
        None => unreachable!("checked argument length"),
    }
    match args.args.last() {
        Some(GenericArgument::Type(value_ty)) => Ok(value_ty),
        _ => Err(syn::Error::new_spanned(
            args,
            "Handle value argument must be a type",
        )),
    }
}

fn generated_state_input_handle_type_tokens(
    ty: &Type,
    program_lifetime: &syn::Lifetime,
    scope_lifetime: &syn::Lifetime,
) -> syn::Result<proc_macro2::TokenStream> {
    reject_known_secret_type(ty)?;
    let Type::Path(type_path) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "state input fields must lift to handles from MfmValue, Vec<T>, NonEmpty<T>, MaybeValue<T>, ArtifactRef<T>, or nested StateInput structs",
        ));
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(ty, "unsupported empty type path"));
    };
    let ident = segment.ident.to_string();
    match ident.as_str() {
        "Vec" => {
            let element = one_generic_type(segment, "Vec")?;
            reject_unsupported_state_input_leaf(element)?;
            Ok(
                quote!(::std::vec::Vec<::mfm_program::Handle<#program_lifetime, #scope_lifetime, #element>>),
            )
        }
        "NonEmpty" => {
            let element = one_generic_type(segment, "NonEmpty")?;
            reject_unsupported_state_input_leaf(element)?;
            Ok(quote!(::mfm_program::NonEmptyHandles<#program_lifetime, #scope_lifetime, #element>))
        }
        "Option" => Err(syn::Error::new_spanned(
            ty,
            "state input optionality must use mfm_values::MaybeValue<T>",
        )),
        _ if ident.ends_with("Input") => {
            let mut handles_path = type_path.path.clone();
            let last = handles_path
                .segments
                .iter_mut()
                .last()
                .expect("last segment exists");
            last.ident = format_ident!("{}Handles", last.ident);
            last.arguments = PathArguments::AngleBracketed(
                syn::parse_quote!(<#program_lifetime, #scope_lifetime>),
            );
            Ok(quote!(#handles_path))
        }
        _ => {
            reject_unsupported_state_input_leaf(ty)?;
            Ok(quote!(::mfm_program::Handle<#program_lifetime, #scope_lifetime, #ty>))
        }
    }
}

fn reject_unsupported_state_input_leaf(ty: &Type) -> syn::Result<()> {
    let Type::Path(type_path) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "state input fields must use named typed values",
        ));
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(ty, "unsupported empty type path"));
    };
    let ident = segment.ident.to_string();
    match ident.as_str() {
        "bool" | "String" | "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
        | "NonZeroU64" | "f32" | "f64" | "usize" | "isize" | "Option" | "HashMap" | "BTreeMap" => {
            Err(syn::Error::new_spanned(
                ty,
                "state input fields must use MfmValue types or supported typed wrappers",
            ))
        }
        "Value"
            if path_contains(&type_path.path, "serde_json")
                || type_path.path.segments.len() == 1 =>
        {
            Err(syn::Error::new_spanned(
                ty,
                "state input fields must use MfmValue types or supported typed wrappers",
            ))
        }
        _ => Ok(()),
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
