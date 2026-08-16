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
    let match_projection_impl = if kind == DeriveKind::Value {
        match &input.data {
            Data::Enum(data) => enum_match_projection_tokens(data, attrs.rename_all.as_deref())?,
            Data::Struct(_) => quote! {},
            Data::Union(union) => {
                return Err(syn::Error::new_spanned(
                    union.union_token,
                    "MFM derives do not support unions",
                ));
            }
        }
    } else {
        quote! {}
    };
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
            impl #impl_generics #ident #ty_generics #where_clause {
                fn __mfm_persisted_schema_identity(
                ) -> ::mfm_values::Result<&'static ::mfm_values::SchemaIdentity> {
                    static IDENTITY: ::std::sync::OnceLock<
                        ::std::result::Result<::mfm_values::SchemaIdentity, ::std::string::String>,
                    > = ::std::sync::OnceLock::new();
                    match IDENTITY.get_or_init(|| {
                            (|| -> ::mfm_values::Result<::mfm_values::SchemaIdentity> {
                                #identity_body
                            })()
                            .map_err(|error| error.to_string())
                        }) {
                        Ok(identity) => Ok(identity),
                        Err(error) => Err(::mfm_values::ValueError::Descriptor(error.clone())),
                    }
                }
            }

            impl #impl_generics ::mfm_values::PersistedSchema for #ident #ty_generics #where_clause {
                fn schema_identity() -> ::mfm_values::Result<::mfm_values::SchemaIdentity> {
                    Self::__mfm_persisted_schema_identity().cloned()
                }

                fn schema_shape() -> ::mfm_values::Result<::mfm_values::SchemaShape> {
                    Self::__mfm_persisted_schema_identity()?
                        .canonical_json_shape()
                        .cloned()
                }

                fn validate_canonical_bytes(bytes: &[u8]) -> ::mfm_values::Result<()> {
                    Self::__mfm_persisted_schema_identity()?
                        .validate_canonical_value_for_prevalidated_owner(bytes)
                }

                fn schema_id() -> ::mfm_values::Result<::mfm_ids::SchemaId> {
                    // The derived identity is constant, so its schema id is too.
                    // Retained content references are derived inside recursive
                    // program and history walks; recanonicalizing the whole
                    // shape per reference would be both quadratic and stack
                    // hungry there.
                    static SCHEMA_ID: ::std::sync::OnceLock<
                        ::std::result::Result<::mfm_ids::SchemaId, ::std::string::String>,
                    > = ::std::sync::OnceLock::new();
                    SCHEMA_ID
                        .get_or_init(|| {
                            Self::__mfm_persisted_schema_identity()
                                .and_then(|identity| identity.schema_id())
                                .map_err(|error| error.to_string())
                        })
                        .clone()
                        .map_err(::mfm_values::ValueError::Descriptor)
                }

                fn validate(&self) -> ::mfm_values::Result<()> {
                    ::mfm_values::validate_derived_persisted_owner_prevalidated(
                        self,
                        Self::__mfm_persisted_schema_identity()?,
                    )
                }
            }
        },
        DeriveKind::Value => quote! {
            impl #impl_generics ::mfm_values::MfmValue for #ident #ty_generics #where_clause {
                #semantic_impl

                fn schema_descriptor() -> ::mfm_values::Result<::mfm_values::SchemaDescriptor> {
                    #descriptor_body
                }

                #match_projection_impl
            }
        },
    };

    Ok(impl_block)
}

fn enum_match_projection_tokens(
    data: &DataEnum,
    rename_all: Option<&str>,
) -> syn::Result<proc_macro2::TokenStream> {
    let mut arms = Vec::new();
    for variant in &data.variants {
        let attrs = VariantAttrs::parse(&variant.attrs)?;
        let wire_name = attrs
            .rename
            .unwrap_or_else(|| apply_rename_all(&variant.ident.to_string(), rename_all));
        let ident = &variant.ident;
        if let Fields::Unnamed(fields) = &variant.fields {
            let payload = fields.unnamed.first().and_then(|field| {
                if fields.unnamed.len() != 1 {
                    None
                } else if is_inline_value_type(&field.ty, &[]) {
                    Some(proc_macro2::TokenStream::new())
                } else if let Some(depth) = boxed_inline_value_depth(&field.ty) {
                    let unbox = (0..depth).map(|_| quote!(let payload = *payload;));
                    Some(quote!(#(#unbox)*))
                } else {
                    None
                }
            });
            if let Some(unbox) = payload {
                arms.push(quote! {
                    Self::#ident(payload) => {
                        #unbox
                        Some(visitor.visit(#wire_name, payload))
                    }
                });
            }
        }
    }
    let supported = arms.len() == data.variants.len();
    let fallback = (!supported).then(|| quote!(_ => None,));
    Ok(quote! {
        const __MFM_MATCH_PROJECTION_SUPPORTED: bool = #supported;

        fn __mfm_visit_match_payload<V: ::mfm_values::MatchPayloadVisitor>(
            self,
            visitor: V,
        ) -> ::std::option::Option<V::Output> {
            match self {
                #(#arms)*
                #fallback
            }
        }
    })
}

fn boxed_inline_value_depth(ty: &Type) -> Option<usize> {
    let mut depth = 0_usize;
    let mut inner = ty;
    loop {
        let Type::Path(path) = inner else {
            return None;
        };
        let segment = path.path.segments.last()?;
        if segment.ident != "Box" {
            return (depth > 0 && is_inline_value_type(inner, &[])).then_some(depth);
        }
        depth = depth.checked_add(1)?;
        inner = one_generic_type(segment, "Box").ok()?;
    }
}

fn is_inline_value_type(ty: &Type, generic_params: &[Ident]) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    if path.path.segments.len() == 1
        && generic_params
            .iter()
            .any(|parameter| parameter == &segment.ident)
    {
        return true;
    }
    !matches!(
        segment.ident.to_string().as_str(),
        "bool"
            | "String"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "NonZeroU16"
            | "NonZeroU32"
            | "NonZeroU64"
            | "ContentRef"
            | "ContentDigest"
            | "SchemaId"
            | "SemanticTypeId"
            | "RunId"
            | "ArtifactId"
            | "StableId"
            | "EntryPointId"
            | "Box"
            | "Option"
            | "Vec"
            | "NonEmpty"
            | "BTreeMap"
    )
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
