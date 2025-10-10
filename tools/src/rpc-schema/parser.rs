use std::collections::{BTreeMap, HashSet};

use convert_case::{Case, Casing};
use quote::ToTokens;
use schemars::{json_schema, Schema};
use serde_json::{json, Map, Value};
use syn::{
    Attribute, Field, Fields, File, GenericArgument, Ident, ItemEnum, ItemStruct, Pat, PatIdent,
    Path, PathArguments, PathSegment, ReturnType, TraitItem, TraitItemFn, Type,
};

use crate::openrpc::document::{ContentDescriptorObject, ContentDescriptorOrReference, JSONSchema};

/// Represents a parsed Rust struct.
#[derive(Clone)]
pub struct ParsedItemStruct(ItemStruct);
/// Represents a parsed Rust trait method.
#[derive(Clone)]
pub struct ParsedTraitItemFn(TraitItemFn);
/// Represents a parsed Rust enum.
#[derive(Clone)]
pub struct ParsedItemEnum(ItemEnum);

impl ParsedItemStruct {
    /// Retrieves the name of a Rust struct.
    #[inline]
    pub fn title(&self) -> String {
        self.0.ident.to_string()
    }

    /// Retrieves the description of a Rust struct based on the Rustdoc.
    #[inline]
    pub fn description(&self) -> String {
        "".into()
    }

