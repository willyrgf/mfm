#![warn(missing_docs)]
//! Derive macros for MFM typed program contracts.
//!
//! This crate generates descriptor implementations for the typed kernel. It
//! does not generate runtime scheduling logic.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::parse_macro_input;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DataEnum, DataStruct, DeriveInput, Fields, FieldsNamed, FieldsUnnamed,
    GenericArgument, GenericParam, Ident, LitStr, PathArguments, Type, TypePath, Variant,
};

#[path = "attributes.rs"]
mod attributes;
use self::attributes::{ContainerAttrs, FieldAttrs, VariantAttrs};
#[path = "shape.rs"]
mod shape;
use self::shape::{schema_shape_tokens, DeriveKind};

mod context;

/// Derives typed field borrowing and replacement markers for a named context.
///
/// Requires `#[context(namespace = "...")]`. Each type parameter must occur as
/// exactly one bare field type, without bounds, defaults, or a where clause.
#[proc_macro_derive(MfmContext, attributes(context))]
pub fn derive_mfm_context(input: TokenStream) -> TokenStream {
    context::expand(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

#[proc_macro_derive(MfmValue, attributes(mfm, serde))]
/// Derives `mfm_values::MfmValue` for a named struct.
pub fn derive_mfm_value(input: TokenStream) -> TokenStream {
    expand_schema_derive(parse_macro_input!(input as DeriveInput), DeriveKind::Value).into()
}

#[proc_macro_derive(PersistedSchema, attributes(mfm, serde))]
/// Derives `mfm_values::PersistedSchema` for a retained owner type.
///
/// The shape follows the Rust definition, so a retained contract cannot drift
/// from the bytes it serializes. Checked identity fields map to their closed
/// grammar and nested persisted owners embed their own declared shape.
pub fn derive_persisted_schema(input: TokenStream) -> TokenStream {
    expand_schema_derive(
        parse_macro_input!(input as DeriveInput),
        DeriveKind::PersistedContract,
    )
    .into()
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
    let generic_params = input
        .generics
        .params
        .iter()
        .map(|parameter| match parameter {
            GenericParam::Type(parameter) => Ok(parameter.ident.clone()),
            other => Err(syn::Error::new_spanned(
                other,
                "MFM derives support only type parameters in v1",
            )),
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let mut impl_generics = input.generics.clone();
    for parameter in &mut impl_generics.params {
        if let GenericParam::Type(parameter) = parameter {
            parameter
                .bounds
                .push(syn::parse_quote!(::mfm_values::MfmValue));
        }
    }
    let (impl_generics, ty_generics, where_clause) = impl_generics.split_for_impl();

    let attrs = ContainerAttrs::parse(&input.attrs, &input.ident)?;
    let shape_output = schema_shape_tokens(
        &input.data,
        attrs.rename_all.as_deref(),
        kind,
        &attrs,
        &generic_params,
    )?;
    let shape = shape_output.shape;
    let default_bounds = shape_output.default_bounds;
    if let Data::Union(union) = &input.data {
        return Err(syn::Error::new_spanned(
            union.union_token,
            "MFM derives do not support unions",
        ));
    }
    let ident = &input.ident;
    let schema_name = attrs.schema_name;
    let version = attrs.version;
    let schema_kind = kind.schema_kind_tokens();
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

    let identity_body = quote! {
        #(#default_bounds)*

        ::mfm_values::SchemaIdentity::new(
            #schema_kind,
            #semantic_identity,
            #schema_name,
            ::mfm_ids::SchemaVersion::new(#version)
                .map_err(|error| ::mfm_values::ValueError::Identity(error.to_string()))?,
            #shape,
        )
    };

    let impl_block = match kind {
        DeriveKind::PersistedContract => quote! {
            impl #impl_generics ::mfm_values::PersistedSchema for #ident #ty_generics #where_clause {
                fn schema_identity() -> ::mfm_values::Result<::mfm_values::SchemaIdentity> {
                    #identity_body
                }

                fn validate(&self) -> ::mfm_values::Result<()> {
                    let identity =
                        <Self as ::mfm_values::PersistedSchema>::schema_identity()?;
                    ::mfm_values::validate_derived_persisted_owner(self, &identity)
                }
            }
        },
        DeriveKind::Value => quote! {
            impl #impl_generics ::mfm_values::MfmValue for #ident #ty_generics #where_clause {
                #semantic_impl

                fn schema_descriptor() -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    #descriptor_body
                }
            }
        },
    };

    Ok(impl_block)
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
    let text = quote!(#ty).to_string().replace("SecretFree", "");
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
        None => value.to_owned(),
        Some("snake_case") => snake_case(value),
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
