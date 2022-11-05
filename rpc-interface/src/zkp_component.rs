use async_trait::async_trait;

use crate::types::ZKPState;

#[nimiq_jsonrpc_derive::proxy(name = "ZKPComponentProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ZKPComponentInterface {
    type Error;

    async fn get_zkp_state(&mut self) -> Result<ZKPState, Self::Error>;
}
