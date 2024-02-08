use convert_case::{Case, Casing};
use quote::ToTokens;
use serde_json::{Map, Value};
use syn::{Field, File, GenericArgument, Ident, ItemStruct, Path, PathArguments, Type};

#[derive(Debug)]
pub struct ParsedItemStruct(ItemStruct);

impl ParsedItemStruct {
    pub fn title(&self) -> String {
        self.0.ident.to_string()
    }

    pub fn description(&self) -> String {
        "".into()
    }

    pub fn to_schema(&self) -> Value {
        let mut map = Map::new();
        map.insert("title".into(), Value::String(self.title()));
        map.insert("description".into(), Value::String(self.description()));
        map.insert("properties".into(), self.properties());
        map.insert("required".into(), Value::Array(self.required_fields()));
        Value::Object(map)
    }

    fn properties(&self) -> Value {
        let props: Map<String, Value> = self
            .0
            .fields
            .iter()
            .map(|field| {
                let mut prop_fields = Map::new();
                let field_ident = field
                    .ident
                    .to_token_stream()
                    .to_string()
                    .to_case(Case::Camel);

                prop_fields.insert("title".into(), Value::String(field_ident.clone()));
                prop_fields.insert("type".into(), Self::param_to_json_type(field));
                (field_ident, Value::Object(prop_fields))
            })
            .collect();

        Value::Object(props)
    }

    fn param_to_json_type(param: &Field) -> Value {
        match &param.ty {
            Type::Path(path) => {
                let ident = path
                    .path
                    .segments
                    .first()
                    .expect("Function paramater should have a segment")
                    .ident
                    .to_string();

                if ident.to_string() == "Vec" {
                    return Value::String("array".into());
                }

                let (_path_type, inner_ident) = Self::flatten_type(path.path.clone());
                match inner_ident.to_string().as_str() {
                    "Address"
                    | "Blake2bHash"
                    | "Blake2sHash"
                    | "CompressedPublicKey"
                    | "PublicKey"
                    | "String"
                    | "VrfSeed" => return Value::String("string".into()),
                    "u8" | "u16" | "u32" | "u64" | "usize" | "Coin" => {
                        return Value::String("number".into())
                    }
                    "bool" => return Value::String("bool".into()),
                    "AccountAdditionalFields"
                    | "BitSet"
                    | "Block"
                    | "BlockAdditionalFields"
                    | "ExecutedTransaction"
                    | "MultiSignature"
                    | "Transaction"
                    | "T"
                    | "S" => return Value::String("object".into()),
                    _ => panic!("{:?}", inner_ident),
                }
            }
            Type::Array(_) => Value::String("array".into()),
            _ => unreachable!(),
        }
    }

    fn required_fields(&self) -> Vec<Value> {
        self.0
            .fields
            .iter()
            .filter_map(|field| {
                if field.ty.to_token_stream().to_string().contains("Option") {
                    return None;
                }

                Some(Value::String(
                    field
                        .ident
                        .to_token_stream()
                        .to_string()
                        .to_case(Case::Camel),
                ))
            })
            .collect()
    }

    /// Check if we are dealing with a wrapped type here, e.g. `Vec<u8>`, `Option<String>` and flatten those to its most inner type.
    /// `Vec<u8>` becomes `u8`, `Option<String>` becomes `String` and `Option<Vec<String>>` becomes `String`.
    fn flatten_type(path: Path) -> (Path, Ident) {
        let ident = path.segments.first().unwrap().ident.clone();

        match ident.to_string().as_str() {
            "Vec" | "Option" => {
                if let PathArguments::AngleBracketed(outer_type) =
                    path.segments.first().unwrap().arguments.clone()
                {
                    if let GenericArgument::Type(arg) = outer_type.args.first().unwrap() {
                        if let Type::Path(inner_type) = arg {
                            return Self::flatten_type(inner_type.path.clone());
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
}

impl Into<ParsedItemStruct> for ItemStruct {
    fn into(self) -> ParsedItemStruct {
        ParsedItemStruct(self)
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