    /// Generates the properties for the Rust struct in the form of a JSON Object.
    pub fn properties(&self, structs: &[ParsedItemStruct], enums: &[ParsedItemEnum]) -> Value {
        let mut props: Map<String, Value> = self
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

                // Check if field has #[serde(flatten)] - skip it, will be expanded below
                if Self::has_serde_flatten(field) {
                    let type_name = inner_type.1.to_string();
                    if enums.iter().any(|e| e.title() == type_name) {
                        return None;
                    }
                }

                let mut prop_fields = Map::new();
                let field_name = field.ident.to_token_stream().to_string();
                let field_ident = if field_name.starts_with('_') {
                    field_name
                } else {
                    field_name.to_case(Case::Camel)
                };

                prop_fields.insert("title".into(), Value::String(field_ident.clone()));
                prop_fields.append(&mut Self::param_to_json_type(field, structs));

                Some((field_ident, Value::Object(prop_fields)))
            })
            .collect();

        // Add discriminator and variant fields for flattened tagged enums
        for field in &self.0.fields {
            if !Self::has_serde_flatten(field) {
                continue;
            }

            if let Type::Path(path) = &field.ty {
                let inner_type = Self::unwrap_type(path.path.clone(), true);
                let type_name = inner_type.1.to_string();

                if let Some(enum_def) = enums.iter().find(|e| e.title() == type_name) {
                    // Add discriminator field (if enum is tagged)
                    if let Some(tag_name) = enum_def.discriminator_key() {
                        let mut type_field = Map::new();
                        type_field.insert("title".into(), Value::String(tag_name.clone()));
                        type_field.insert("type".into(), Value::String("string".into()));
                        type_field
                            .insert("enum".into(), json!(enum_def.get_discriminator_values()));
                        props.insert(tag_name, Value::Object(type_field));
                    }

                    // Add all variant fields (union of all variants)
                    for (field_name, field_types) in enum_def.get_all_variant_fields() {
                        let mut field_schema = Map::new();
                        field_schema.insert("title".into(), Value::String(field_name.clone()));

                        let mut seen = HashSet::new();
                        let mut schemas = Vec::new();
                        for field_type in field_types {
                            let schema_map = Self::type_to_schema(&field_type, structs);
                            let serialized = serde_json::to_string(&schema_map).unwrap();
                            if seen.insert(serialized) {
                                schemas.push(schema_map);
                            }
                        }

                        if schemas.len() == 1 {
                            field_schema.extend(schemas.into_iter().next().unwrap());
                        } else if !schemas.is_empty() {
                            let any_of = schemas.into_iter().map(Value::Object).collect();
                            field_schema.insert("anyOf".into(), Value::Array(any_of));
                        }

                        props.insert(field_name, Value::Object(field_schema));
                    }
                }
            }
        }

        Value::Object(props)
    }

    /// Check if a field has #[serde(flatten)] attribute
    fn has_serde_flatten(field: &Field) -> bool {
        field.attrs.iter().any(|attr| {
            attr.path().is_ident("serde")
                && attr.meta.to_token_stream().to_string().contains("flatten")
        })
    }

    /// Converts a Rust struct property to JSON type.
    fn param_to_json_type(param: &Field, structs: &[ParsedItemStruct]) -> Map<String, Value> {
        Self::type_to_schema(&param.ty, structs)
    }

    fn type_to_schema(ty: &Type, structs: &[ParsedItemStruct]) -> Map<String, Value> {
        let mut map = Map::new();
        match ty {
            Type::Path(type_path) => {
                let mut path = type_path.path.clone();
                let mut ident = path
                    .segments
                    .first()
                    .expect("Function parameter should have a segment")
                    .ident
                    .clone();

                if ident == "Option" {
                    (path, ident) = Self::unwrap_type(type_path.path.clone(), false);
                }

                let inner_for_ref = Self::unwrap_type(path.clone(), true);
                let schema_ref = structs.iter().find(|s| inner_for_ref.1 == s.title());

                if ident == "Vec" || ident == "BitSet" {
                    let mut items_map = Map::new();
                    if let Some(reference) = schema_ref {
                        items_map.insert(
                            "$ref".into(),
                            Value::String(format!("#/components/schemas/{}", reference.title())),
                        );
                    } else if ident == "BitSet" {
                        items_map.insert("type".into(), Value::String("number".into()));
                    } else {
                        items_map.insert("type".into(), Self::map_type(&path));
                    }

                    map.insert("type".into(), Value::String("array".into()));
                    map.insert("items".into(), Value::Object(items_map));
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

    /// Maps a Rust type to a JSON type.
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
            "BitSet" => Value::String("array".into()),
            "AccountAdditionalFields"
            | "BTreeSet"
            | "S"
            | "T"
            | "BlockAdditionalFields"
            | "EquivocationProof"
            | "Inherent"
            | "BlockLog"
            | "AnyHash"
            | "MultiSignature"
            | "TendermintProof"
            | "MicroJustification" => Value::String("object".into()),
            _ => panic!("{}", inner_ident),
        }
    }

    /// Creates a list of Rust struct properties that are mandatory.
    /// This is determined by checking if the type is wrapped in an `Option<T>`.
    pub fn required_fields(&self, enums: &[ParsedItemEnum]) -> Vec<Value> {
        let mut required: Vec<Value> = self
            .0
            .fields
            .iter()
            .filter_map(|field| {
                if field.ty.to_token_stream().to_string().contains("Option") {
                    return None;
                }

                // Skip flattened enum fields - discriminator will be added below
                if Self::has_serde_flatten(field)
                    && let Type::Path(path) = &field.ty
                {
                    let inner_type = Self::unwrap_type(path.path.clone(), true);
                    let type_name = inner_type.1.to_string();
                    if enums.iter().any(|e| e.title() == type_name) {
                        return None;
                    }
                }

                let field_name = field.ident.to_token_stream().to_string();
                Some(Value::String(if field_name.starts_with('_') {
                    field_name
                } else {
                    field_name.to_case(Case::Camel)
                }))
            })
            .collect();

        // Add discriminator(s) for flattened tagged enums
        let mut added = HashSet::new();
        for field in &self.0.fields {
            if !Self::has_serde_flatten(field) {
                continue;
            }

            if let Type::Path(path) = &field.ty {
                let inner_type = Self::unwrap_type(path.path.clone(), true);
                let type_name = inner_type.1.to_string();
                if let Some(enum_def) = enums.iter().find(|e| e.title() == type_name)
                    && let Some(tag_name) = enum_def.discriminator_key()
                    && added.insert(tag_name.clone())
                {
                    required.push(Value::String(tag_name));
                }
            }
        }

        required
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
    /// Retrieves the name of a Rust trait method.
    #[inline]
    pub fn title(&self) -> String {
        self.0.sig.ident.to_string().to_case(Case::Camel)
    }

    /// Retrieves the description of a Rust trait method based on the Rustdoc.
    #[inline]
    pub fn description(&self) -> String {
        self.0
            .attrs
            .iter()
            .fold(String::new(), |acc, attr| {
                if attr.path().is_ident("doc") {
                    return acc + &attr.to_token_stream().to_string();
                }
                acc
            })
            .replace("# [doc = \"", "")
            .replace("\"]", "")
            .trim_start()
            .into()
    }

    /// Generates a list of parameters that accepted by the Rust trait method.
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
                            schema: JSONSchema::JsonSchemaObject(Self::return_type_schema(
                                &inner_segment,
                                schema_ref,
                            )),
                            required: Some(Self::param_required(&typed.ty)),
                            deprecated: None,
                        },
                    ))
                }
                _ => None,
            })
            .collect()
    }

    /// Extract the identity of a parameter.
    fn param_ident(pat: &Pat) -> PatIdent {
        match pat {
            Pat::Ident(ident) => ident.clone(),
            _ => unreachable!(),
        }
    }

    /// Determines whether a Rust trait method parameter is required.
    fn param_required(_ty: &Type) -> bool {
        // All params required in JSON-RPC array, even Option<T> (can be null but must be present)
        true
    }

    /// Generates a content descriptor containing the base information about the return type
    /// of the Rust trait method.
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
                            schema: JSONSchema::JsonSchemaObject(json_schema!({
                                "type": "null"
                            })),
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
            schema: JSONSchema::JsonSchemaObject(Self::return_type_schema(ident, schema_ref)),
            required: None,
            deprecated: None,
        })
    }

    /// Generates the schema object for the return type of the Rust trait method.
    fn return_type_schema(ident: &PathSegment, schema_ref: Option<&ParsedItemStruct>) -> Schema {
        let (is_rust_type, json_type) = Self::to_json_type(&ident.ident);

        // Special case for ValidityStartHeight which accepts both string and number
        if ident.ident == "ValidityStartHeight" {
            let schema_value = json!({
                "oneOf": [
                    { "type": "string" },
                    { "type": "number" }
                ]
            });
            return serde_json::from_value(schema_value).unwrap();
        }

        if is_rust_type {
            if json_type == "array" {
                // Check if the inner type is a tuple (e.g., Vec<(Type1, Type2)>)
                if let PathArguments::AngleBracketed(args) = &ident.arguments
                    && let Some(GenericArgument::Type(Type::Tuple(tuple))) = args.args.first()
                {
                    // Handle tuple case: generate schema for array of tuples
                    let mut properties = Map::new();
                    for (idx, elem) in tuple.elems.iter().enumerate() {
                        if let Type::Path(elem_path) = elem {
                            let elem_ident = elem_path
                                .path
                                .segments
                                .first()
                                .expect("Tuple element must have an identity.");
                            let (_, json_elem_type) = Self::to_json_type(&elem_ident.ident);
                            properties.insert(
                                idx.to_string(),
                                json_schema!({
                                    "type": json_elem_type
                                })
                                .into(),
                            );
                        }
                    }
                    return json_schema!({
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": properties
                        }
                    });
                }

                // Handle non-tuple cases
                let inner_type = Self::unwrap_type(ident);
                let (_inner_is_rust_type, inner_json_type) = Self::to_json_type(&inner_type.1);

                if schema_ref.is_some() {
                    json_schema!({
                        "type": "array",
                        "items": {
                            "$ref": format!("#/components/schemas/{}", inner_type.1)
                        }
                    })
                } else {
                    let final_inner_type = if inner_json_type == "array" {
                        "string"
                    } else {
                        inner_json_type
                    };
                    json_schema!({
                        "type": "array",
                        "items": {
                            "type": final_inner_type
                        }
                    })
                }
            } else {
                json_schema!({
                    "type": json_type
                })
            }
        } else {
            json_schema!({
                "$ref": format!("#/components/schemas/{}", ident.ident)
            })
        }
    }

    /// Converts a Rust type to a JSON type string.
    fn to_json_type(ident: &Ident) -> (bool, &'static str) {
        match ident.to_string().as_str() {
            "u8"
            | "u16"
            | "u32"
            | "u64"
            | "f64"
            | "usize"
            | "BoxStream"
            | "ValidityStartHeight"
            | "Coin" => (true, "number"),
            "Vec" | "LogType" => (true, "array"),
            "String" | "AnyHash" | "Ed25519Signature" | "Ed25519PublicKey" | "PreImage"
            | "Blake2bHash" | "Address" => (true, "string"),
            "bool" => (true, "boolean"),
            _ => (false, "object"),
        }
    }

    /// Unwrap a type if it is wrapped. This method turns an `Option<String>` into a String for example.
    fn unwrap_type(path_segment: &PathSegment) -> (PathSegment, Ident) {
        let ident = path_segment.ident.clone();

        match ident.to_string().as_str() {
            "Vec" | "Option" => {
                if let PathArguments::AngleBracketed(outer_type) = &path_segment.arguments
                    && let Some(GenericArgument::Type(Type::Path(inner_type))) =
                        outer_type.args.first()
                {
                    let segment = inner_type.path.segments.first().unwrap();
                    return (segment.to_owned(), segment.ident.clone());
                }

                // For non-path types (e.g., tuples like Vec<(Blake2bHash, u32)>),
                // return the wrapper as-is since we can't extract a simple inner type
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

impl From<ItemEnum> for ParsedItemEnum {
    fn from(val: ItemEnum) -> Self {
        ParsedItemEnum(val)
    }
}

impl ParsedItemEnum {
    /// Retrieves the name of a Rust enum.
    pub fn title(&self) -> String {
        self.0.ident.to_string()
    }

    /// Get discriminator enum values from variants, applying serde rename rules
    fn discriminator_key(&self) -> Option<String> {
        Self::get_serde_tag(&self.0.attrs)
    }

    fn get_discriminator_values(&self) -> Vec<String> {
        let enum_rename_all = Self::get_serde_rename_all(&self.0.attrs);
        self.0
            .variants
            .iter()
            .map(|variant| {
                if let Some(rename) = Self::get_serde_rename(&variant.attrs) {
                    rename
                } else {
                    let variant_rename_all = Self::get_serde_rename_all(&variant.attrs);
                    Self::apply_rename(
                        &variant.ident.to_string(),
                        variant_rename_all.as_deref().or(enum_rename_all.as_deref()),
                    )
                }
            })
            .collect()
    }

    /// Get all fields from all variants including duplicates. Returned map keeps deterministic order.
    fn get_all_variant_fields(&self) -> BTreeMap<String, Vec<Type>> {
        let mut fields: BTreeMap<String, Vec<Type>> = BTreeMap::new();
        let enum_rename_all = Self::get_serde_rename_all(&self.0.attrs);

        for variant in &self.0.variants {
            let variant_rename_all = Self::get_serde_rename_all(&variant.attrs);
            if let Fields::Named(variant_fields) = &variant.fields {
                for field in &variant_fields.named {
                    let field_name = Self::resolve_field_name(
                        field,
                        enum_rename_all.as_deref(),
                        variant_rename_all.as_deref(),
                    );
                    fields.entry(field_name).or_default().push(field.ty.clone());
                }
            }
        }

        fields
    }

    /// Extract rename_all strategy from serde attribute
    fn get_serde_rename_all(attrs: &[Attribute]) -> Option<String> {
        Self::extract_serde_value(attrs, "rename_all")
    }

    fn get_serde_rename(attrs: &[Attribute]) -> Option<String> {
        Self::extract_serde_value(attrs, "rename")
    }

    fn get_serde_tag(attrs: &[Attribute]) -> Option<String> {
        Self::extract_serde_value(attrs, "tag")
    }

    fn extract_serde_value(attrs: &[Attribute], key: &str) -> Option<String> {
        for attr in attrs {
            if attr.path().is_ident("serde") {
                let tokens = attr.meta.to_token_stream().to_string();
                let needle = format!("{key} = \"");
                if let Some(start) = tokens.find(&needle) {
                    let start = start + needle.len();
                    if let Some(end) = tokens[start..].find('"') {
                        return Some(tokens[start..start + end].to_string());
                    }
                }
            }
        }
        None
    }

    fn resolve_field_name(
        field: &Field,
        enum_rename_all: Option<&str>,
        variant_rename_all: Option<&str>,
    ) -> String {
        if let Some(rename) = Self::get_serde_rename(&field.attrs) {
            return rename;
        }

        let field_rename_all = Self::get_serde_rename_all(&field.attrs);
        let rename_rule = field_rename_all
            .as_deref()
            .or(variant_rename_all)
            .or(enum_rename_all);

        let ident = field
            .ident
            .as_ref()
            .expect("Named fields must have an ident")
            .to_string();
        Self::apply_rename(&ident, rename_rule)
    }

    /// Apply serde rename rule to a field/variant name
    fn apply_rename(name: &str, rename_all: Option<&str>) -> String {
        match rename_all {
            Some("camelCase") => name.to_case(Case::Camel),
            Some("kebab-case") => name.to_case(Case::Kebab),
            Some("snake_case") => name.to_case(Case::Snake),
            Some("SCREAMING_SNAKE_CASE") => name.to_case(Case::UpperSnake),
            Some("PascalCase") => name.to_case(Case::Pascal),
            _ => name.to_string(),
        }
    }
}

/// Filter a syntrax tree and only extract the Rust struct definitions.
pub fn extract_structs_from_ast(file: &File) -> Vec<ParsedItemStruct> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item_struct) => Some(item_struct.clone().into()),
            _ => None,
        })
        .collect()
}

/// Filter a syntax tree and only extract the Rust enum definitions.
pub fn extract_enums_from_ast(file: &File) -> Vec<ParsedItemEnum> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item_enum) => Some(item_enum.clone().into()),
            _ => None,
        })
        .collect()
}

/// Filter a syntrax tree and only extract the Rust methods within a trait definition.
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
