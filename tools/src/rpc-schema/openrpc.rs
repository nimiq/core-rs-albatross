use open_rpc::{Components, Info, OpenRpc, Schema};

use crate::parser::ParsedItemStruct;

pub struct OpenRpcBuilder(OpenRpc);

impl OpenRpcBuilder {
    pub fn open_rpc_version(mut self, open_rpc_version: String) -> OpenRpcBuilder {
        self.0.openrpc = open_rpc_version;
        self
    }

    pub fn api_version(mut self, version: String) -> OpenRpcBuilder {
        self.0.info.version = version;
        self
    }

    pub fn title(mut self, title: String) -> OpenRpcBuilder {
        self.0.info.title = title;
        self
    }

    pub fn description(mut self, description: String) -> OpenRpcBuilder {
        self.0.info.description = Some(description);
        self
    }

    pub fn add_schema(mut self, item_struct: &ParsedItemStruct) -> OpenRpcBuilder {
        self.0
            .components
            .as_mut()
            .expect("OpenRPC components not initialized.")
            .schemas
            .insert(
                item_struct.ident().to_string(),
                Schema {
                    title: Some(item_struct.title()),
                    description: Some(item_struct.description()),
                    contents: item_struct.fields_to_open_rpc_schema_contents(),
                },
            );

        self
    }

    pub fn with_components(mut self) -> OpenRpcBuilder {
        if self.0.components.is_none() {
            self.0.components = Some(Components {
                content_descriptors: Default::default(),
                schemas: Default::default(),
                examples: Default::default(),
                links: Default::default(),
                errors: Default::default(),
                example_pairings: Default::default(),
                tags: Default::default(),
            });
        }

        self
    }

    pub fn builder() -> OpenRpcBuilder {
        OpenRpcBuilder(OpenRpc {
            openrpc: "1.2.6".to_string(),
            info: Info {
                title: "Nimiq JSON-RPC Specification".to_string(),
                description: Some("Through the use of JSON-RPC, Nimiq nodes expose a set of standardized methods and endpoints that allow external applications and tools to interact, stream and control the behavior of the nodes. This includes functionalities such as retrieving information about the blockchain state, submitting transactions, managing accounts, and configuring node settings.".to_string()),
                terms_of_service: Default::default(),
                contact: Default::default(),
                license: Default::default(),
                version: "0.0.1".to_string(),
            },
            servers: Default::default(),
            methods: Default::default(),
            components: Default::default(),
            external_docs: Default::default(),
        },
    )
    }

    pub fn build(self) -> OpenRpc {
        OpenRpc {
            openrpc: self.0.openrpc,
            info: self.0.info,
            servers: self.0.servers,
            methods: self.0.methods,
            components: self.0.components,
            external_docs: self.0.external_docs,
        }
    }
}
