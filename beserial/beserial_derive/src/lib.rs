#![recursion_limit = "128"]

extern crate proc_macro;

use proc_macro2::{Span, TokenStream};
use syn::{parse_macro_input, Data, DeriveInput, Ident, Index, Meta, Path};

use quote::quote;

enum FieldAttribute {
    Uvar,
    Skip(Option<syn::Lit>),
    LenType(syn::Ident),
    Discriminant(u64),
}

#[inline]
fn cmp_ident(path: &Path, ident: &str) -> bool {
    match path.get_ident() {
        Some(id) => id == ident,
        None => false,
    }
}

// This will return a tuple once we have more options
fn parse_field_attribs(attrs: &[syn::Attribute]) -> Option<FieldAttribute> {
    for attr in attrs {
        if let Meta::List(ref meta_list) = attr.parse_meta().unwrap() {
            // Something like #[beserial(_)]
            if cmp_ident(&meta_list.path, "beserial") {
                for nested in meta_list.nested.iter() {
                    if let syn::NestedMeta::Meta(ref item) = nested {
                        // item is what's inside of beserial(_)
                        match item {
                            // something nested with item(_)
                            Meta::List(ref meta_list) => {
                                if cmp_ident(&meta_list.path, "len_type") {
                                    for nested in meta_list.nested.iter() {
                                        if let syn::NestedMeta::Meta(ref item) = nested {
                                            if let Meta::Path(value) = item {
                                                if !cmp_ident(value, "u8")
                                                    && !cmp_ident(value, "u16")
                                                    && !cmp_ident(value, "u32")
                                                {
                                                    panic!("beserial(len_type) must be one of [u8, u16, u32], but was {:?}", value);
                                                }
                                                return Some(FieldAttribute::LenType(
                                                    value.get_ident().cloned().unwrap(),
                                                ));
                                            }
                                        }
                                    }
                                }
                                if cmp_ident(&meta_list.path, "skip") {
                                    for nested in meta_list.nested.iter() {
                                        if let syn::NestedMeta::Meta(ref item) = nested {
                                            if let Meta::NameValue(meta_name_value) = item {
                                                if cmp_ident(&meta_name_value.path, "default") {
                                                    return Some(FieldAttribute::Skip(Some(
                                                        meta_name_value.lit.clone(),
                                                    )));
                                                }
                                            }
                                        }
                                    }
                                    return Some(FieldAttribute::Skip(None));
                                }
                            }
                            // Just a name, no additional nested ().
                            Meta::Path(ref path) => {
                                if cmp_ident(path, "skip") {
                                    return Some(FieldAttribute::Skip(None));
                                } else if cmp_ident(path, "uvar") {
                                    return Some(FieldAttribute::Uvar);
                                } else {
                                    panic!("unknown flag for beserial: {:?}", path)
                                }
                            }
                            Meta::NameValue(ref name_value) => {
                                if cmp_ident(&name_value.path, "discriminant") {
                                    // We do have something like beserial(discriminant = 123).
                                    // Parse discriminant.
                                    if let syn::Lit::Int(lit_int) = &name_value.lit {
                                        if let Ok(discriminant) = lit_int.base10_parse::<u64>() {
                                            return Some(FieldAttribute::Discriminant(
                                                discriminant,
                                            ));
                                        } else {
                                            panic!("discriminant cannot be parsed as u64");
                                        }
                                    } else {
                                        panic!("non-integer discriminant");
                                    }
                                } else {
                                    panic!("unknown flag for beserial: {:?}", name_value)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn parse_enum_attribs(ast: &syn::DeriveInput) -> (Option<syn::Ident>, bool) {
    let mut enum_type: Option<syn::Ident> = Option::None;
    let mut uvar = false;
    for attr in &ast.attrs {
        if let Meta::List(ref meta_list) = attr.parse_meta().unwrap() {
            if cmp_ident(&meta_list.path, "repr") {
                enum_type = meta_list.nested.first().and_then(|n| {
                    if let syn::NestedMeta::Meta(Meta::Path(ref meta_type)) = n {
                        meta_type.get_ident().cloned()
                    } else {
                        Option::None
                    }
                })
            } else if cmp_ident(&meta_list.path, "beserial") {
                for nested in meta_list.nested.iter() {
                    if let syn::NestedMeta::Meta(ref item) = nested {
                        if let Meta::Path(ref attr_ident) = item {
                            if cmp_ident(attr_ident, "uvar") {
                                uvar = true;
                            } else {
                                panic!("unknown flag for beserial: {:?}", attr_ident)
                            }
                        }
                    }
                }
            }
        }
    }
    (enum_type, uvar)
}

fn enum_has_data_attached(enum_def: &syn::DataEnum) -> bool {
    enum_def
        .variants
        .iter()
        .any(|variant| variant.fields != syn::Fields::Unit)
}

fn expr_from_value(value: u64) -> syn::Expr {
    let lit_int = syn::LitInt::new(&value.to_string(), Span::call_site());
    let expr_lit = syn::ExprLit {
        attrs: vec![],
        lit: syn::Lit::Int(lit_int),
    };
    syn::Expr::from(expr_lit)
}

fn int_to_correct_type(value: TokenStream, ty: &syn::Ident, uvar: bool) -> TokenStream {
    if uvar {
        quote! { ::beserial::uvar::from(#value as #ty) }
    } else {
        quote! { (#value as #ty) }
    }
}

#[proc_macro_derive(Serialize, attributes(beserial))]
pub fn derive_serialize(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(impl_serialize(&ast))
}

/// None means skip, otherwise return (serialize_body, serialized_size_body) token streams.
fn impl_serialize_field(
    field: &syn::Field,
    i: usize,
    override_ident: Option<&syn::Ident>,
) -> Option<(TokenStream, TokenStream)> {
    let len_type = match parse_field_attribs(&field.attrs) {
        Some(FieldAttribute::Skip(_)) => return None,
        Some(FieldAttribute::LenType(ty)) => Some(ty),
        _ => None,
    };

    match (override_ident, &field.ident) {
        // Tuple Structs
        (None, None) => {
            let index = Index::from(i);
            match len_type {
                Some(ty) => Some((
                    quote! { size += ::beserial::SerializeWithLength::serialize::<#ty, W>(&self.#index, writer)?; },
                    quote! { size += ::beserial::SerializeWithLength::serialized_size::<#ty>(&self.#index); },
                )),
                None => Some((
                    quote! { size += Serialize::serialize(&self.#index, writer)?; },
                    quote! { size += Serialize::serialized_size(&self.#index); },
                )),
            }
        }
        // Named Structs
        (None, Some(ref ident)) => match len_type {
            Some(ty) => Some((
                quote! { size += ::beserial::SerializeWithLength::serialize::<#ty, W>(&self.#ident, writer)?; },
                quote! { size += ::beserial::SerializeWithLength::serialized_size::<#ty>(&self.#ident); },
            )),
            None => Some((
                quote! { size += Serialize::serialize(&self.#ident, writer)?; },
                quote! { size += Serialize::serialized_size(&self.#ident); },
            )),
        },
        // Enums
        (Some(ident), _) => match len_type {
            Some(ty) => Some((
                quote! { size += ::beserial::SerializeWithLength::serialize::<#ty, W>(#ident, writer)?; },
                quote! { size += ::beserial::SerializeWithLength::serialized_size::<#ty>(#ident); },
            )),
            None => Some((
                quote! { size += Serialize::serialize(#ident, writer)?; },
                quote! { size += Serialize::serialized_size(#ident); },
            )),
        },
    }
}

fn impl_serialize(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let mut serialize_body = Vec::<TokenStream>::new();
    let mut serialized_size_body = Vec::<TokenStream>::new();

    match ast.data {
        Data::Enum(ref enum_def) => {
            let (enum_type, uvar) = parse_enum_attribs(ast);

            let ty = if uvar {
                enum_type.unwrap_or_else(|| Ident::new("u64", Span::call_site()))
            } else {
                enum_type.unwrap_or_else(|| {
                    panic!(
                        "Serialize can not be derived for enum {} without repr(u*) or repr(i*)",
                        name
                    )
                })
            };

            if enum_has_data_attached(enum_def) {
                // Serialization for enums carrying data is more difficult.
                // We have to build a match.
                let mut serialize_body_variants = Vec::<TokenStream>::new();
                let mut serialized_size_body_variants = Vec::<TokenStream>::new();

                // We start with a 0 discriminant if not given otherwise.
                let mut discriminant = 0;
                let mut first = true;
                for variant in enum_def.variants.iter() {
                    // Set discriminant.
                    match parse_field_attribs(&variant.attrs) {
                        // Some(FieldAttribute::Skip(_)) => continue, // For now, we do not allow skipping inside an enum.
                        Some(FieldAttribute::Discriminant(d)) => discriminant = d,
                        _ => {
                            // Only increase discriminant if we are not looking at the first variant.
                            if !first {
                                discriminant += 1;
                            }
                        }
                    };
                    first = false;

                    let discriminant_expr = expr_from_value(discriminant);
                    let discriminant_ts =
                        int_to_correct_type(quote! { #discriminant_expr }, &ty, uvar);
                    let variant_ident = &variant.ident;
                    // The match looks different depending on the type of the Fields.
                    match variant.fields {
                        syn::Fields::Unit => {
                            serialize_body_variants.push(quote! { #name::#variant_ident => {
                                size += Serialize::serialize(&#discriminant_ts, writer)?
                            }, });
                            serialized_size_body_variants.push(quote! { #name::#variant_ident => {
                                size += Serialize::serialized_size(&#discriminant_ts)
                            }, });
                        }
                        syn::Fields::Named(ref fields) => {
                            let mut idents = Vec::new();
                            let mut serialize_body_fields = Vec::<TokenStream>::new();
                            let mut serialized_size_body_fields = Vec::<TokenStream>::new();
                            for field in fields.named.iter() {
                                let ident = field.ident.as_ref().unwrap();
                                if let Some((serialize, serialized_size)) =
                                    impl_serialize_field(field, 0, Some(ident))
                                {
                                    serialize_body_fields.push(serialize);
                                    serialized_size_body_fields.push(serialized_size);
                                }
                                // Might be unused in the end.
                                idents.push(ident);
                            }

                            serialize_body_variants.push(
                                quote! { #name::#variant_ident { #(ref #idents),* } => {
                                    size += Serialize::serialize(&#discriminant_ts, writer)?;
                                    #(#serialize_body_fields)*
                                }, },
                            );
                            serialized_size_body_variants.push(
                                quote! { #name::#variant_ident { #(ref #idents),* } => {
                                    size += Serialize::serialized_size(&#discriminant_ts);
                                    #(#serialized_size_body_fields)*
                                }, },
                            );
                        }
                        syn::Fields::Unnamed(ref fields) => {
                            let mut idents = Vec::new();
                            let mut serialize_body_fields = Vec::<TokenStream>::new();
                            let mut serialized_size_body_fields = Vec::<TokenStream>::new();

                            for (i, field) in fields.unnamed.iter().enumerate() {
                                let ident = syn::Ident::new(&format!("f{}", i), Span::call_site());
                                if let Some((serialize, serialized_size)) =
                                    impl_serialize_field(field, 0, Some(&ident))
                                {
                                    serialize_body_fields.push(serialize);
                                    serialized_size_body_fields.push(serialized_size);
                                }
                                idents.push(ident);
                            }

                            serialize_body_variants.push(
                                quote! { #name::#variant_ident ( #(ref #idents),* ) => {
                                    size += Serialize::serialize(&#discriminant_ts, writer)?;
                                    #(#serialize_body_fields)*
                                }, },
                            );
                            serialized_size_body_variants.push(
                                quote! { #name::#variant_ident ( #(ref #idents),* ) => {
                                    size += Serialize::serialized_size(&#discriminant_ts);
                                    #(#serialized_size_body_fields)*
                                }, },
                            );
                        }
                    }
                }

                serialize_body.push(quote! { match self { #(#serialize_body_variants)* } });
                serialized_size_body
                    .push(quote! { match self { #(#serialized_size_body_variants)* } });
            } else {
                // Serialization for enums that do not carry any data is easy.
                // As defined here: https://doc.rust-lang.org/reference/items/enumerations.html
                // these can be cast to an integer directly.
                let discriminant = int_to_correct_type(quote! { *self }, &ty, uvar);
                serialize_body
                    .push(quote! { size += Serialize::serialize(&#discriminant, writer)?; });
                serialized_size_body
                    .push(quote! { size += Serialize::serialized_size(&#discriminant); });
            }
        }
        Data::Struct(ref data_struct) => {
            for (i, field) in data_struct.fields.iter().enumerate() {
                if let Some((serialize, serialized_size)) = impl_serialize_field(field, i, None) {
                    serialize_body.push(serialize);
                    serialized_size_body.push(serialized_size);
                } else {
                    continue;
                }
            }
        }
        Data::Union(_) => panic!("Serialize can not be derived for Union {}", name),
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
    gen
}

#[proc_macro_derive(Deserialize, attributes(beserial))]
pub fn derive_deserialize(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(impl_deserialize(&ast))
}

fn impl_deserialize_field(field: &syn::Field) -> TokenStream {
    let field_attrib = parse_field_attribs(&field.attrs);

    match (&field.ident, &field_attrib) {
        // tuple field, but skip with given default value
        (None, Some(FieldAttribute::Skip(Some(default_value)))) => {
            quote! { #default_value, }
        }
        // tuple field, but skip with Default trait
        (None, Some(FieldAttribute::Skip(None))) => {
            let ty = &field.ty;
            quote! { <#ty>::default(), }
        }

        // struct field, but skip with given default value
        (Some(ident), Some(FieldAttribute::Skip(Some(default_value)))) => {
            quote! { #ident: #default_value, }
        }
        // struct field, but skip with Default trait
        (Some(ident), Some(FieldAttribute::Skip(None))) => {
            let ty = &field.ty;
            quote! { #ident: <#ty>::default(), }
        }

        // tuple field with len_type
        (None, Some(FieldAttribute::LenType(ty))) => {
            quote! { ::beserial::DeserializeWithLength::deserialize::<#ty,R>(reader)?, }
        }
        // tuple field without len_type
        (None, None) => {
            quote! { ::beserial::Deserialize::deserialize(reader)?, }
        }

        // struct field with len_type
        (Some(ident), Some(FieldAttribute::LenType(ty))) => {
            quote! { #ident: ::beserial::DeserializeWithLength::deserialize::<#ty,R>(reader)?, }
        }
        // struct field without len_type
        (Some(ident), None) => {
            quote! { #ident: ::beserial::Deserialize::deserialize(reader)?, }
        }
        (_, Some(FieldAttribute::Uvar)) => {
            panic!("beserial(uvar) attribute not allowed for struct fields")
        }

        // struct field with discriminant
        (_, Some(FieldAttribute::Discriminant(_))) => {
            panic!("beserial(discriminant = ...) attribute not allowed for struct fields")
        }
    }
}

fn impl_deserialize(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let deserialize_body;

    match ast.data {
        Data::Enum(ref data_enum) => {
            let (enum_type, uvar) = parse_enum_attribs(ast);

            let ty = if uvar {
                enum_type.unwrap_or_else(|| Ident::new("u64", Span::call_site()))
            } else {
                enum_type.unwrap_or_else(|| {
                    panic!(
                        "Deserialize can not be derived for enum {} without repr(u*) or repr(i*)",
                        name
                    )
                })
            };

            // Deserialization works more generically.
            // We have to build a match anyway.
            let mut discriminant = expr_from_value(0);
            let mut first = true;
            let mut match_cases = Vec::<TokenStream>::new();
            let has_data = enum_has_data_attached(data_enum);

            // Iterate over all variants.
            for variant in data_enum.variants.iter() {
                let ident = &variant.ident;
                // The discriminant is calculated differently depending on whether the enum has data.
                // For data enums, we determine it by using our attributes.
                // For non-data enums, we solely rely on the explicit discriminants.
                if has_data {
                    match parse_field_attribs(&variant.attrs) {
                        // Some(FieldAttribute::Skip(_)) => continue, // For now, we do not allow skipping inside an enum.
                        Some(FieldAttribute::Discriminant(d)) => discriminant = expr_from_value(d),
                        _ => {
                            // Only increase discriminant if we are not looking at the first variant.
                            if !first {
                                if let syn::Expr::Lit(ref expr_lit) = discriminant {
                                    if let syn::Lit::Int(lit_int) = &expr_lit.lit {
                                        discriminant = expr_from_value(
                                            lit_int.base10_parse::<u64>().map(|x| x + 1).unwrap(),
                                        );
                                    } else {
                                        panic!("non-integer discriminant");
                                    }
                                } else {
                                    panic!("non-literal discriminant");
                                }
                            }
                        }
                    };
                } else {
                    match &variant.discriminant {
                        Some((_, expr)) => discriminant = expr.clone(),
                        None => {
                            // Only increment if not first.
                            if !first {
                                if let syn::Expr::Lit(ref expr_lit) = discriminant {
                                    if let syn::Lit::Int(lit_int) = &expr_lit.lit {
                                        discriminant = expr_from_value(
                                            lit_int.base10_parse::<u64>().map(|x| x + 1).unwrap(),
                                        );
                                    } else {
                                        panic!("non-integer discriminant");
                                    }
                                } else {
                                    panic!("non-literal discriminant");
                                }
                            }
                        }
                    };
                }
                first = false;

                // Now deserialize depending on type of variant's fields.
                match variant.fields {
                    syn::Fields::Unit => {
                        match_cases.push(quote! { #discriminant => Ok(#name::#ident), });
                    }
                    syn::Fields::Named(ref fields) => {
                        let mut field_cases = Vec::<TokenStream>::new();
                        for field in fields.named.iter() {
                            field_cases.push(impl_deserialize_field(field));
                        }
                        match_cases.push(
                            quote! { #discriminant => Ok(#name::#ident { #(#field_cases)* }), },
                        );
                    }
                    syn::Fields::Unnamed(ref fields) => {
                        let mut field_cases = Vec::<TokenStream>::new();
                        for field in fields.unnamed.iter() {
                            field_cases.push(impl_deserialize_field(field));
                        }
                        match_cases.push(
                            quote! { #discriminant => Ok(#name::#ident ( #(#field_cases)* )), },
                        );
                    }
                }
            }

            if uvar {
                deserialize_body = quote! {
                    let u: uvar = Deserialize::deserialize(reader)?;
                    let discriminant: u64 = u.into();
                    return match discriminant {
                        #(#match_cases)*
                        _ => Err(::beserial::SerializingError::InvalidValue)
                    };
                };
            } else {
                deserialize_body = quote! {
                    let discriminant: #ty = Deserialize::deserialize(reader)?;
                    return match discriminant {
                        #(#match_cases)*
                        _ => Err(::beserial::SerializingError::InvalidValue)
                    };
                };
            }
        }
        Data::Struct(ref data_struct) => {
            let mut tuple = false;
            let mut field_cases = Vec::<TokenStream>::new();
            for field in data_struct.fields.iter() {
                if field.ident.is_none() {
                    tuple = true;
                }
                field_cases.push(impl_deserialize_field(field));
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
        Data::Union(_) => panic!("Deserialize can not be derived for Union {}", name),
    };

    let gen = quote! {
        impl #impl_generics Deserialize for #name #ty_generics #where_clause {
            #[allow(unused_mut,unused_variables)]
            fn deserialize<R: ::beserial::ReadBytesExt>(reader: &mut R) -> Result<Self, ::beserial::SerializingError> {
                #deserialize_body
            }
        }
    };
    gen
}
