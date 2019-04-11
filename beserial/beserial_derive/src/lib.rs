#![recursion_limit = "128"]

extern crate proc_macro;

use proc_macro2::{TokenStream, Span};
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Ident, Index, Meta};

// This will return a tuple once we have more options
fn parse_field_attribs(field: &syn::Field) -> (Option<syn::Ident>, Option<Option<syn::Lit>>, bool) {
    let mut len_type = Option::None;
    let mut skip = Option::None;
    let mut uvar = false;
    for attr in &field.attrs {
        if let Meta::List(ref meta_list) = attr.parse_meta().unwrap() {
            if meta_list.ident == "beserial" {
                for nested in meta_list.nested.iter() {
                    if let syn::NestedMeta::Meta(ref item) = nested {
                        match item {
                            Meta::List(ref meta_list) => {
                                if meta_list.ident == "len_type" {
                                    for nested in meta_list.nested.iter() {
                                        if let syn::NestedMeta::Meta(ref item) = nested {
                                            if let Meta::Word(value) = item {
                                                if value != "u8" && value != "u16" && value != "u32" {
                                                    panic!("beserial(len_type) must be one of [u8, u16, u32], but was {}", value);
                                                }
                                                len_type = Option::Some(value.clone());
                                            }
                                        }
                                    }
                                }
                                if meta_list.ident == "skip" {
                                    skip = Option::Some(Option::None);
                                    for nested in meta_list.nested.iter() {
                                        if let syn::NestedMeta::Meta(ref item) = nested {
                                            if let Meta::NameValue(meta_name_value) = item {
                                                if meta_name_value.ident == "default" {
                                                    skip = Option::Some(Option::Some(meta_name_value.lit.clone()));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Meta::Word(ref attr_ident) => {
                                if attr_ident == "skip" {
                                    skip = Option::Some(Option::None);
                                } else if attr_ident == "uvar" {
                                    uvar = true;
                                } else {
                                    panic!("unknown flag for beserial: {}", attr_ident)
                                }
                            }
                            _ => panic!("unknown attribute for beserial: {:?}", item)
                        }
                    }
                }
            }
        }
    }
    return (len_type, skip, uvar);
}

fn parse_enum_attribs(ast: &syn::DeriveInput) -> (Option<syn::Ident>, bool) {
    let mut enum_type: Option<syn::Ident> = Option::None;
    let mut uvar = false;
    for attr in &ast.attrs {
        if let Meta::List(ref meta_list) = attr.parse_meta().unwrap() {
            if meta_list.ident == "repr" {
                enum_type = meta_list.nested.first().map_or(Option::None, |n| {
                    if let syn::NestedMeta::Meta(Meta::Word(ref meta_type)) = n.value() { Option::Some(meta_type.clone()) } else { Option::None }
                })
            } else if meta_list.ident == "beserial" {
                for nested in meta_list.nested.iter() {
                    if let syn::NestedMeta::Meta(ref item) = nested {
                        if let Meta::Word(ref attr_ident) = item {
                            if attr_ident == "uvar" {
                                uvar = true;
                            } else {
                                panic!("unknown flag for beserial: {}", attr_ident)
                            }
                        }
                    }
                }
            }
        }
    }
    return (enum_type, uvar);
}

fn expr_from_value(value: u64) -> syn::Expr {
    let lit_int = syn::LitInt::new(value, syn::IntSuffix::None, Span::call_site());
    let expr_lit = syn::ExprLit{ attrs: vec!(), lit: syn::Lit::Int(lit_int)};
    syn::Expr::from(expr_lit)
}

#[proc_macro_derive(Serialize, attributes(beserial))]
pub fn derive_serialize(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(impl_serialize(&ast))
}

fn impl_serialize(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let mut serialize_body = Vec::<TokenStream>::new();
    let mut serialized_size_body = Vec::<TokenStream>::new();

    match ast.data {
        Data::Enum(_) => {
            let (enum_type, uvar) = parse_enum_attribs(ast);

            if uvar {
                let ty = enum_type.unwrap_or_else(|| Ident::new("u64", Span::call_site()));
                serialize_body.push(quote! { size += Serialize::serialize(&::beserial::uvar::from(*self as #ty), writer)?; });
                serialized_size_body.push(quote! { size += Serialize::serialized_size(&::beserial::uvar::from(*self as #ty)); });
            } else {
                let ty = enum_type.expect(format!("Serialize can not be derived for enum {} without repr(u*) or repr(i*)", name).as_str());
                serialize_body.push(quote! { size += Serialize::serialize(&(*self as #ty), writer)?; });
                serialized_size_body.push(quote! { size += Serialize::serialized_size(&(*self as #ty)); });
            }
        }
        Data::Struct(ref data_struct) => {
            for (i, field) in data_struct.fields.iter().enumerate() {
                let (len_type, skip, _) = parse_field_attribs(&field);
                if skip.is_some() { continue; };
                match field.ident {
                    None => {
                        let index = Index::from(i);
                        match len_type {
                            Some(ty) => {
                                serialize_body.push(quote! { size += ::beserial::SerializeWithLength::serialize::<#ty, W>(&self.#index, writer)?; });
                                serialized_size_body.push(quote! { size += ::beserial::SerializeWithLength::serialized_size::<#ty>(&self.#index); });
                            }
                            None => {
                                serialize_body.push(quote! { size += Serialize::serialize(&self.#index, writer)?; });
                                serialized_size_body.push(quote! { size += Serialize::serialized_size(&self.#index); });
                            }
                        }
                    }
                    Some(ref ident) => {
                        match len_type {
                            Some(ty) => {
                                serialize_body.push(quote! { size += ::beserial::SerializeWithLength::serialize::<#ty, W>(&self.#ident, writer)?; });
                                serialized_size_body.push(quote! { size += ::beserial::SerializeWithLength::serialized_size::<#ty>(&self.#ident); });
                            }
                            None => {
                                serialize_body.push(quote! { size += Serialize::serialize(&self.#ident, writer)?; });
                                serialized_size_body.push(quote! { size += Serialize::serialized_size(&self.#ident); });
                            }
                        }
                    }
                }
            }
        }
        Data::Union(_) => panic!("Serialize can not be derived for Union {}", name)
    };

    let gen = quote! {
        impl #impl_generics Serialize for #name #ty_generics #where_clause {
            #[allow(unused_mut,unused_variables)]
            fn serialize<W: ::beserial::WriteBytesExt>(&self, writer: &mut W) -> Result<usize, ::beserial::SerializingError> {
                let mut size = 0;
                #(#serialize_body)*
                return Ok(size);
            }
            #[allow(unused_mut,unused_variables)]
            fn serialized_size(&self) -> usize {
                let mut size = 0;
                #(#serialized_size_body)*
                return size;
            }
        }
    };
    gen.into()
}

#[proc_macro_derive(Deserialize, attributes(beserial))]
pub fn derive_deserialize(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(impl_deserialize(&ast))
}

fn impl_deserialize(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let deserialize_body;

    match ast.data {
        Data::Enum(ref data_enum) => {
            let (enum_type, uvar) = parse_enum_attribs(ast);

            let ty;
            if uvar {
                ty = enum_type.unwrap_or(Ident::new("u64", syn::export::Span::call_site()));
            } else {
                ty = enum_type.expect(format!("Deserialize can not be derived for enum {} without repr(u*) or repr(i*)", name).as_str());
            }

            let mut num = expr_from_value(0);
            let mut num_cases = Vec::<TokenStream>::new();
            for variant in data_enum.variants.iter() {
                let ident = &variant.ident;
                num = match &variant.discriminant {
                    None => {
                        if let syn::Expr::Lit(ref expr_lit) = num {
                            if let syn::Lit::Int(lit_int) = &expr_lit.lit {
                                expr_from_value(lit_int.value() + 1)
                            } else {
                                panic!("non-integer discriminant");
                            }
                        } else {
                            panic!("non-literal discriminant");
                        }
                    },
                    Some((_, expr)) => expr.clone()
                };
                num_cases.push(quote! { #num => Ok(#name::#ident), });
            }

            if uvar {
                deserialize_body = quote! {
                    let u: uvar = Deserialize::deserialize(reader)?;
                    let num: u64 = u.into();
                    return match num {
                        #(#num_cases)*
                        _ => Err(::beserial::SerializingError::InvalidValue)
                    };
                };
            } else {
                deserialize_body = quote! {
                    let num: #ty = Deserialize::deserialize(reader)?;
                    return match num {
                        #(#num_cases)*
                        _ => Err(::beserial::SerializingError::InvalidValue)
                    };
                };
            }
        }
        Data::Struct(ref data_struct) => {
            let mut tuple = false;
            let mut field_cases = Vec::<TokenStream>::new();
            for field in data_struct.fields.iter() {
                let (len_type, skip, _) = parse_field_attribs(&field);
                match field.ident {
                    None => {
                        tuple = true;
                        if let Option::Some(Option::Some(default_value)) = skip {
                            field_cases.push(quote! { #default_value, });
                            continue;
                        } else if let Option::Some(Option::None) = skip {
                            let ty = &field.ty;
                            field_cases.push(quote! { <#ty>::default(), });
                            continue;
                        }
                        match len_type {
                            Some(ty) =>
                                field_cases.push(quote! { ::beserial::DeserializeWithLength::deserialize::<#ty,R>(reader)?, }),
                            None =>
                                field_cases.push(quote! { Deserialize::deserialize(reader)?, })
                        }
                    }
                    Some(ref ident) => {
                        if let Option::Some(Option::Some(default_value)) = skip {
                            field_cases.push(quote! { #ident: #default_value });
                            continue;
                        } else if let Option::Some(Option::None) = skip {
                            let ty = &field.ty;
                            field_cases.push(quote! { #ident: <#ty>::default() });
                            continue;
                        }
                        match len_type {
                            Some(ty) => {
                                field_cases.push(quote! { #ident: ::beserial::DeserializeWithLength::deserialize::<#ty,R>(reader)?, })
                            }
                            None => {
                                field_cases.push(quote! { #ident: Deserialize::deserialize(reader)?, })
                            }
                        }
                    }
                }
            }

            if tuple {
                deserialize_body = quote!({
                    return Ok(#name (
                        #(#field_cases)*
                    ));
                });
            } else {
                deserialize_body = quote!({
                    return Ok(#name {
                        #(#field_cases)*
                    });
                });
            }
        }
        Data::Union(_) => panic!("Deserialize can not be derived for Union {}", name)
    };

    let gen = quote! {
        impl #impl_generics Deserialize for #name #ty_generics #where_clause {
            #[allow(unused_mut,unused_variables)]
            fn deserialize<R: ::beserial::ReadBytesExt>(reader: &mut R) -> Result<Self, ::beserial::SerializingError> {
                #deserialize_body
            }
        }
    };
    gen.into()
}
