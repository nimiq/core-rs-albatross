use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Couldn't create directory: {0}")]
    CreateDirectory(#[from] std::io::Error),
    #[error("Mdbx error: {0}")]
    Mdbx(#[from] libmdbx::Error),
}
