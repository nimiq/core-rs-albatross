use convert_case::{Case, Casing};
use open_rpc_schema::{
    document::{ContentDescriptorObject, ContentDescriptorOrReference, JSONSchema, MethodObject},
    schemars::schema::{InstanceType, RootSchema, SchemaObject, SingleOrVec},
};
use quote::ToTokens;
use serde_json::{Map, Value};
use syn::{
    Field, File, GenericArgument, Ident, ItemStruct, Pat, PatIdent, Path, PathArguments,
    PathSegment, ReturnType, TraitItem, TraitItemFn, Type,
};

#[derive(Debug, Clone)]
pub struct ParsedItemStruct(ItemStruct);
pub struct ParsedTraitItemFn(TraitItemFn);

impl ParsedItemStruct {
    #[inline]
    pub fn title(&self) -> String {
        self.0.ident.to_string()
    }

    #[inline]
    pub fn description(&self) -> String {
        "".into()
    }

    pub fn properties(&self, structs: &Vec<ParsedItemStruct>) -> Value {
        let props: Map<String, Value> = self
            .0
            .fields
            .iter()
            .filter_map(|field| {
                let path = match &field.ty {
                    Type::Path(p) => p,
                    Type::Array(_) => {
                        return None;
                    }
                    _ => unreachable!(),
                };
                let inner_type = Self::unwrap_type(path.path.clone(), true);

                let mut prop_fields = Map::new();
                let field_ident = field
                    .ident
                    .to_token_stream()
                    .to_string()
                    .to_case(Case::Camel);

                prop_fields.insert("title".into(), Value::String(field_ident.clone()));
                let schema_ref = structs
                    .iter()
                    .find(|s| inner_type.1.to_string() == s.title());
                prop_fields.append(&mut Self::param_to_json_type(&field, schema_ref));

                Some((field_ident, Value::Object(prop_fields)))
            })
            .collect();

        Value::Object(props)
    }

    fn param_to_json_type(
        param: &Field,
        schema_ref: Option<&ParsedItemStruct>,
    ) -> Map<String, Value> {
        let mut map = Map::new();
        match &param.ty {
            Type::Path(type_path) => {
                let mut path = type_path.path.clone();
                let mut ident = path
                    .segments
                    .first()
                    .expect("Function paramater should have a segment")
                    .ident
                    .clone();

                if ident == "Option" {
                    (path, ident) = Self::unwrap_type(type_path.path.clone(), false);
                }

                if ident.to_string() == "Vec" {
                    let mut items_map = Map::new();
                    if let Some(reference) = schema_ref {
                        items_map.insert(
                            "$ref".into(),
                            Value::String(format!("#/components/schemas/{}", reference.title())),
                        );
                    } else {
                        items_map.insert("type".into(), Self::map_type(&path));
                    }

                    map.insert("type".to_string(), Value::String("array".into()));
                    map.insert("items".to_string(), Value::Object(items_map));
                } else {
                    if let Some(reference) = schema_ref {
                        map.insert(
                            "$ref".into(),
                            Value::String(format!("#/components/schemas/{}", reference.title())),
                        );
                    } else {
                        map.insert("type".into(), Self::map_type(&path));
                    }
                }
            }
            _ => unreachable!(),
        }

        map
    }

