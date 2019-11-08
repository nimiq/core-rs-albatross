use failure::Fail;
use std::io::Error as IoError;

use database::lmdb::LmdbError;
use database::volatile::VolatileDatabaseError;
use network::error::Error as NetworkError;
use utils::key_store::Error as KeyStoreError;
use consensus::Error as ConsensusError;
#[cfg(feature="validator")]
use validator::error::Error as ValidatorError;
use toml::de::Error as TomlError;



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
    #[cfg(feature="validator")]
    #[fail(display = "Validator error: {}", _0)]
    Validator(#[cause] ValidatorError),
    #[fail(display = "Config file parsing error: {}", _0)]
    Toml(#[cause] TomlError),
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

#[cfg(feature="validator")]
impl From<ValidatorError> for Error {
    fn from(e: ValidatorError) -> Self {
        Self::Validator(e)
    }
}

impl From<TomlError> for Error {
    fn from(e: TomlError) -> Self {
        Self::Toml(e)
    }
}
