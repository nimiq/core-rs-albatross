use json::JsonValue;

pub mod wallet;
pub mod network;
pub mod blockchain;
pub mod blockchain_nimiq;
pub mod blockchain_albatross;
pub mod mempool;
pub mod mempool_albatross;
pub mod block_production;

pub trait Handler: Send + Sync {
    fn call(&self, name: &str, params: &[JsonValue]) -> Option<Result<JsonValue, JsonValue>>;
}
