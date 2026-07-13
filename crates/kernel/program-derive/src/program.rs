use super::*;

pub(super) fn expand_program_public_outputs_derive_result(
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
    let field_output = program_output_field_tokens(
        fields,
        attrs.rename_all.as_deref(),
        program_lifetime,
        scope_lifetime,
        ProgramOutputKind::Public,
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

pub(super) fn expand_program_operation_output_derive_result(
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
    let field_output = program_output_field_tokens(
        fields,
        attrs.rename_all.as_deref(),
        program_lifetime,
        scope_lifetime,
        ProgramOutputKind::Operation,
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

struct ProgramPublicOutputFieldOutput {
    descriptors: Vec<proc_macro2::TokenStream>,
    output_cells: Vec<proc_macro2::TokenStream>,
}

#[derive(Debug, Clone, Copy)]
enum ProgramOutputKind {
    Public,
    Operation,
}

impl ProgramOutputKind {
    fn default_field_error(self) -> &'static str {
        match self {
            Self::Public => "program public output handle fields cannot use serde(default)",
            Self::Operation => "program operation output handle fields cannot use serde(default)",
        }
    }
}

fn program_output_field_tokens(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    rename_all: Option<&str>,
    program_lifetime: &syn::Lifetime,
    scope_lifetime: &syn::Lifetime,
    kind: ProgramOutputKind,
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
            return Err(syn::Error::new(ident.span(), kind.default_field_error()));
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
        output_cells.push(match kind {
            ProgramOutputKind::Public => quote! {
                ::mfm_program::PublicOutputCellSpec::from_handle(
                    ::mfm_program::PublicFieldPath::new(#wire_name)?,
                    &self.#ident,
                )
            },
            ProgramOutputKind::Operation => quote!(self.#ident.typed_ref()),
        });
    }

    Ok(ProgramPublicOutputFieldOutput {
        descriptors,
        output_cells,
    })
}

pub(super) fn generated_state_input_handles_tokens(
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

    let handle_definition = if handle_fields.is_empty() {
        quote! {
            #[derive(Clone)]
            #[allow(missing_docs)]
            pub struct #handle_ident {}
        }
    } else {
        quote! {
            #[derive(Clone)]
            #[allow(missing_docs)]
            pub struct #handle_ident<#program_lifetime, #scope_lifetime> {
                #(#handle_fields,)*
            }
        }
    };
    let handle_ty = if handle_fields.is_empty() {
        quote!(#handle_ident)
    } else {
        quote!(#handle_ident<#program_lifetime, #scope_lifetime>)
    };

    Ok(quote! {
        #handle_definition

        impl<#program_lifetime, #scope_lifetime> ::mfm_program::IntoInputBindingNode<#input_ident>
            for #handle_ty
        {
            fn into_binding_node(
                self,
                field_path: ::mfm_program::InputFieldPath,
            ) -> ::mfm_program::Result<::mfm_program::InputBindingNode> {
                ::mfm_program::InputBindingNode::struct_fields(vec![#(#input_bindings),*])
            }
        }

        impl<#program_lifetime, #scope_lifetime> ::mfm_program::IntoStateInput<#program_lifetime, #scope_lifetime, #input_ident>
            for #handle_ty
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
