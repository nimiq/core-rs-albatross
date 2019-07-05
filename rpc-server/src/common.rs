use std::sync::Arc;

use json::{Array, JsonValue};

use crate::JsonRpcConfig;
use crate::error::AuthenticationError;
use crate::handlers::Handler;
use crate::jsonrpc;

pub struct RpcHandler {
    pub handlers: Vec<Box<dyn Handler>>,
    pub config: Arc<JsonRpcConfig>,
}

impl jsonrpc::Handler for RpcHandler {
    fn call_method(&self, name: &str, params: Array) -> Option<Result<JsonValue, JsonValue>> {
        trace!("RPC method called: {}", name);

        if !self.config.methods.is_empty() && !self.config.methods.contains(name) {
            info!("RPC call to black-listed method: {}", name);
            //return Some(|_, _| Err(object!("message" => "Method is not allowed.")))
            return None
        }

        self.handlers.iter().find_map(|h| h.call(name, &params))
    }

    fn authorize(&self, username: &str, password: &str) -> Result<(), AuthenticationError> {
        if !self.config.credentials.as_ref().map(|c| c.check(username, password)).unwrap_or(true) {
            return Err(AuthenticationError::IncorrectCredentials);
        }
        Ok(())
    }
}
