pub mod dispatchers;
pub mod error;
pub mod wallets;

pub use error::Error;
pub use nimiq_jsonrpc_server::{Config, Server};
