use std::{
    fs::OpenOptions,
    io::{BufReader, BufWriter, Error as IoError, Write},
    path::{Path, PathBuf},
};

use thiserror::Error;

use beserial::{Deserialize, Serialize, SerializingError};

pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        FileStore {
            path: path.as_ref().to_owned(),
        }
    }

    pub fn load_key<T: Deserialize>(&self) -> Result<T, Error> {
        let file = OpenOptions::new().read(true).open(&self.path)?;
        let mut buf_reader = BufReader::new(file);
        let item = Deserialize::deserialize(&mut buf_reader)?;
        Ok(item)
    }

    pub fn save_key<T: Serialize>(&self, item: &T) -> Result<(), Error> {
        let file = OpenOptions::new().write(true).create(true).open(&self.path)?;
        let mut buf_writer = BufWriter::new(file);
        Serialize::serialize(item, &mut buf_writer)?;
        buf_writer.flush()?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Serialization error: {0}")]
    Serialization(#[from] SerializingError),

    #[error("IO error: {0}")]
    IoError(#[from] IoError),
}
