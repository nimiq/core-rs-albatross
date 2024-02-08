use std::collections::HashMap;

use open_rpc_schema::document::{Components, InfoObject, Openrpc, OpenrpcDocument};

use crate::parser::ParsedItemStruct;

#[derive(Clone)]
pub struct OpenRpcBuilder(OpenrpcDocument);

impl OpenRpcBuilder {
    pub fn with_components(mut self) -> OpenRpcBuilder {
        if self.0.components.is_none() {
            self.0.components = Some(Components {
                schemas: Some(HashMap::new()),
                links: None,
                errors: None,
                examples: None,
                example_pairings: None,
                content_descriptors: None,
                tags: None,
            });
        }
        self
    }

    pub fn with_schema(mut self, item_struct: &ParsedItemStruct) -> OpenRpcBuilder {
        self.0
            .components
            .as_mut()
            .expect("Components not initialized. Consider calling with_components first.")
            .schemas
            .as_mut()
            .expect("Component schema not initialized.")
            .insert(item_struct.title(), Some(item_struct.to_schema()));
        self
    }

    pub fn builder() -> OpenRpcBuilder {
        OpenRpcBuilder(OpenrpcDocument {
            openrpc: Openrpc::V26,
            info: InfoObject {
                title: "Nimiq JSON-RPC Specification".to_string(),
                description: Some("Through the use of JSON-RPC, Nimiq nodes expose a set of standardized methods and endpoints that allow external applications and tools to interact, stream and control the behavior of the nodes. This includes functionalities such as retrieving information about the blockchain state, submitting transactions, managing accounts, and configuring node settings.".to_string()),
                version: "0.19.0".to_string(),
                terms_of_service: None,
                contact: None,
                license: None,
            },
            ..Default::default()
        },
    )
    }

    pub fn build(self) -> OpenrpcDocument {
        OpenrpcDocument {
            openrpc: self.0.openrpc,
            info: self.0.info,
            servers: self.0.servers,
            methods: self.0.methods,
            components: self.0.components,
            external_docs: self.0.external_docs,
        }
    }
}
