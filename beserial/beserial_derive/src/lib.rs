#![recursion_limit = "128"]

extern crate proc_macro;
#[macro_use]
extern crate quote;
extern crate syn;

use proc_macro::TokenStream;

// This will return a tuple once we have more options
fn parse_field_attribs(field: &syn::Field) -> (Option<&syn::Ident>, Option<Option<&syn::Lit>>, bool) {
    let mut len_type = Option::None;
    let mut skip = Option::None;
    let mut uvar = false;
    for attr in &field.attrs {
        if let syn::MetaItem::List(ref attr_ident, ref nesteds) = attr.value {
            if attr_ident == "beserial" {
                for nested in nesteds {
                    if let syn::NestedMetaItem::MetaItem(ref item) = nested {
                        match item {
                            syn::MetaItem::List(ref attr_ident, ref nesteds) => {
                                if attr_ident == "len_type" {
                                    for nested in nesteds {
                                        if let syn::NestedMetaItem::MetaItem(ref item) = nested {
                                            if let syn::MetaItem::Word(value) = item {
                                                if value != "u8" && value != "u16" && value != "u32" {
                                                    panic!("beserial(len_type) must be one of [u8, u16, u32], but was {}", value);
                                                }
                                                len_type = Option::Some(value);
                                            }
                                        }
                                    }
                                }
                                if attr_ident == "skip" {
                                    skip = Option::Some(Option::None);
                                    for nested in nesteds {
                                        if let syn::NestedMetaItem::MetaItem(ref item) = nested {
                                            if let syn::MetaItem::NameValue(name, value) = item {
                                                if name == "default" {
                                                    skip = Option::Some(Option::Some(value));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            syn::MetaItem::Word(ref attr_ident) => {
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
        if let syn::MetaItem::List(ref attr_ident, ref nesteds) = attr.value {
            if attr_ident == "repr" {
                enum_type = nesteds.iter().next().map_or(Option::None, |n| {
                    if let syn::NestedMetaItem::MetaItem(syn::MetaItem::Word(ref meta_type)) = n { Option::Some(meta_type.clone()) } else { Option::None }
                })
            } else if attr_ident == "beserial" {
                for nested in nesteds {
                    if let syn::NestedMetaItem::MetaItem(ref item) = nested {
                        if let syn::MetaItem::Word(ref attr_ident) = item {
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

#[proc_macro_derive(Serialize, attributes(beserial))]
pub fn derive_serialize(input: TokenStream) -> TokenStream {
    let s = input.to_string();
    let ast = syn::parse_derive_input(&s).unwrap();
    impl_serialize(&ast).parse().unwrap()
}

fn impl_serialize(ast: &syn::DeriveInput) -> quote::Tokens {
    let name = &ast.ident;

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let mut serialize_body = quote::Tokens::new();
    let mut serialized_size_body = quote::Tokens::new();

    match ast.body {
        syn::Body::Enum(_) => {
            let (enum_type, uvar) = parse_enum_attribs(ast);

            if uvar {
                let ty = enum_type.unwrap_or_else(|| syn::Ident::from("u64"));
                serialize_body.append(quote! { size += Serialize::serialize(&::beserial::uvar::from(*self as #ty), writer)?; });
                serialized_size_body.append(quote! { size += Serialize::serialized_size(&::beserial::uvar::from(*self as #ty)); });
            } else {
                let ty = enum_type.expect(format!("Serialize can not be derived for enum {} without repr(u*) or repr(i*)", name).as_str());
                serialize_body.append(quote! { size += Serialize::serialize(&(*self as #ty), writer)?; });
                serialized_size_body.append(quote! { size += Serialize::serialized_size(&(*self as #ty)); });
            }
        }
        syn::Body::Struct(ref variant) => {
            match variant {
                syn::VariantData::Struct(ref fields) => {
                    for field in fields {
                        let (len_type, skip, _) = parse_field_attribs(&field);
                        if skip.is_some() { continue; };
                        match field.ident {
                            None => panic!(),
                            Some(ref ident) => {
                                match len_type {
                                    Some(ty) => {
                                        serialize_body.append(quote! { size += ::beserial::SerializeWithLength::serialize::<#ty, W>(&self.#ident, writer)?; }.as_str());
                                        serialized_size_body.append(quote! { size += ::beserial::SerializeWithLength::serialized_size::<#ty>(&self.#ident); }.as_str());
                                    }
                                    None => {
                                        serialize_body.append(quote! { size += Serialize::serialize(&self.#ident, writer)?; }.as_str());
                                        serialized_size_body.append(quote! { size += Serialize::serialized_size(&self.#ident); }.as_str());
                                    }
                                }
                            }
                        }
                    }
                }
                syn::VariantData::Tuple(ref fields) => {
                    let mut i = 0;
                    for field in fields {
                        let (len_type, skip, _) = parse_field_attribs(&field);
                        if skip.is_some() { continue; };
                        match len_type {
                            Some(ty) => {
                                serialize_body.append(quote! { size += ::beserial::SerializeWithLength::serialize::<#ty, W>(&self.#i, writer)?; }.as_str());
                                serialized_size_body.append(quote! { size += ::beserial::SerializeWithLength::serialized_size::<#ty>(&self.#i); }.as_str());
                            }
                            None => {
                                serialize_body.append(quote! { size += Serialize::serialize(&self.#i, writer)?; }.as_str());
                                serialized_size_body.append(quote! { size += Serialize::serialized_size(&self.#i); }.as_str());
                            }
                        }
                        i = i + 1;
                    }
                }
                syn::VariantData::Unit => panic!("Serialize can not be derived for unit struct {}", name)
            };
        }
    };

    quote! {
        impl #impl_generics Serialize for #name #ty_generics #where_clause {
            #[allow(unused_mut,unused_variables)]
            fn serialize<W: ::beserial::WriteBytesExt>(&self, writer: &mut W) -> Result<usize, ::beserial::SerializingError> {
                let mut size = 0;
                #serialize_body
                return Ok(size);
            }
            #[allow(unused_mut,unused_variables)]
            fn serialized_size(&self) -> usize {
                let mut size = 0;
                #serialized_size_body
                return size;
            }
        }
    }
}

#[proc_macro_derive(Deserialize, attributes(beserial))]
pub fn derive_deserialize(input: TokenStream) -> TokenStream {
    let s = input.to_string();
    let ast = syn::parse_derive_input(&s).unwrap();
    impl_deserialize(&ast).parse().unwrap()
}

fn impl_deserialize(ast: &syn::DeriveInput) -> quote::Tokens {
    let name = &ast.ident;

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let deserialize_body;

    match ast.body {
        syn::Body::Enum(ref variants) => {
            let (enum_type, uvar) = parse_enum_attribs(ast);

            let ty;
            if uvar {
                ty = enum_type.unwrap_or_else(|| syn::Ident::from("u64"));
            } else {
                ty = enum_type.expect(format!("Deserialize can not be derived for enum {} without repr(u*) or repr(i*)", name).as_str());
            }

            let mut num_cases = quote::Tokens::new();
            let mut num = syn::ConstExpr::Lit(syn::Lit::from(0));
            for variant in variants {
                let ident = &variant.ident;
                num = variant.discriminant.clone().unwrap_or_else(|| if let syn::ConstExpr::Lit(syn::Lit::Int(i, _)) = num { syn::ConstExpr::Lit(syn::Lit::Int(i + 1, syn::IntTy::Unsuffixed)) } else { panic!("undiscriminated enum value following non-literal discriminant") });
                num_cases.append(quote! { #num => Ok(#name::#ident), });
            }

            if uvar {
                deserialize_body = quote! {
                    let u: uvar = Deserialize::deserialize(reader)?;
                    let num: u64 = u.into();
                    return match num {
                        #num_cases
                        _ => Err(::beserial::SerializingError::InvalidValue)
                    };
                };
            } else {
                deserialize_body = quote! {
                    let num: #ty = Deserialize::deserialize(reader)?;
                    return match num {
                        #num_cases
                        _ => Err(::beserial::SerializingError::InvalidValue)
                    };
                };
            }
        }
        syn::Body::Struct(ref variant) => {
            match variant {
                syn::VariantData::Struct(ref fields) => {
                    let mut field_cases = quote::Tokens::new();
                    for field in fields {
                        match field.ident {
                            None => panic!(),
                            Some(ref ident) => {
                                let (len_type, skip, _) = parse_field_attribs(&field);
                                if let Option::Some(Option::Some(default_value)) = skip {
                                    field_cases.append(quote! { #ident: #default_value });
                                    continue;
                                } else if let Option::Some(Option::None) = skip {
                                    let ty = &field.ty;
                                    field_cases.append(quote! { #ident: <#ty>::default() });
                                    continue;
                                }
                                match len_type {
                                    Some(ty) => {
                                        field_cases.append(quote! { #ident: ::beserial::DeserializeWithLength::deserialize::<#ty,R>(reader)?, })
                                    }
                                    None => {
                                        field_cases.append(quote! { #ident: Deserialize::deserialize(reader)?, })
                                    }
                                }
                            }
                        }
                    }
                    deserialize_body = quote!({
                        return Ok(#name {
                            #field_cases
                        });
                    });
                }
                syn::VariantData::Tuple(ref fields) => {
                    let mut field_cases = quote::Tokens::new();
                    for field in fields {
                        let (len_type, skip, _) = parse_field_attribs(&field);
                        if let Option::Some(Option::Some(default_value)) = skip {
                            field_cases.append(quote! { #default_value, });
                            continue;
                        } else if let Option::Some(Option::None) = skip {
                            let ty = &field.ty;
                            field_cases.append(quote! { <#ty>::default(), });
                            continue;
                        }
                        match len_type {
                            Some(ty) =>
                                field_cases.append(quote! { ::beserial::DeserializeWithLength::deserialize::<#ty,R>(reader)?, }),
                            None =>
                                field_cases.append(quote! { Deserialize::deserialize(reader)?, })
                        }
                    }
                    deserialize_body = quote!({
                        return Ok(#name (
                            #field_cases
                        ));
                    });
                }
                syn::VariantData::Unit => panic!("Serialize can not be derived for unit struct {}", name)
            };
        }
    };

    quote! {
        impl #impl_generics Deserialize for #name #ty_generics #where_clause {
            #[allow(unused_mut,unused_variables)]
            fn deserialize<R: ::beserial::ReadBytesExt>(reader: &mut R) -> Result<Self, ::beserial::SerializingError> {
                #deserialize_body
            }
        }
    }
}
