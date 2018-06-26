extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;

use proc_macro::TokenStream;

// This will return a tuple once we have more options
fn parse_field_attribs(field: &syn::Field) -> Option<&syn::Ident> {
    let mut len_type = Option::None;
    for attr in &field.attrs {
        if let syn::MetaItem::List(ref attr_ident, ref nesteds) = attr.value {
            if attr_ident == "beserial" {
                for nested in nesteds {
                    if let syn::NestedMetaItem::MetaItem(ref item) = nested {
                        if let syn::MetaItem::List(ref attr_ident, ref nesteds) = item {
                            if attr_ident == "len_type" {
                                for nested in nesteds {
                                    if let syn::NestedMetaItem::MetaItem(ref item) = nested {
                                        if let syn::MetaItem::Word(value) = item {
                                            if value != "u8" && value != "u16" && value != "u32" {
                                                panic!("beserial(len_type) must be one of u8, u16 and u32");
                                            }
                                            len_type = Option::Some(value);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    return len_type;
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
            let mut enum_type: Option<syn::Ident> = Option::None;
            for attr in &ast.attrs {
                if let syn::MetaItem::List(ref ident, ref items) = attr.value {
                    if ident == "repr" {
                        enum_type = items.iter().next().map_or(Option::None, |n| {
                            if let syn::NestedMetaItem::MetaItem(syn::MetaItem::Word(ref meta_type)) = n { Option::Some(meta_type.clone()) } else { Option::None }
                        })
                    }
                }
            }

            let ty = enum_type.expect(format!("Serialize can not be derived for enum {} without repr(u*) or repr(i*)", name).as_str());

            serialize_body.append(quote! { size += Serialize::serialize(&(*self as #ty), writer)?; });
            serialized_size_body.append(quote! { size += Serialize::serialized_size(&(0 as #ty)); });
        },
        syn::Body::Struct(ref variant) => {
            match variant {
                syn::VariantData::Struct(ref fields) => {
                    for field in fields {
                        let len_type = parse_field_attribs(&field);
                        match field.ident {
                            None => panic!(),
                            Some(ref ident) => {
                                match len_type {
                                    Some(ty) => {
                                        serialize_body.append(quote! { size += SerializeWithLength::serialize::<#ty, W>(&self.#ident, writer)?; }.as_str());
                                        serialized_size_body.append(quote! { size += SerializeWithLength::serialized_size::<#ty>(&self.#ident); }.as_str());
                                    },
                                    None => {
                                        serialize_body.append(quote! { size += Serialize::serialize(&self.#ident, writer)?; }.as_str());
                                        serialized_size_body.append(quote! { size += Serialize::serialized_size(&self.#ident); }.as_str());
                                    }
                                }
                            }
                        }
                    }
                },
                syn::VariantData::Tuple(ref fields) => {
                    let mut i = 0;
                    for field in fields {
                        let len_type = parse_field_attribs(&field);
                        match len_type {
                            Some(ty) => {
                                serialize_body.append(quote! { size += SerializeWithLength::serialize::<#ty, W>(&self.#i, writer)?; }.as_str());
                                serialized_size_body.append(quote! { size += SerializeWithLength::serialized_size::<#ty>(&self.#i); }.as_str());
                            },
                            None => {
                                serialize_body.append(quote! { size += Serialize::serialize(&self.#i, writer)?; }.as_str());
                                serialized_size_body.append(quote! { size += Serialize::serialized_size(&self.#i); }.as_str());
                            }
                        }
                    }
                },
                syn::VariantData::Unit => panic!("Serialize can not be derived for unit struct {}", name)
            };
        }
    };

    quote! {
        impl #impl_generics Serialize for #name #ty_generics #where_clause {
            fn serialize<W: ::beserial::WriteBytesExt>(&self, writer: &mut W) -> ::std::io::Result<usize> {
                let mut size = 0;
                #serialize_body
                return Ok(size);
            }
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

    let mut deserialize_body = quote::Tokens::new();

    match ast.body {
        syn::Body::Enum(ref variants) => {
            let mut enum_type: Option<syn::Ident> = Option::None;
            for attr in &ast.attrs {
                if let syn::MetaItem::List(ref ident, ref items) = attr.value {
                    if ident == "repr" {
                        enum_type = items.iter().next().map_or(Option::None, |n| {
                            if let syn::NestedMetaItem::MetaItem(syn::MetaItem::Word(ref meta_type)) = n { Option::Some(meta_type.clone()) } else { Option::None }
                        })
                    }
                }
            }

            let ty = enum_type.expect(format!("Deserialize can not be derived for enum {} without repr(u*) or repr(i*)", name).as_str());
            deserialize_body.append(quote! { let num: #ty = Deserialize::deserialize(reader)?; });
            deserialize_body.append("return match num {");

            let mut num = syn::ConstExpr::Lit(syn::Lit::from(0));
            for variant in variants {
                let ident = &variant.ident;
                num = variant.discriminant.clone().unwrap_or(syn::ConstExpr::Binary(syn::BinOp::Add, num.into(), syn::ConstExpr::Lit(syn::Lit::from(1)).into()));
                deserialize_body.append(quote! { #num => Ok(#name::#ident), });
            }

            deserialize_body.append(quote! { _ => Err(::std::io::Error::from(::std::io::ErrorKind::InvalidInput)) });

            deserialize_body.append("} ;");
        },
        syn::Body::Struct(ref variant) => {
            match variant {
                syn::VariantData::Struct(ref fields) => {
                    deserialize_body.append("return Ok ( ");
                    deserialize_body.append(name);
                    deserialize_body.append("{");
                    for field in fields {
                        match field.ident {
                            None => panic!(),
                            Some(ref ident) => {
                                let len_type = parse_field_attribs(&field);
                                match len_type {
                                    Some(ty) => {
                                        deserialize_body.append(quote! { #ident: DeserializeWithLength::deserialize::<#ty,R>(reader)?, }.as_str())
                                    },
                                    None => {
                                        deserialize_body.append(quote! { #ident: Deserialize::deserialize(reader)?, }.as_str())
                                    }
                                }
                            }
                        }
                    }
                    deserialize_body.append("} ) ;");
                },
                syn::VariantData::Tuple(ref fields) => {
                    deserialize_body.append("return Ok ( ");
                    deserialize_body.append(name);
                    deserialize_body.append("(");
                    for field in fields {
                        let len_type = parse_field_attribs(&field);
                        match len_type {
                            Some(ty) =>
                                deserialize_body.append(quote! { DeserializeWithLength::deserialize::<#ty,R>(reader)?, }.as_str()),
                            None =>
                                deserialize_body.append(quote! { Deserialize::deserialize(reader)?, }.as_str())
                        }
                    }
                    deserialize_body.append(") ) ;");
                },
                syn::VariantData::Unit => panic!("Serialize can not be derived for unit struct {}", name)
            };
        }
    };

    quote! {
        impl #impl_generics Deserialize for #name #ty_generics #where_clause {
            fn deserialize<R: ::beserial::ReadBytesExt>(reader: &mut R) -> ::std::io::Result<Self> {
                #deserialize_body
            }
        }
    }
}
