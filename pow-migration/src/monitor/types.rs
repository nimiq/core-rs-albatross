use nimiq_primitives::coin::Coin;
use thiserror::Error;

pub const ACTIVATION_HEIGHT: u32 = 100;
pub enum ValidatorsReadiness {
    NotReady(Coin),
    Ready(Coin),
}

#[derive(Error, Debug)]
pub enum Error {
    /// RPC error
    #[error("RPC error")]
    Rpc,
}
