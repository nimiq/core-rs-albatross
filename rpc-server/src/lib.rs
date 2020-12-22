pub mod dispatchers;
pub mod wallets;
pub mod error;

pub use nimiq_jsonrpc_server::{Config, Server};
pub use error::Error;