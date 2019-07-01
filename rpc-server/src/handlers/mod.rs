use json::Array;
use json::JsonValue;

pub mod wallet;
pub mod network;
pub mod blockchain;
pub mod blockchain_albatross;
pub mod mempool;
pub mod block_production;

pub trait Handler: Send + Sync {
    fn call(&self, name: &str, params: &Array) -> Option<Result<JsonValue, JsonValue>>;
}
