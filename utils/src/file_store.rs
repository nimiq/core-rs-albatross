use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use thiserror::Error;

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
        log::debug!(path = ?self.path.display(), "Reading from file");
        Ok(T::deserialize_from_vec(&fs::read(&self.path)?)?)
    }

    /// Loads from the file, storing and returning a default value if the file does not exist.
    pub fn load_or_store<T, F>(&self, mut f: F) -> Result<T, Error>
    where
        T: Serialize + Deserialize,
        F: FnMut() -> T,
    {
        match self.load() {
            Err(Error::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
                log::debug!(path = ?self.path.display(), "File does not exist, falling back to default");
                let default = f();
                self.store(&default)?;
                Ok(default)
            }
            Ok(result) => Ok(result),
            Err(err) => Err(err),
        }
    }

    pub fn store<T: Serialize>(&self, item: &T) -> Result<(), Error> {
        log::debug!(path = ?self.path.display(), "Writing to file");

        let file = create_file_creating_parent_if_not_exists(&self.path)?;
        let mut buf_writer = BufWriter::new(file);
        Serialize::serialize(item, &mut buf_writer)?;
        buf_writer.flush()?;
        Ok(())
    }
}

fn create_file_creating_parent_if_not_exists(path: &Path) -> io::Result<File> {
    match File::create(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
                return File::create(path);
            }
            Err(err)
        }
        result => result,
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Serialization error: {0}")]
    Serialization(#[from] DeserializeError),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}
