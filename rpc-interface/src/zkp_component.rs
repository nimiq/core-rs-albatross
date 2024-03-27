use async_trait::async_trait;

use crate::types::{RPCResult, ZKPState};

#[nimiq_jsonrpc_derive::proxy(name = "ZKPComponentProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ZKPComponentInterface {
    type Error;

    /// Returns the current ZKP state (proof with its related block hash and block number).
    async fn get_zkp_state(&mut self) -> RPCResult<ZKPState, (), Self::Error>;
}
