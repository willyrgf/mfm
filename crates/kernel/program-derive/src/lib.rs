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

#[path = "attributes.rs"]
mod attributes;
#[path = "fact.rs"]
mod fact;
use self::attributes::{ContainerAttrs, FieldAttrs, VariantAttrs};
#[path = "program.rs"]
mod program;
use self::program::{
    expand_program_operation_output_derive_result, expand_program_public_outputs_derive_result,
    generated_state_input_handles_tokens,
};
#[path = "shape.rs"]
mod shape;
use self::shape::{schema_shape_tokens, DeriveKind};

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
    fact::derive(input)
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
