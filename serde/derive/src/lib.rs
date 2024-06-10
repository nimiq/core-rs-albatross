use darling::{
    ast::{Data, Fields},
    FromDeriveInput, FromField, FromVariant,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Expr, Generics, Ident, Type, TypeArray};

#[derive(FromDeriveInput)]
#[darling(attributes(serialize_size))]
struct Input {
    ident: Ident,
    generics: Generics,
    data: Data<Variant, Field>,
}

#[derive(FromField)]
#[darling(attributes(serialize_size))]
struct Field {
    ty: Type,
    seq_max_elems: Option<Expr>,
    bitset_max_elem: Option<Expr>,
}

#[derive(FromVariant)]
#[darling(attributes(serialize_size))]
struct Variant {
    fields: Fields<Field>,
}

fn size(ty: &Type) -> TokenStream {
    if let Type::Array(TypeArray { elem, len, .. }) = ty {
        let elem_size = size(elem);
        return quote! { (#elem_size) * (#len) };
    }
    quote! { <#ty as ::nimiq_serde::SerializedSize>::SIZE }
}

fn max_size(ty: &Type) -> TokenStream {
    quote! { <#ty as ::nimiq_serde::SerializedMaxSize>::MAX_SIZE }
}

fn size_fields(fields: &[Field], max: bool) -> TokenStream {
    let mut sum = quote! { 0 };
    for Field {
        ty,
        seq_max_elems,
        bitset_max_elem,
    } in fields
    {
        let size = match (seq_max_elems, bitset_max_elem) {
            (None, None) => {
                if max {
                    max_size(ty)
                } else {
                    size(ty)
                }
            }
            _ if !max => panic!(
                "`#[serialize_size]` attributes not supported while deriving `SerializedSize`"
            ),
            (Some(seq_max_elems), None) => quote! {
                ::nimiq_serde::seq_max_size(<#ty as ::nimiq_serde::SerializeSeqMaxSize>::Element::MAX_SIZE, #seq_max_elems)
            },
            (None, Some(bitset_max_elem)) => quote! {
                ::nimiq_collections::BitSet::max_size(#bitset_max_elem)
            },
            _ => panic!("`more than one #[serialize_size]` attribute specified"),
        };
        sum.extend(quote! { + #size });
    }
    sum
}

fn max_size_variants(variants: &[Variant]) -> TokenStream {
    let mut max = quote! { 0 };
    for Variant { fields } in variants {
        let size = size_fields(&fields.fields, true);
        max = quote! { ::nimiq_serde::max(#max, #size) };
    }
    let num_variants = u64::try_from(variants.len()).unwrap();
    quote! { ::nimiq_serde::uint_max_size(#num_variants) + #max }
}

#[proc_macro_derive(SerializedSize, attributes(serialize_size))]
pub fn derive_serialize_size(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let Input {
        ident,
        generics,
        data,
    } = Input::from_derive_input(&parse_macro_input!(input)).unwrap();
    let (impl_generics, ty_generics, _) = generics.split_for_impl();

    let size = match data {
        Data::Enum(_) => panic!("`SerializedSize` cannot be derived for enums"),
        Data::Struct(f) => size_fields(&f.fields, false),
    };

    quote! {
        impl #impl_generics ::nimiq_serde::SerializedSize for #ident #ty_generics {
            const SIZE: usize = #size;
        }
    }
    .into()
}

#[proc_macro_derive(SerializedMaxSize, attributes(serialize_size))]
pub fn derive_serialize_max_size(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let Input {
        ident,
        generics,
        data,
    } = Input::from_derive_input(&parse_macro_input!(input)).unwrap();
    let (impl_generics, ty_generics, _) = generics.split_for_impl();

    let size = match data {
        Data::Enum(v) => max_size_variants(&v),
        Data::Struct(f) => size_fields(&f.fields, true),
    };

    quote! {
        impl #impl_generics ::nimiq_serde::SerializedMaxSize for #ident #ty_generics {
            const MAX_SIZE: usize = #size;
        }
    }
    .into()
}
