use async_trait::async_trait;

use crate::error::Error;

use nimiq_rpc_interface::validator::ValidatorInterface;

pub struct ValidatorDispatcher {
}

impl ValidatorDispatcher {
    pub fn new() -> Self {
        Self {
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl ValidatorInterface for ValidatorDispatcher {
    type Error = Error;
}
