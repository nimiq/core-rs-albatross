use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(DbSerializable)]
pub fn derive_serialize(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(impl_db_serialize(&ast))
}

fn impl_db_serialize(ast: &DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, ty_generics, _) = ast.generics.split_for_impl();

    let gen = quote! {
        impl #impl_generics ::nimiq_database_value::AsDatabaseBytes for #name #ty_generics where #name #ty_generics: ::nimiq_hash::nimiq_serde::Serialize {
            fn as_key_bytes(&self) -> std::borrow::Cow<[u8]> {
                std::borrow::Cow::Owned(::nimiq_hash::nimiq_serde::Serialize::serialize_to_vec(&self))
            }
        }

        impl #impl_generics ::nimiq_database_value::FromDatabaseBytes for #name #ty_generics where #name #ty_generics: ::nimiq_hash::nimiq_serde::Deserialize {
            fn from_key_bytes(bytes: &[u8]) -> Self {
                ::nimiq_hash::nimiq_serde::Deserialize::deserialize_from_vec(bytes).unwrap()
            }
        }

        impl #impl_generics ::nimiq_database_value::IntoDatabaseValue for #name #ty_generics where #name #ty_generics: ::nimiq_hash::nimiq_serde::Deserialize {
            fn database_byte_size(&self) -> usize {
                ::nimiq_hash::nimiq_serde::Serialize::serialized_size(&self)
            }

            fn copy_into_database(&self, mut bytes: &mut [u8]) {
                ::nimiq_hash::nimiq_serde::Serialize::serialize(&self, &mut bytes).unwrap();
            }
        }
    };
    gen
}
