extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(StateConfigReqs)]
pub fn state_reqs_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let ident = &input.ident;
    let expanded = quote! {
        impl StateConfig for #ident {
            fn label(&self) -> &Label {
                &self.label
            }

            fn tags(&self) -> &[Tag] {
                &self.tags
            }

            fn depends_on(&self) -> &[Tag] {
                &self.depends_on
            }

            fn depends_on_strategy(&self) -> &DependencyStrategy {
                &self.depends_on_strategy
            }
        }
    };

    TokenStream::from(expanded)
}
