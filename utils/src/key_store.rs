use std::fs;
use std::io::Error as IoError;

use failure::Fail;

use beserial::{Deserialize, Serialize};

pub struct KeyStore {
    path: String,
}

impl KeyStore {
    pub fn new(path: String) -> Self {
        KeyStore {
            path
        }
    }

    pub fn load_key<T: Serialize + Deserialize>(&self) -> Result<T, Error> {
        match fs::read(&self.path) {
            Ok(data) => {
                Deserialize::deserialize_from_vec(&data).map_err(|_| Error::InvalidKey)
            },
            Err(e) => Err(Error::IoError(e)),
        }
    }

    pub fn save_key<T: Serialize + Deserialize>(&self, key_pair: &T) -> Result<(), Error> {
        Ok(fs::write(&self.path, key_pair.serialize_to_vec())?)
    }
}

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "Key could not be deserialized")]
    InvalidKey,
    #[fail(display = "{}", _0)]
    IoError(#[cause] IoError),
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Self {
        Error::IoError(e)
    }
}
