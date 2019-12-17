use std::io::Error as IoError;

use toml::de::Error as TomlError;
use failure::Fail;
use log::SetLoggerError;

use database::lmdb::LmdbError;
use database::volatile::VolatileDatabaseError;
use network::error::Error as NetworkError;
use utils::key_store::Error as KeyStoreError;
use consensus::Error as ConsensusError;
#[cfg(feature="validator")]
use validator::error::Error as ValidatorError;
#[cfg(feature="rpc-server")]
use rpc_server::error::Error as RpcServerError;
#[cfg(feature="metrics-server")]
use metrics_server::error::Error as MetricsServerError;


#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Configuration error: {}", _0)]
    Config(String),

    #[fail(display = "LMDB error: {}", _0)]
    Lmdb(#[cause] LmdbError),

    #[fail(display = "I/O error: {}", _0)]
    Io(#[cause] IoError),

    #[fail(display = "Network error: {}", _0)]
    Network(#[cause] NetworkError),

    #[fail(display = "Key store error: {}", _0)]
    KeyStore(#[cause] KeyStoreError),

    #[fail(display = "Consensus error: {}", _0)]
    Consensus(#[cause] ConsensusError),

    #[fail(display = "Config file parsing error: {}", _0)]
    Toml(#[cause] TomlError),

    #[cfg(feature="validator")]
    #[fail(display = "Validator error: {}", _0)]
    Validator(#[cause] ValidatorError),

    #[cfg(feature="rpc-server")]
    #[fail(display = "RPC server error: {}", _0)]
    RpcServer(#[cause] RpcServerError),

    #[cfg(feature="metrics-server")]
    #[fail(display = "Metrics server error: {}", _0)]
    MetricsServer(#[cause] MetricsServerError),

    #[fail(display = "Logger error: {}", _0)]
    Logging(#[cause] SetLoggerError),
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

impl From<LmdbError> for Error {
    fn from(e: LmdbError) -> Self {
        Self::Lmdb(e)
    }
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Self {
        Self::Io(e)
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

impl From<NetworkError> for Error {
    fn from(e: NetworkError) -> Self {
        Self::Network(e)
    }
}

impl From<KeyStoreError> for Error {
    fn from(e: KeyStoreError) -> Self {
        Self::KeyStore(e)
    }
}

impl From<ConsensusError> for Error {
    fn from(e: ConsensusError) -> Self {
        Self::Consensus(e)
    }
}

impl From<TomlError> for Error {
    fn from(e: TomlError) -> Self {
        Self::Toml(e)
    }
}

#[cfg(feature="validator")]
impl From<ValidatorError> for Error {
    fn from(e: ValidatorError) -> Self {
        Self::Validator(e)
    }
}

#[cfg(feature="rpc-server")]
impl From<RpcServerError> for Error {
    fn from(e: RpcServerError) -> Self {
        Self::RpcServer(e)
    }
}

#[cfg(feature="metrics-server")]
impl From<MetricsServerError> for Error {
    fn from(e: MetricsServerError) -> Self {
        Self::MetricsServer(e)
    }
}

impl From<SetLoggerError> for Error {
    fn from(e: SetLoggerError) -> Self {
        Self::Logging(e)
    }
}
