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

    pub fn load<T: Deserialize>(&self) -> Result<T, Error> {
        debug!("Reading from: {}", self.path.display());
        let file = OpenOptions::new().read(true).open(&self.path)?;
        let mut buf_reader = BufReader::new(file);
        let item = Deserialize::deserialize(&mut buf_reader)?;
        Ok(item)
    }

    pub fn load_or_store<T, F>(&self, mut f: F) -> Result<T, Error>
    where
        T: Serialize + Deserialize,
        F: FnMut() -> T,
    {
        if self.path.exists() {
            self.load()
        } else {
            let x = f();
            self.store(&x)?;
            Ok(x)
        }
    }

    pub fn store<T: Serialize>(&self, item: &T) -> Result<(), Error> {
        log::debug!("Writing to: {}", self.path.display());
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.path)?;
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
