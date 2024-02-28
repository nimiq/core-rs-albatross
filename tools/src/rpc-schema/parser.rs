use convert_case::{Case, Casing};
use open_rpc_schema::{
    document::{ContentDescriptorObject, ContentDescriptorOrReference, JSONSchema},
    schemars::schema::{
        ArrayValidation, InstanceType, RootSchema,
        Schema::{self},
        SchemaObject, SingleOrVec,
    },
};
use quote::ToTokens;
use serde_json::{Map, Value};
use syn::{
    Field, File, GenericArgument, Ident, ItemStruct, Pat, PatIdent, Path, PathArguments,
    PathSegment, ReturnType, TraitItem, TraitItemFn, Type,
};

#[derive(Clone)]
pub struct ParsedItemStruct(ItemStruct);
#[derive(Clone)]
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

    pub fn properties(&self, structs: &[ParsedItemStruct]) -> Value {
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
                let schema_ref = structs.iter().find(|s| inner_type.1 == s.title());
                prop_fields.append(&mut Self::param_to_json_type(field, schema_ref));

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

                if ident == "Vec" {
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
                } else if let Some(reference) = schema_ref {
                    map.insert(
                        "$ref".into(),
                        Value::String(format!("#/components/schemas/{}", reference.title())),
                    );
                } else {
                    map.insert("type".into(), Self::map_type(&path));
                }
            }
            _ => unreachable!(),
        }

        map
    }

    fn map_type(path: &Path) -> Value {
        let (_, inner_ident) = Self::unwrap_type(path.clone(), true);
        match inner_ident.to_string().as_str() {
            "Address"
            | "Blake2bHash"
            | "Blake2sHash"
            | "CompressedPublicKey"
            | "Ed25519PublicKey"
            | "NetworkId"
            | "PrivateKey"
            | "Ed25519Signature"
            | "String"
            | "VrfSeed" => Value::String("string".into()),
            "u8" | "u16" | "u32" | "u64" | "usize" | "Coin" => Value::String("number".into()),
            "bool" => Value::String("boolean".into()),
            "AccountAdditionalFields"
            | "BitSet"
            | "S"
            | "T"
            | "BlockAdditionalFields"
            | "MultiSignature" => Value::String("object".into()),
            _ => panic!("{}", inner_ident),
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
                    if let GenericArgument::Type(Type::Path(inner_type)) =
                        outer_type.args.first().unwrap()
                    {
                        if recursive {
                            return Self::unwrap_type(inner_type.path.clone(), true);
                        }

                        (
                            inner_type.path.clone(),
                            inner_type.path.segments.first().unwrap().ident.clone(),
                        )
                    } else {
                        (path, ident)
                    }
                } else {
                    (path, ident)
                }
            }
            _ => (path, ident),
        }
    }
}

impl ParsedTraitItemFn {
    #[inline]
    pub fn title(&self) -> String {
        self.0.sig.ident.to_string().to_case(Case::Camel)
    }

    #[inline]
    pub fn description(&self) -> String {
        "".into()
    }

