extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;

use proc_macro::TokenStream;

#[proc_macro_derive(Serialize)]
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

            serialize_body.append(quote! { size += (*self as #ty).serialize(writer)?; });
            serialized_size_body.append(quote! { size += (0 as #ty).serialized_size(); });
        },
        syn::Body::Struct(ref variant) => {
            match variant {
                syn::VariantData::Struct(ref fields) => {
                    for field in fields {
                        match field.ident {
                            None => panic!(),
                            Some(ref ident) => {
                                serialize_body.append(quote! { size += self.#ident.serialize(writer)?; }.as_str());
                                serialized_size_body.append(quote! { size += self.#ident.serialized_size(); }.as_str());
                            }
                        }
                    }
                },
                syn::VariantData::Tuple(ref fields) => {
                    let mut i = 0;
                    for _ in fields {
                        serialize_body.append(quote! { size += self.#i.serialize(writer)?; }.as_str());
                        serialized_size_body.append(quote! { size += self.#i.serialized_size(); }.as_str());
                    }
                },
                syn::VariantData::Unit => panic!("Serialize can not be derived for unit struct {}", name)
            };
        }
    };

    quote! {
        impl #impl_generics ::beserial::Serialize for #name #ty_generics #where_clause {
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

#[proc_macro_derive(Deserialize)]
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
                                deserialize_body.append(quote! { #ident: Deserialize::deserialize(reader)?, }.as_str());
                            }
                        }
                    }
                    deserialize_body.append("} ) ;");
                },
                syn::VariantData::Tuple(ref fields) => {
                    deserialize_body.append("return Ok ( ");
                    deserialize_body.append(name);
                    deserialize_body.append("(");
                    for _ in fields {
                        deserialize_body.append(quote! { Deserialize::deserialize(reader)?, }.as_str());
                    }
                    deserialize_body.append(") ) ;");
                },
                syn::VariantData::Unit => panic!("Serialize can not be derived for unit struct {}", name)
            };
        }
    };

    quote! {
        impl #impl_generics ::beserial::Deserialize for #name #ty_generics #where_clause {
            fn deserialize<R: ::beserial::ReadBytesExt>(reader: &mut R) -> ::std::io::Result<Self> {
                #deserialize_body
            }
        }
    }
}
