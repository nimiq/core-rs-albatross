#![recursion_limit = "128"]

//! This crate allows to automatically derive Serialize and Deserialize implementations for beserial.
//!
//! While the derivation for basic structs works simply by adding `#[derive(Serialize, Deserialize)]`,
//! there are multiple advanced options possible.
//!
//! ## (De-)serializing structs
//! If the struct contains fields that have an unknown size (such as `Vec`) and these implement
//! `SerializeWithLength` and `DeserializeWithLength`, automatic derivation is still possible.
//! You can add the `#[beserial(len_type(X))]` attribute to the field and specify how the length
//! should be encoded (`u8`, `u16`, or `u32`).
//!
//! Other field options are:
//! - `#[beserial(skip)]` skips this field during (de-)serialization and relies on `Default` instead
//! - `#[beserial(skip(default = X))]` same as `skip` but allows to give a custom default value literal
//! - `#[beserial(len_type(X))]` see above, allows to derive (de-)serialization for fields
//!   implementing `(De-)SerializeWithLength`
//! - `#[beserial(len_type(X, limit = Y))]` same as `len_type` but allows to specify a custom
//!   limit on the length during deserialization
//!
//! ## (De-)serializing enums
//! Enums are a special case as they require a discriminant for each enum case.
//! For the derivation to know how the discriminant should be encoded, it is required that
//! the enum itself has a `#[repr(X)]` attribute or `#[beserial(uvar)]`.
//!
//! An enum with `#[repr(u8)]`, for example, will encode the enum case as a `u8`.
//! Given `#[beserial(uvar)]`, the discriminant will be encoded as a variable sized unsigned int.
//!
//! By default, the discriminant starts at 0 for the first case and increases by 1.
//! For enums that do not carry any data, you can specify the discriminant directly
//! as long as the enum implements `Copy`:
//! ```
//! # #[macro_use] extern crate beserial_derive;
//! # fn main() {
//! use beserial::{Serialize, Deserialize};
//! #[derive(Serialize, Deserialize, Copy, Clone)]
//! #[repr(u8)]
//! enum Foo {
//!     A = 1,
//!     B = 5,
//! }
//! # }
//! ```
//!
//! For enums carrying data, you can specify the discriminant via `#[beserial(discriminant = X)]`:
//! ```
//! # #[macro_use] extern crate beserial_derive;
//! # fn main() {
//! use beserial::{Serialize, Deserialize};
//! #[derive(Serialize, Deserialize)]
//! #[repr(u8)]
//! enum Foo {
//!     #[beserial(discriminant = 1)]
//!     A(bool),
//!     #[beserial(discriminant = 5)]
//!     B(u8),
//! }
//! # }
//! ```
//!
//! *Note:* Fields in data-carrying enums are allowed to have the same options as available in structs.
//! This means that you can create enum cases that carry a `Vec` and you can specify the `len_type`
//! for this `Vec`.
//! ```
//! # #[macro_use] extern crate beserial_derive;
//! # fn main() {
//! use beserial::{Serialize, Deserialize};
//! #[derive(Serialize, Deserialize)]
//! #[repr(u8)]
//! enum Foo {
//!     #[beserial(discriminant = 1)]
//!     A(#[beserial(len_type(u8, limit = 2))] Vec<u16>),
//!     #[beserial(discriminant = 5)]
//!     B(u8),
//! }
//! # }
//! ```

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Ident, Index, Meta, Path};

