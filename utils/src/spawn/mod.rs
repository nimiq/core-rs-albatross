#[cfg(not(target_family = "wasm"))]
mod tokio;
#[cfg(target_family = "wasm")]
mod wasm;

#[cfg(not(target_family = "wasm"))]
pub use tokio::*;
#[cfg(target_family = "wasm")]
pub use wasm::*;