    fn map_type(path: &Path) -> Value {
        let (_path_type, inner_ident) = Self::unwrap_type(path.clone(), true);
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
            "bool" => return Value::String("boolean".into()),
            "AccountAdditionalFields"
            | "BitSet"
            | "S"
            | "T"
            | "BlockAdditionalFields"
            | "MultiSignature" => return Value::String("object".into()),
            _ => panic!("{:?}", inner_ident),
        }
    }

    pub fn required_fields(&self) -> Vec<Value> {
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

    /// Check if we are dealing with a wrapped type here, e.g. `Vec<u8>`, `Option<String>` and flatten those to its child type.
    /// `Vec<u8>` becomes `u8`, `Option<String>` becomes `String` and `Option<Vec<String>>` becomes `Vec<String>`.
    /// Calling this function with `recursive` is true, it will keep unwrapping until it no further can. `Option<Vec<String>>` would become `String`.
    fn unwrap_type(path: Path, recursive: bool) -> (Path, Ident) {
        let ident = path.segments.first().unwrap().ident.clone();

        match ident.to_string().as_str() {
            "Vec" | "Option" => {
                if let PathArguments::AngleBracketed(outer_type) =
                    path.segments.first().unwrap().arguments.clone()
                {
                    if let GenericArgument::Type(arg) = outer_type.args.first().unwrap() {
                        if let Type::Path(inner_type) = arg {
                            if recursive {
                                return Self::unwrap_type(inner_type.path.clone(), true);
                            }

                            (
                                inner_type.path.clone(),
                                inner_type.path.segments.first().unwrap().ident.clone(),
                            )
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

impl ParsedTraitItemFn {
    #[inline]
    pub fn title(&self) -> String {
        self.0.sig.ident.to_string()
    }

    #[inline]
    pub fn description(&self) -> String {
        "".into()
    }

    pub fn to_method(&self) -> MethodObject {
        let mut method =
            MethodObject::new(self.title().to_case(Case::Camel), Some(self.description()));
        method.params = self.params();
        method.result = self.return_type();
        method
    }

    fn params(&self) -> Vec<ContentDescriptorOrReference> {
        self.0
            .sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                syn::FnArg::Typed(typed) => {
                    Some(ContentDescriptorOrReference::ContentDescriptorObject(
                        ContentDescriptorObject {
                            name: Self::param_ident(&typed.pat)
                                .ident
                                .to_string()
                                .to_case(Case::Camel),
                            description: None,
                            summary: None,
                            schema: JSONSchema::JsonSchemaObject(RootSchema {
                                // meta_schema: Some(typed.ty.to_token_stream().to_string()),
                                ..Default::default()
                            }),
                            required: Some(Self::param_required(&*typed.ty)),
                            deprecated: None,
                        },
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn param_ident(pat: &Box<Pat>) -> PatIdent {
        match &**pat {
            syn::Pat::Ident(ident) => ident.clone(),
            _ => unreachable!(),
        }
    }

    fn param_required(ty: &Type) -> bool {
        if ty.to_token_stream().to_string().contains("Option") {
            return false;
        }
        true
    }

    fn return_type(&self) -> ContentDescriptorOrReference {
        let ty = match &self.0.sig.output {
            ReturnType::Type(_, ty) => match ty.as_ref() {
                Type::Path(path) => path,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };

        let arg = match &ty
            .path
            .segments
            .first()
            .expect("Type must have at least one segment.")
            .arguments
        {
            PathArguments::AngleBracketed(arg) => arg
                .args
                .first()
                .expect("Type must have at least one argument."),
            _ => unreachable!(),
        };

        let path = match arg {
            GenericArgument::Type(return_type) => match return_type {
                Type::Path(path) => path,
                _ => {
                    unreachable!()
                }
            },
            _ => unreachable!(),
        };
        let ident = path
            .path
            .segments
            .first()
            .expect("Path must have an identity.");

        ContentDescriptorOrReference::ContentDescriptorObject(ContentDescriptorObject {
            name: ident.ident.to_string(),
            description: None,
            summary: None,
            schema: JSONSchema::JsonSchemaObject(RootSchema {
                schema: Self::return_type_schema(&ident),
                ..Default::default()
            }),
            required: None,
            deprecated: None,
        })
    }

    fn return_type_schema(ident: &PathSegment) -> SchemaObject {
        let mut schema = SchemaObject {
            ..Default::default()
        };

        let (is_rust_type, instance_type) = match ident.ident.to_string().as_str() {
            "u32" | "u64" | "usize" | "BoxStream" => (true, InstanceType::Number),
            "Vec" => (true, InstanceType::Array),
            "String" => (true, InstanceType::String),
            "bool" => (true, InstanceType::Boolean),
            _ => (false, InstanceType::Null),
        };

        if is_rust_type {
            schema.instance_type = Some(SingleOrVec::Single(Box::new(instance_type)))
        } else {
            schema.reference = Some(format!("#/components/schemas/{}", ident.ident.to_string()));
        }

        schema
    }
}

impl Into<ParsedItemStruct> for ItemStruct {
    fn into(self) -> ParsedItemStruct {
        ParsedItemStruct(self)
    }
}

impl Into<ParsedTraitItemFn> for TraitItemFn {
    fn into(self) -> ParsedTraitItemFn {
        ParsedTraitItemFn(self)
    }
}

pub fn extract_structs_from_ast(file: &File) -> Vec<ParsedItemStruct> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item_struct) => Some(item_struct.clone().into()),
            _ => None,
        })
        .collect()
}

pub fn extract_fns_from_ast(file: &File) -> Vec<ParsedTraitItemFn> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Trait(trait_item) => Some(trait_item.items.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|trait_item| match trait_item {
            TraitItem::Fn(trait_item_fn) => Some(trait_item_fn.into()),
            _ => None,
        })
        .collect()
}