enum FieldAttribute {
    Uvar,
    Skip(Option<syn::Lit>),
    LenType(syn::Ident, Option<usize>),
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
                                    // len_type accepts the len type (mandatory) and a limit
                                    let mut len_type = None;
                                    let mut limit = None;
                                    for nested in meta_list.nested.iter() {
                                        if let syn::NestedMeta::Meta(ref item) = nested {
                                            match item {
                                                Meta::Path(value) => {
                                                    if !cmp_ident(value, "u8")
                                                        && !cmp_ident(value, "u16")
                                                        && !cmp_ident(value, "u32")
                                                    {
                                                        panic!("beserial(len_type) must be one of [u8, u16, u32], but was {value:?}");
                                                    }
                                                    len_type =
                                                        Some(value.get_ident().cloned().unwrap());
                                                }
                                                Meta::NameValue(name_value) => {
                                                    if !cmp_ident(&name_value.path, "limit") {
                                                        panic!("beserial(len_type) can only have an additional limit attribute, but was {name_value:?}");
                                                    }
                                                    // We do have something like beserial(discriminant = 123).
                                                    // Parse discriminant.
                                                    if let syn::Lit::Int(lit_int) = &name_value.lit
                                                    {
                                                        if let Ok(l) =
                                                            lit_int.base10_parse::<usize>()
                                                        {
                                                            limit = Some(l);
                                                        } else {
                                                            panic!(
                                                                "limit cannot be parsed as usize"
                                                            );
                                                        }
                                                    } else {
                                                        panic!("non-integer limit");
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    if let Some(len_type) = len_type {
                                        return Some(FieldAttribute::LenType(len_type, limit));
                                    }
                                }
                                if cmp_ident(&meta_list.path, "skip") {
                                    for nested in meta_list.nested.iter() {
                                        if let syn::NestedMeta::Meta(Meta::NameValue(
                                            meta_name_value,
                                        )) = nested
                                        {
                                            if cmp_ident(&meta_name_value.path, "default") {
                                                return Some(FieldAttribute::Skip(Some(
                                                    meta_name_value.lit.clone(),
                                                )));
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
                                    panic!("unknown flag for beserial: {path:?}")
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
                                    panic!("unknown flag for beserial: {name_value:?}")
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
                    if let syn::NestedMeta::Meta(Meta::Path(ref attr_ident)) = nested {
                        if cmp_ident(attr_ident, "uvar") {
                            uvar = true;
                        } else {
                            panic!("unknown flag for beserial: {attr_ident:?}")
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

fn expr_from_u64(value: u64) -> syn::Expr {
    let lit_int = syn::LitInt::new(&value.to_string(), Span::call_site());
    let expr_lit = syn::ExprLit {
        attrs: vec![],
        lit: syn::Lit::Int(lit_int),
    };
    syn::Expr::from(expr_lit)
}

fn expr_from_limit(limit: &Option<usize>) -> TokenStream {
    match limit {
        None => quote! { None },
        Some(limit) => {
            quote! { Some(#limit) }
        }
    }
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
        Some(FieldAttribute::LenType(ty, _)) => Some(ty),
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
                        "Serialize can not be derived for enum {name} without repr(u*) or repr(i*)"
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

                    let discriminant_expr = expr_from_u64(discriminant);
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
                                let ident = syn::Ident::new(&format!("f{i}"), Span::call_site());
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
        Data::Union(_) => panic!("Serialize can not be derived for Union {name}"),
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
        (None, Some(FieldAttribute::LenType(ty, limit))) => {
            let limit = expr_from_limit(limit);
            quote! { ::beserial::DeserializeWithLength::deserialize_with_limit::<#ty,R>(reader, #limit)?, }
        }
        // tuple field without len_type
        (None, None) => {
            quote! { ::beserial::Deserialize::deserialize(reader)?, }
        }

        // struct field with len_type
        (Some(ident), Some(FieldAttribute::LenType(ty, limit))) => {
            let limit = expr_from_limit(limit);
            quote! { #ident: ::beserial::DeserializeWithLength::deserialize_with_limit::<#ty,R>(reader, #limit)?, }
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
                        "Deserialize can not be derived for enum {name} without repr(u*) or repr(i*)"
                    )
                })
            };

            // Deserialization works more generically.
            // We have to build a match anyway.
            let mut discriminant = expr_from_u64(0);
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
                        Some(FieldAttribute::Discriminant(d)) => discriminant = expr_from_u64(d),
                        _ => {
                            // Only increase discriminant if we are not looking at the first variant.
                            if !first {
                                if let syn::Expr::Lit(ref expr_lit) = discriminant {
                                    if let syn::Lit::Int(lit_int) = &expr_lit.lit {
                                        discriminant = expr_from_u64(
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
                                        discriminant = expr_from_u64(
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
        Data::Union(_) => panic!("Deserialize can not be derived for Union {name}"),
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
