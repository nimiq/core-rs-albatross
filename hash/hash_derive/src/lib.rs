use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(SerializeContent)]
pub fn derive_serialize(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(impl_serialize_content(&ast))
}

fn impl_serialize_content(ast: &DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, ty_generics, _) = ast.generics.split_for_impl();

    let gen = quote! {
        impl #impl_generics ::nimiq_hash::SerializeContent for #name #ty_generics where #name #ty_generics: ::nimiq_hash::nimiq_serde::Serialize {
            #[allow(unused_mut, unused_variables)]
            fn serialize_content<W: ::std::io::Write, H>(&self, writer: &mut W) -> ::std::io::Result<()> {
                ::nimiq_hash::nimiq_serde::Serialize::serialize_to_writer(self, writer)?;
                Ok(())
            }
        }
    };
    gen
}
