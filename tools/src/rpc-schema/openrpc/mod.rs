use std::collections::BTreeMap;

use self::document::{
    Components, ContactObject, InfoObject, LicenseObject, MethodObject, Openrpc, OpenrpcDocument,
};
use serde_json::{Map, Value};

use crate::parser::{ParsedItemStruct, ParsedTraitItemFn};

#[allow(unused)]
pub mod document;

#[derive(Clone)]
pub struct OpenRpcBuilder {
    open_rpc_doc: OpenrpcDocument,
    structs: Vec<ParsedItemStruct>,
    methods: Vec<ParsedTraitItemFn>,
}

impl OpenRpcBuilder {
    pub fn with_schema(mut self, item_struct: &ParsedItemStruct) -> OpenRpcBuilder {
        self.structs.push(item_struct.clone());
        self
    }

    pub fn with_method(mut self, item_fn: &ParsedTraitItemFn) -> OpenRpcBuilder {
        self.methods.push(item_fn.clone());
        self
    }

    pub fn builder() -> OpenRpcBuilder {
        OpenRpcBuilder{ open_rpc_doc: OpenrpcDocument {
            openrpc: Openrpc::V26,
            info: InfoObject {
                title: "Nimiq JSON-RPC Specification".to_string(),
                description: Some("Through the use of JSON-RPC, Nimiq nodes expose a set of standardized methods and endpoints that allow external applications and tools to interact, stream and control the behavior of the nodes. This includes functionalities such as retrieving information about the blockchain state, submitting transactions, managing accounts, and configuring node settings.".to_string()),
                version: "Testnet 0.20.0".to_string(),
                terms_of_service: None,
                contact: Some(ContactObject { name: Some("The Nimiq Foundation".to_string()), email: Some("info@nimiq.com".to_string()), url: Some("https://nimiq.com".to_string()) }),
                license: Some(LicenseObject{ name: Some("Apache License, Version 2.0".to_string()), url: Some("http://www.apache.org/licenses/LICENSE-2.0".to_string()) }),
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
            let mut method = MethodObject::new(m.title(), Some(m.description()));
            method.params = m.params(&self.structs);
            method.result = m.return_type(&self.structs);
            doc.methods.push(method);
        });
        doc.methods.sort_by_key(|m| m.name.clone());

        doc
    }
}
