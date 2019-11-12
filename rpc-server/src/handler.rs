use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use json::{Array, JsonValue};

use crate::JsonRpcConfig;
use crate::error::AuthenticationError;
use crate::jsonrpc;
use crate::handlers::Module;

pub struct Method {
    f: Box<dyn Fn(&[JsonValue]) -> Result<JsonValue, JsonValue> + Send + Sync>
}

impl Method {
    pub fn new<F>(f: F) -> Self
        where F: Fn(&[JsonValue]) -> Result<JsonValue, JsonValue> + Send + Sync + 'static
    {
        Self { f: Box::new(f) }
    }

    pub fn call(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        (self.f)(params)
    }
}

pub struct Handler {
    pub methods: RwLock<HashMap<&'static str, Method>>,
    pub config: Arc<JsonRpcConfig>,
}

impl Handler {
    pub fn new(config: JsonRpcConfig) -> Self {
        Handler {
            methods: RwLock::new(HashMap::new()),
            config: Arc::new(config),
        }
    }

    pub fn register_method(&self, name: &'static str, method: Method) {
        let previous = self.methods.write().insert(name, method);
        debug_assert!(previous.is_none(), "Trying to re-register method {}", name);
    }

    pub fn add_module<M: Module>(&self, module: M) {
        for (name, method) in module.methods() {
            self.register_method(name, method)
        }
    }
}

impl jsonrpc::Handler for Handler {
    fn call_method(&self, name: &str, params: Array) -> Option<Result<JsonValue, JsonValue>> {
        trace!("RPC method called: {}", name);

        if !self.config.methods.is_empty() && !self.config.methods.contains(name) {
            info!("RPC call to black-listed method: {}", name);
            //return Some(|_, _| Err(object!("message" => "Method is not allowed.")))
            return None
        }

        self.methods.read().get(name).map(|h| h.call(&params))
    }

    fn authorize(&self, username: &str, password: &str) -> Result<(), AuthenticationError> {
        let ok = match (&self.config.username, &self.config.password) {
            (Some(u), Some(p)) => u == username && p == password,
            (None, None) => true,
            _ => false, // Configuration error
        };

        if ok {
            Ok(())
        }
        else {
            Err(AuthenticationError::IncorrectCredentials)
        }
    }
}

