#[cfg(target_family = "wasm")]
mod gloo;
#[cfg(not(target_family = "wasm"))]
mod tokio;

#[cfg(target_family = "wasm")]
pub use gloo::*;
#[cfg(not(target_family = "wasm"))]
pub use tokio::*;
