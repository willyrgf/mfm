use std::collections::BTreeSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::visit::Visit;
use syn::{Data, DeriveInput, Fields, GenericParam, Ident, LitStr, Type};

struct ParameterUses<'a> {
    parameter: &'a Ident,
    count: usize,
    opaque: bool,
}

impl<'ast> Visit<'ast> for ParameterUses<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == *self.parameter)
        {
            self.count += 1;
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_type_macro(&mut self, _: &'ast syn::TypeMacro) {
        // An unexpanded type macro could hide another occurrence of a slot parameter.
        self.opaque = true;
    }
}

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let mut namespace = None;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("context"))
    {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("namespace") || namespace.is_some() {
                return Err(meta.error("expected one context namespace"));
            }
            namespace = Some(meta.value()?.parse::<LitStr>()?);
            Ok(())
        })?;
    }
    let namespace = namespace.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "MfmContext requires #[context(namespace = \"...\")]",
        )
    })?;
    if namespace.value().is_empty() {
        return Err(syn::Error::new_spanned(
            namespace,
            "context namespace must not be empty",
        ));
    }
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "MfmContext supports named structs only",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "MfmContext supports named structs only",
            ))
        }
    };
    if let Some(clause) = &input.generics.where_clause {
        return Err(syn::Error::new_spanned(
            clause,
            "context slot parameters cannot have a where clause",
        ));
    }
    let parameters = input.generics.params.iter().map(|parameter| match parameter {
        GenericParam::Type(parameter) if parameter.bounds.is_empty() && parameter.default.is_none() => Ok(parameter.ident.clone()),
        _ => Err(syn::Error::new_spanned(parameter, "context slots require unbounded type parameters without defaults; lifetimes and const parameters are unsupported")),
    }).collect::<syn::Result<Vec<_>>>()?;
    let mut replacement = format_ident!("__MfmContextValue");
    while parameters.contains(&replacement) {
        replacement = format_ident!("{}_", replacement);
    }
    let ident = &input.ident;
    let visibility = &input.vis;
    let mut markers = BTreeSet::new();
    let mut output = TokenStream::new();
    for parameter in &parameters {
        let mut uses = ParameterUses {
            parameter,
            count: 0,
            opaque: false,
        };
        for field in fields {
            uses.visit_type(&field.ty);
        }
        let selected = fields.iter().find(|field| matches!(&field.ty, Type::Path(path) if path.qself.is_none() && path.path.is_ident(parameter)));
        let Some(selected) = selected.filter(|_| uses.count == 1 && !uses.opaque) else {
            return Err(syn::Error::new_spanned(parameter, "each context slot parameter must occur exactly once as a bare field type; nested, repeated, and opaque uses are unsupported"));
        };
        let field = selected.ident.as_ref().expect("named field");
        let field_name = field.to_string();
        let field_name = field_name.trim_start_matches("r#");
        let pascal: String = field_name
            .split('_')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                let first = chars.next().expect("nonempty part");
                first.to_uppercase().chain(chars).collect::<String>()
            })
            .collect();
        let marker = format_ident!(
            "{}{}Slot",
            ident.to_string().trim_start_matches("r#"),
            pascal
        );
        if !markers.insert(marker.to_string()) {
            return Err(syn::Error::new_spanned(
                field,
                "context field names produce colliding slot markers",
            ));
        }
        let slot_id = format!("{}/slot/{}", namespace.value(), field_name);
        let replaced = parameters
            .iter()
            .map(|p| if p == parameter { &replacement } else { p });
        let siblings = fields
            .iter()
            .filter_map(|sibling| sibling.ident.as_ref())
            .map(|sibling| {
                if sibling == field {
                    quote!(#sibling: value)
                } else {
                    quote!(#sibling: context.#sibling)
                }
            });
        let doc = format!("Typed access to `{ident}::{field}`.");
        output.extend(quote! {
            #[doc = #doc]
            #visibility struct #marker;
            impl<#(#parameters: ::mfm_values::MfmValue),*> ::mfm_values::ContextSlot<#ident<#(#parameters),*>> for #marker {
                type Value = #parameter;
                type With<#replacement: ::mfm_values::MfmValue> = #ident<#(#replaced),*>;
                fn get(context: &#ident<#(#parameters),*>) -> &Self::Value { &context.#field }
                fn replace<#replacement: ::mfm_values::MfmValue>(context: #ident<#(#parameters),*>, value: #replacement) -> Self::With<#replacement> {
                    #ident { #(#siblings),* }
                }
                fn slot_id() -> ::mfm_values::Result<::mfm_ids::StableId> {
                    ::mfm_ids::StableId::new(#slot_id).map_err(|_| ::mfm_values::ValueError::Identity("invalid context slot identity".to_owned()))
                }
            }
        });
    }
    Ok(output)
}
