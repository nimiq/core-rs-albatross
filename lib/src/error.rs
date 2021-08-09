use thiserror::Error;

use nimiq_database::volatile::VolatileDatabaseError;

// #[cfg(feature = "validator")]
// use validator::error::Error as ValidatorError;
#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String), // TODO

    #[error("LMDB error: {0}")]
    Lmdb(#[from] nimiq_database::lmdb::LmdbError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error: {0}")]
    Network(#[from] nimiq_network_libp2p::NetworkError),

    #[error("File store error: {0}")]
    FileStore(#[from] nimiq_utils::file_store::Error),

    #[error("Consensus error: {0}")]
    Consensus(#[from] nimiq_consensus::Error),

    #[error("Config file parsing error: {0}")]
    Toml(#[from] toml::de::Error),

    // #[cfg(feature = "validator")]
    // #[error("Validator error: {0}")]
    // Validator(#[from] ValidatorError),
    #[cfg(feature = "rpc-server")]
    #[error("RPC server error: {0}")]
    RpcServer(#[from] nimiq_rpc_server::Error),

    #[error("Logger error: {0}")]
    Logging(#[from] log::SetLoggerError),

    #[error("Failed to parse multiaddr: {0}")]
    Multiaddr(#[from] nimiq_network_libp2p::libp2p::core::multiaddr::Error),
}

impl Error {
    /// Constructs a configuration error from an error message.
    ///
    /// # Arguments
    ///
    /// * msg - The error message
    ///
    pub fn config_error<S: AsRef<str>>(msg: S) -> Self {
        Self::Config(msg.as_ref().to_string())
    }
}

impl From<VolatileDatabaseError> for Error {
    fn from(e: VolatileDatabaseError) -> Self {
        match e {
            VolatileDatabaseError::IoError(e) => e.into(),
            VolatileDatabaseError::LmdbError(e) => e.into(),
        }
    }
}
