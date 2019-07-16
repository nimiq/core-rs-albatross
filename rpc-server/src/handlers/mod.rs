use json::Array;
use json::JsonValue;

pub mod wallet;
pub mod network;
pub mod generic_blockchain;
pub mod blockchain_nimiq;
pub mod blockchain_albatross;
pub mod mempool;
pub mod block_production;

pub trait Handler: Send + Sync {
    fn call(&self, name: &str, params: &Array) -> Option<Result<JsonValue, JsonValue>>;
}
