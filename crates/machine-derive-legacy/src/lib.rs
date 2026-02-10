extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{
    bracketed, parse_macro_input, Data, DeriveInput, Fields, Ident, ItemStruct, LitStr, Token,
};

#[proc_macro_derive(StateMetadataReqs)]
pub fn state_reqs_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Validate input early so consumers get a clear compile-time error instead of
    // "no field `...` on type ...".
    let named_fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => &fields_named.named,
            Fields::Unnamed(_) | Fields::Unit => {
                return syn::Error::new_spanned(
                    ident,
                    "StateMetadataReqs can only be derived for a struct with named fields: \
                     label, tags, depends_on, depends_on_strategy",
                )
                .to_compile_error()
                .into();
            }
        },
        Data::Enum(_) | Data::Union(_) => {
            return syn::Error::new_spanned(
                ident,
                "StateMetadataReqs can only be derived for a struct with named fields: \
                 label, tags, depends_on, depends_on_strategy",
            )
            .to_compile_error()
            .into();
        }
    };

    let required = ["label", "tags", "depends_on", "depends_on_strategy"];
    let mut missing: Vec<&'static str> = Vec::new();
    for req in required {
        let found = named_fields
            .iter()
            .any(|field| field.ident.as_ref().map(|id| id == req).unwrap_or(false));
        if !found {
            missing.push(req);
        }
    }

    if !missing.is_empty() {
        return syn::Error::new_spanned(
            ident,
            format!(
                "StateMetadataReqs requires the following named fields: label, tags, depends_on, depends_on_strategy. Missing: {}",
                missing.join(", ")
            ),
        )
        .to_compile_error()
        .into();
    }

    let expanded = quote! {
        impl #impl_generics StateMetadata for #ident #ty_generics #where_clause {
            fn label(&self) -> Label {
                self.label.clone()
            }

            fn tags(&self) -> Vec<Tag> {
                self.tags.clone()
            }

            fn depends_on(&self) -> Vec<Tag> {
                self.depends_on.clone()
            }

            fn depends_on_strategy(&self) -> DependencyStrategy {
                self.depends_on_strategy.clone()
            }
        }
    };

    TokenStream::from(expanded)
}

#[derive(Clone, Copy)]
enum DependencyStrategyArg {
    Latest,
    Earliest,
    LatestSuccessful,
}

struct StateHandlerArgs {
    label: LitStr,
    tags: Vec<LitStr>,
    depends_on: Vec<LitStr>,
    strategy: DependencyStrategyArg,
}

fn parse_lit_str(input: ParseStream<'_>) -> syn::Result<LitStr> {
    input.parse()
}

impl Parse for StateHandlerArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut label: Option<LitStr> = None;
        let mut tags: Option<Vec<LitStr>> = None;
        let mut depends_on: Option<Vec<LitStr>> = None;
        let mut strategy: Option<DependencyStrategyArg> = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "label" => {
                    label = Some(input.parse()?);
                }
                "tags" => {
                    let content;
                    bracketed!(content in input);
                    let elems = content.parse_terminated::<LitStr, Token![,]>(parse_lit_str)?;
                    tags = Some(elems.into_iter().collect());
                }
                "depends_on" => {
                    let content;
                    bracketed!(content in input);
                    let elems = content.parse_terminated::<LitStr, Token![,]>(parse_lit_str)?;
                    depends_on = Some(elems.into_iter().collect());
                }
                "strategy" => {
                    let ident: Ident = input.parse()?;
                    let parsed = match ident.to_string().as_str() {
                        "Latest" => DependencyStrategyArg::Latest,
                        "Earliest" => DependencyStrategyArg::Earliest,
                        "LatestSuccessful" => DependencyStrategyArg::LatestSuccessful,
                        other => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!(
                                    "invalid strategy '{other}'; expected one of: Latest, Earliest, LatestSuccessful"
                                ),
                            ));
                        }
                    };
                    strategy = Some(parsed);
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        key,
                        format!(
                            "unknown argument '{other}'; expected: label, tags, depends_on, strategy"
                        ),
                    ));
                }
            }

            if input.peek(Token![,]) {
                let _ = input.parse::<Token![,]>()?;
            }
        }

        let label =
            label.ok_or_else(|| syn::Error::new(input.span(), "missing required arg: label"))?;

        Ok(Self {
            label,
            tags: tags.unwrap_or_default(),
            depends_on: depends_on.unwrap_or_default(),
            strategy: strategy.unwrap_or(DependencyStrategyArg::Latest),
        })
    }
}

#[proc_macro_attribute]
pub fn state_handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as StateHandlerArgs);
    let input = parse_macro_input!(item as ItemStruct);

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &input.generics,
            "state_handler does not support generics (yet)",
        )
        .to_compile_error()
        .into();
    }

    if !matches!(input.fields, Fields::Unit) {
        return syn::Error::new_spanned(
            &input.fields,
            "state_handler must be applied to a unit struct: `struct MyState;`",
        )
        .to_compile_error()
        .into();
    }

    let vis = &input.vis;
    let ident = &input.ident;
    let attrs = &input.attrs;

    let label = args.label;
    let tags = args.tags;
    let depends_on = args.depends_on;

    let strategy = match args.strategy {
        DependencyStrategyArg::Latest => {
            quote! { ::mfm_machine_legacy::state::DependencyStrategy::Latest }
        }
        DependencyStrategyArg::Earliest => {
            quote! { ::mfm_machine_legacy::state::DependencyStrategy::Earliest }
        }
        DependencyStrategyArg::LatestSuccessful => {
            quote! { ::mfm_machine_legacy::state::DependencyStrategy::LatestSuccessful }
        }
    };

    let expanded = quote! {
        #(#attrs)*
        #vis struct #ident {
            label: ::mfm_machine_legacy::state::Label,
            tags: ::std::vec::Vec<::mfm_machine_legacy::state::Tag>,
            depends_on: ::std::vec::Vec<::mfm_machine_legacy::state::Tag>,
            depends_on_strategy: ::mfm_machine_legacy::state::DependencyStrategy,
        }

        impl #ident {
            pub fn new() -> Self {
                Self {
                    label: ::mfm_machine_legacy::state::Label::new_unchecked(#label),
                    tags: vec![#(::mfm_machine_legacy::state::Tag::new_unchecked(#tags)),*],
                    depends_on: vec![#(::mfm_machine_legacy::state::Tag::new_unchecked(#depends_on)),*],
                    depends_on_strategy: #strategy,
                }
            }
        }

        impl ::std::default::Default for #ident {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ::mfm_machine_legacy::state::StateMetadata for #ident {
            fn label(&self) -> ::mfm_machine_legacy::state::Label {
                self.label.clone()
            }

            fn tags(&self) -> ::std::vec::Vec<::mfm_machine_legacy::state::Tag> {
                self.tags.clone()
            }

            fn depends_on(&self) -> ::std::vec::Vec<::mfm_machine_legacy::state::Tag> {
                self.depends_on.clone()
            }

            fn depends_on_strategy(&self) -> ::mfm_machine_legacy::state::DependencyStrategy {
                self.depends_on_strategy
            }
        }
    };

    TokenStream::from(expanded)
}
