use std::collections::BTreeMap;

use serde_json::{Map, Value};

use self::document::{
    Components, ContactObject, InfoObject, LicenseObject, MethodObject, Openrpc, OpenrpcDocument,
    TagObject, TagOrReference,
};
use crate::parser::{ParsedItemStruct, ParsedTraitItemFn};

#[allow(unused)]
pub mod document;

/// A builder for constructing an OpenRPC document.
#[derive(Clone)]
pub struct OpenRpcBuilder {
    open_rpc_doc: OpenrpcDocument,
    structs: Vec<ParsedItemStruct>,
    methods: Vec<(ParsedTraitItemFn, String)>,
}

impl OpenRpcBuilder {
    /// Adds a schema based on a Rust struct to the OpenRPC document.
    pub fn with_schema(mut self, item_struct: &ParsedItemStruct) -> OpenRpcBuilder {
        self.structs.push(item_struct.clone());
        self
    }

    /// Adds a method based on a Rust trait method to the OpenRPC document.
    pub fn with_method(mut self, item_fn: &ParsedTraitItemFn, module: String) -> OpenRpcBuilder {
        self.methods.push((item_fn.clone(), module));
        self
    }

    /// Sets the title of the OpenRPC document.
    pub fn title(mut self, title: String) -> OpenRpcBuilder {
        self.open_rpc_doc.info.title = title;
        self
    }

    /// Sets the version of the OpenRPC document.
    pub fn version(mut self, version: String) -> OpenRpcBuilder {
        self.open_rpc_doc.info.version = version;
        self
    }

    /// Creates a new instance of OpenRpcBuilder with default settings and returns it.
    pub fn builder() -> OpenRpcBuilder {
        OpenRpcBuilder{ open_rpc_doc: OpenrpcDocument {
            openrpc: Openrpc::V26,
            info: InfoObject {
                description: Some("Through the use of JSON-RPC, Nimiq nodes expose a set of standardized methods and endpoints that allow external applications and tools to interact, stream and control the behavior of the nodes. This includes functionalities such as retrieving information about the blockchain state, submitting transactions, managing accounts, and configuring node settings.".to_string()),
                contact: Some(ContactObject { name: Some(env!("CARGO_PKG_AUTHORS").to_string()), email: Some("".to_string()), url: Some(env!("CARGO_PKG_HOMEPAGE").to_string()) }),
                license: Some(LicenseObject{ name: Some(env!("CARGO_PKG_LICENSE").to_string()), url: Some("".to_string()) }),
                ..Default::default()
            },
            components: Some(Components {
                schemas: Some(BTreeMap::new()),
                links: None,
                errors: None,
                examples: None,
                example_pairings: Some(BTreeMap::new()),
                content_descriptors: None,
                tags: None,
            }),
            ..Default::default()
        }, structs: vec![], methods: vec![]}
    }

    /// Builds the final OpenRPC document and returns constructed OpenrpcDocument.
    pub fn build(self) -> OpenrpcDocument {
        let mut doc = OpenrpcDocument {
            openrpc: self.open_rpc_doc.openrpc,
            info: self.open_rpc_doc.info,
            servers: self.open_rpc_doc.servers,
            methods: self.open_rpc_doc.methods,
            components: self.open_rpc_doc.components,
            external_docs: self.open_rpc_doc.external_docs,
        };

        let schemas = doc
            .components
            .as_mut()
            .expect("Components not initialized.")
            .schemas
            .as_mut()
            .expect("Component schema not initialized.");

        self.structs.iter().for_each(|s| {
            let mut schema = Map::new();
            schema.insert("title".into(), Value::String(s.title()));
            schema.insert("description".into(), Value::String(s.description()));
            schema.insert("required".into(), Value::Array(s.required_fields()));
            schema.insert("properties".into(), s.properties(&self.structs));
            schemas.insert(s.title(), Some(Value::Object(schema)));
        });

        self.methods.iter().for_each(|m| {
            let mut method = MethodObject::new(m.0.title(), Some(m.0.description()));
            method.params = m.0.params(&self.structs);
            method.result = m.0.return_type(&self.structs);
            method.tags = Some(Vec::with_capacity(2));

            if let Some(ref mut tags) = method.tags {
                tags.push(TagOrReference::TagObject(TagObject {
                    name: m.1.clone(),
                    description: None,
                    external_docs: None,
                }));

                if method.name.contains("subscribe") {
                    tags.push(TagOrReference::TagObject(TagObject {
                        name: "stream".to_string(),
                        description: None,
                        external_docs: None,
                    }));
                }
            }

            doc.methods.push(method);
        });
        doc.methods.sort_by_key(|m| m.name.clone());

        doc
    }
}
