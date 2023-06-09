#![recursion_limit = "128"]

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(SerializeContent)]
pub fn derive_serialize(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(impl_serialize_content(&ast))
}

fn impl_serialize_content(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;

    let gen = quote! {
        impl SerializeContent for #name where #name: Serialize {
            #[allow(unused_mut,unused_variables)]
            fn serialize_content<W: ::std::io::Write, H>(&self, writer: &mut W) -> ::std::io::Result<usize> {
                self.serialize_to_writer(writer)
            }
        }
    };
    gen
}
