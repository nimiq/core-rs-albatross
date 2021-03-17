use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum NanoError {
    #[error("Can't find block for account proof.")]
    MissingBlock,
    #[error("Block has no body.")]
    MissingBody,
    #[error("Block is either wrong type or has no body.")]
    InvalidBlock,
    #[error("Merkle tree proof is wrong.")]
    WrongProof,
}