    pub fn params(&self, structs: &[ParsedItemStruct]) -> Vec<ContentDescriptorOrReference> {
        self.0
            .sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                syn::FnArg::Typed(typed) => {
                    let segment = match *typed.ty.clone() {
                        Type::Path(p) => p.path.segments.first().unwrap().clone(),
                        _ => unreachable!(),
                    };

                    let (inner_segment, inner_type) = Self::unwrap_type(&segment);
                    let schema_ref = structs.iter().find(|s| inner_type == s.title());

                    Some(ContentDescriptorOrReference::ContentDescriptorObject(
                        ContentDescriptorObject {
                            name: Self::param_ident(&typed.pat)
                                .ident
                                .to_string()
                                .to_case(Case::Camel),
                            description: None,
                            summary: None,
                            schema: JSONSchema::JsonSchemaObject(RootSchema {
                                schema: Self::return_type_schema(&inner_segment, schema_ref),
                                ..Default::default()
                            }),
                            required: Some(Self::param_required(&typed.ty)),
                            deprecated: None,
                        },
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn param_ident(pat: &Pat) -> PatIdent {
        match pat {
            syn::Pat::Ident(ident) => ident.clone(),
            _ => unreachable!(),
        }
    }

    fn param_required(_ty: &Type) -> bool {
        // At the moment, all params are required even if the type is wrapped in an Option.
        // if ty.to_token_stream().to_string().contains("Option") {
        //     return false;
        // }
        true
    }

    pub fn return_type(&self, structs: &[ParsedItemStruct]) -> ContentDescriptorOrReference {
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
                Type::Tuple(_) => {
                    return ContentDescriptorOrReference::ContentDescriptorObject(
                        ContentDescriptorObject {
                            name: "null".to_string(),
                            description: None,
                            summary: None,
                            schema: JSONSchema::JsonSchemaObject(RootSchema {
                                schema: SchemaObject {
                                    instance_type: Some(SingleOrVec::Single(Box::new(
                                        InstanceType::Null,
                                    ))),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }),
                            required: None,
                            deprecated: None,
                        },
                    )
                }
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

        let inner_type = Self::unwrap_type(ident);
        let schema_ref = structs.iter().find(|s| inner_type.1 == s.title());

        ContentDescriptorOrReference::ContentDescriptorObject(ContentDescriptorObject {
            name: ident.ident.to_string(),
            description: None,
            summary: None,
            schema: JSONSchema::JsonSchemaObject(RootSchema {
                schema: Self::return_type_schema(ident, schema_ref),
                ..Default::default()
            }),
            required: None,
            deprecated: None,
        })
    }

    fn return_type_schema(
        ident: &PathSegment,
        schema_ref: Option<&ParsedItemStruct>,
    ) -> SchemaObject {
        let mut schema = SchemaObject {
            ..Default::default()
        };

        let (is_rust_type, instance_type) = Self::to_instance_type(&ident.ident);

        if is_rust_type {
            schema.instance_type = Some(SingleOrVec::Single(Box::new(instance_type)));
        } else {
            schema.reference = Some(format!("#/components/schemas/{}", ident.ident));
        }

        if instance_type == InstanceType::Array {
            let inner_type = Self::unwrap_type(ident);
            let inner_instance_type = Self::to_instance_type(&inner_type.1);
            let mut inner_schema = SchemaObject {
                ..Default::default()
            };

            if schema_ref.is_some() {
                inner_schema.reference = Some(format!("#/components/schemas/{}", inner_type.1));
            } else {
                inner_schema.instance_type =
                    Some(SingleOrVec::Single(Box::new(inner_instance_type.1)))
            }

            schema.array = Some(Box::new(ArrayValidation {
                items: Some(SingleOrVec::Single(Box::new(Schema::Object(inner_schema)))),
                ..Default::default()
            }));
        }

        schema
    }

    fn to_instance_type(ident: &Ident) -> (bool, InstanceType) {
        match ident.to_string().as_str() {
            "u8"
            | "u16"
            | "u32"
            | "u64"
            | "f64"
            | "usize"
            | "BoxStream"
            | "ValidityStartHeight"
            | "Coin" => (true, InstanceType::Number),
            "Vec" | "LogType" => (true, InstanceType::Array),
            "String" | "AnyHash" | "Ed25519Signature" | "Ed25519PublicKey" | "PreImage"
            | "Blake2bHash" | "Address" => (true, InstanceType::String),
            "bool" => (true, InstanceType::Boolean),
            _ => (false, InstanceType::Object),
        }
    }

    fn unwrap_type(path_segment: &PathSegment) -> (PathSegment, Ident) {
        let ident = path_segment.ident.clone();

        match ident.to_string().as_str() {
            "Vec" | "Option" => {
                if let PathArguments::AngleBracketed(outer_type) = &path_segment.arguments {
                    if let GenericArgument::Type(Type::Path(inner_type)) =
                        outer_type.args.first().unwrap()
                    {
                        let segment = inner_type.path.segments.first().unwrap();
                        return (segment.to_owned(), segment.ident.clone());
                    }
                    unreachable!()
                }
                (path_segment.to_owned(), ident)
            }
            _ => (path_segment.to_owned(), ident),
        }
    }
}

impl From<ItemStruct> for ParsedItemStruct {
    fn from(val: ItemStruct) -> Self {
        ParsedItemStruct(val)
    }
}

impl From<TraitItemFn> for ParsedTraitItemFn {
    fn from(val: TraitItemFn) -> Self {
        ParsedTraitItemFn(val)
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
