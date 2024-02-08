use std::collections::BTreeMap;

use convert_case::{Case, Casing};
use open_rpc::{IntegerLiteral, Literal, ObjectLiteral, Schema, SchemaContents, StringLiteral};
use quote::ToTokens;
use syn::{Field, File, GenericArgument, Ident, ItemStruct, Path, PathArguments, Type};

pub struct ParsedItemStruct(ItemStruct);

impl ParsedItemStruct {
    pub fn ident(&self) -> &Ident {
        &self.0.ident
    }

    pub fn title(&self) -> String {
        self.0.ident.to_string()
    }

    pub fn description(&self) -> String {
        "".into()
    }

    pub fn fields_to_open_rpc_schema_contents(&self) -> SchemaContents {
        let mut properties = BTreeMap::new();
        let mut required_properties = Vec::with_capacity(self.0.fields.len());

        self.0.fields.iter().for_each(|item| {
            let ident = item
                .ident
                .to_token_stream()
                .to_string()
                .to_case(Case::Camel);

            properties.insert(
                ident.clone(),
                Schema {
                    title: Some(ident.clone()),
                    description: None,
                    contents: SchemaContents::Literal(Self::field_to_schema_type(item)),
                },
            );

            if Self::is_field_required(item) {
                required_properties.push(ident)
            }
        });

        let schema = SchemaContents::Literal(open_rpc::Literal::Object(open_rpc::ObjectLiteral {
            properties,
            required: required_properties,
        }));

        schema
    }

    fn is_field_required(field: &Field) -> bool {
        if field.ty.to_token_stream().to_string().contains("Option") {
            return false;
        }

        true
    }

    fn field_to_schema_type(field: &Field) -> Literal {
        match &field.ty {
            syn::Type::Path(path) => field_to_literal(&path.path),
            _ => unreachable!(),
        }
    }
}

pub fn extract_structs_from_ast(file: &File) -> Vec<ParsedItemStruct> {
    let structs: Vec<ParsedItemStruct> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item_struct) => Some(item_struct.clone().into()),
            _ => None,
        })
        .collect();

    structs
}

impl Into<ParsedItemStruct> for ItemStruct {
    fn into(self) -> ParsedItemStruct {
        ParsedItemStruct(self)
    }
}

/// Check if we are dealing with a wrapped type here, e.g. `Vec<u8>`, `Option<String>` and flatten those to its most inner type.
/// `Vec<u8>` becomes `u8`, `Option<String>` becomes `String` and `Option<Vec<String>>` becomes `String`.
fn flatten_path(path: Path) -> (Path, Ident) {
    let ident = path.segments.first().unwrap().ident.clone();

    match ident.to_string().as_str() {
        "Vec" | "Option" => {
            if let PathArguments::AngleBracketed(outer_type) =
                path.segments.first().unwrap().arguments.clone()
            {
                if let GenericArgument::Type(arg) = outer_type.args.first().unwrap() {
                    if let Type::Path(inner_type) = arg {
                        return flatten_path(inner_type.path.clone());
                    } else {
                        return (path, ident);
                    }
                } else {
                    return (path, ident);
                }
            } else {
                return (path, ident);
            }
        }
        _ => return (path, ident),
    }
}

pub fn field_to_literal(path: &Path) -> Literal {
    let (_flattened_path, ident) = flatten_path(path.to_owned());
    match ident.to_string().as_str() {
        "Blake2bHash" | "Blake2sHash" | "VrfSeed" => DefaultStringLiteral::default().to_literal(),
        "u8" | "u16" | "u32" | "u64" => DefaultIntegerLiteral::default().to_literal(),
        "ExecutedTransaction" | "BlockAdditionalFields" => {
            DefaultObjectLiteral::default().to_literal()
        }
        _ => {
            dbg!(ident);
            unreachable!()
        }
    }
}

trait ToLiteral<T> {
    fn to_literal(self) -> T;
}

struct DefaultStringLiteral(StringLiteral);
struct DefaultIntegerLiteral(IntegerLiteral);
struct DefaultObjectLiteral(ObjectLiteral);

impl ToLiteral<Literal> for DefaultStringLiteral {
    fn to_literal(self) -> Literal {
        Literal::String(self.0)
    }
}

impl ToLiteral<Literal> for DefaultIntegerLiteral {
    fn to_literal(self) -> Literal {
        Literal::Integer(self.0)
    }
}

impl ToLiteral<Literal> for DefaultObjectLiteral {
    fn to_literal(self) -> Literal {
        Literal::Object(self.0)
    }
}

impl Default for DefaultStringLiteral {
    fn default() -> Self {
        Self(StringLiteral {
            min_length: Default::default(),
            max_length: Default::default(),
            pattern: Default::default(),
            format: Default::default(),
            enumeration: Default::default(),
        })
    }
}

impl Default for DefaultIntegerLiteral {
    fn default() -> Self {
        Self(IntegerLiteral {
            multiple_of: Default::default(),
            minimum: Default::default(),
            maximum: Default::default(),
            exclusive_minimum: Default::default(),
            exclusive_maximum: Default::default(),
        })
    }
}

impl Default for DefaultObjectLiteral {
    fn default() -> Self {
        Self(ObjectLiteral {
            properties: Default::default(),
            required: Default::default(),
        })
    }
}
