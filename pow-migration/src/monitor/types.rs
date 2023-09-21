use nimiq_primitives::coin::Coin;
use thiserror::Error;

/// Readiness state of all of the validators registered in the PoW chain
pub enum ValidatorsReadiness {
    /// Validators are not ready.
    /// Encodes the stake that is ready in the inner type.
    NotReady(Coin),
    /// Validators are ready.
    /// Encodes the stake that is ready in the inner type.
    Ready(Coin),
}

/// Error types that can be returned by the monitor
#[derive(Error, Debug)]
pub enum Error {
    /// RPC error
    #[error("RPC error: {0}")]
    Rpc(#[from] jsonrpsee::core::Error),
}
