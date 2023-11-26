extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(StateMetadataReqs)]
pub fn state_reqs_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = &input.ident;

    let expanded = quote! {
        impl StateMetadata for #ident {
            fn label(&self) -> Label {
                self.label
            }

            fn tags(&self) -> Vec<Tag> {
                self.tags.clone()
            }

            fn depends_on(&self) -> Vec<Tag> {
                self.depends_on.clone()
            }

            fn depends_on_strategy(&self) -> DependencyStrategy {
                self.depends_on_strategy
            }
        }
    };

    TokenStream::from(expanded)
}
