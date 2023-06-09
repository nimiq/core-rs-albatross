use thiserror::Error;

// #[cfg(feature = "validator")]
// use validator::error::Error as ValidatorError;
#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String), // TODO

    #[cfg(feature = "database-storage")]
    #[error("MDBX error: {0}")]
    Lmdb(#[from] nimiq_database::Error),

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

    #[cfg(feature = "logging")]
    #[error("Logger error: {0}")]
    Logging(#[from] tracing_subscriber::filter::FromEnvError),

    #[cfg(feature = "loki")]
    #[error("Loki logger error: {0}")]
    LoggingLoki(#[from] tracing_loki::Error),

    #[error("Failed to parse multiaddr: {0}")]
    Multiaddr(#[from] nimiq_network_libp2p::libp2p::core::multiaddr::Error),

    #[error("Failed to parse Address: {0}")]
    Address(#[from] nimiq_keys::AddressParseError),

    #[error("Serializing Error: {0}")]
    Serializing(#[from] nimiq_serde::DeserializeError),

    #[error("Nano ZKP Error: {0}")]
    NanoZKP(#[from] nimiq_zkp_primitives::NanoZKPError),
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
