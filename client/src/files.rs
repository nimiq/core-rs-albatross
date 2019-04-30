use std::path::PathBuf;

use failure::Fail;

use primitives::networks::NetworkId;
use std::fs::create_dir_all;
#[cfg(not(feature = "system-install"))]
use directories::UserDirs;


#[derive(Debug, Fail)]
pub enum Error {
    #[cfg(not(feature = "system-install"))]
    #[fail(display = "Can't find home directory")]
    NoHome,
    #[fail(display = "Can't create file or directory: {}", _1)]
    CantCreate(#[cause] std::io::Error, String),
}

/// Default paths for files
#[derive(Debug)]
pub struct FileLocations {
    config: PathBuf,
    peer_key: PathBuf,
    database_parent: PathBuf,
}

impl FileLocations {
    const CONFIG_FILE: &'static str = "client.toml";
    const PEER_KEY_FILE: &'static str = "peer_key.dat";

    const EXAMPLE_CONFIG: &'static str = include_str!("../client.example.toml");

    fn create_dir(path: &PathBuf) -> Result<bool, Error> {
        if !path.exists() {
            // create directory
            create_dir_all(path).map_err(|e| {
                Error::CantCreate(e, path.display().to_string())
            })?;
            Ok(true)
        }
        else {
            Ok(false)
        }
    }

    fn create_config_dir(path: &PathBuf) -> Result<bool, Error> {
        if Self::create_dir(&path)? {
            // populate with example config
            let config_path = path.join("client.example.toml");
            std::fs::write(&config_path, Self::EXAMPLE_CONFIG)
                .map_err(|e| {
                    Error::CantCreate(e, config_path.display().to_string())
                })?;
            Ok(true)
        }
        else {
            Ok(false)
        }
    }

    /// Create file locations for `~/.nimiq`
    #[cfg(not(feature = "system-install"))]
    pub fn home() -> Result<Self, Error> {
        let nimiq_home = UserDirs::new()
            .ok_or(Error::NoHome)?
            .home_dir()
            .join(".nimiq");
        Self::create_config_dir(&nimiq_home)?;

        Ok(FileLocations {
            config: nimiq_home.join(Self::CONFIG_FILE),
            peer_key: nimiq_home.join(Self::PEER_KEY_FILE),
            database_parent: nimiq_home
        })
    }

    /// Create file locations for system-wide installation, i.e. `/etc/nimiq`, `/var/local/nimiq`
    #[cfg(feature = "system-install")]
    pub fn system() -> Result<Self, Error> {
        let etc_nimiq = PathBuf::from("/etc/nimiq");
        Self::create_config_dir(&etc_nimiq)?;
        let var_nimiq = PathBuf::from("/var/lib/nimiq");
        Self::create_dir(&var_nimiq)?;

        Ok(FileLocations {
            config: etc_nimiq.join(Self::CONFIG_FILE),
            peer_key: var_nimiq.join(Self::PEER_KEY_FILE),
            database_parent: var_nimiq
        })
    }

    /// Return default path for config file
    pub fn config(&self) -> PathBuf {
        self.config.clone()
    }

    /// Return default path for peer key file
    pub fn peer_key(&self) -> PathBuf {
        self.peer_key.clone()
    }

    /// Return default path for database, depending on network ID
    pub fn database(&self, network: NetworkId) -> PathBuf {
        self.database_parent.join(format!("db.{}", network))
    }

    pub fn default() -> Result<Self, Error> {
        #[cfg(feature = "system-install")] {
            Self::system()
        }
        #[cfg(not(feature = "system-install"))] {
            Self::home()
        }
    }
}


pub struct LazyFileLocations(Option<FileLocations>);
impl LazyFileLocations {
    pub fn new() -> Self {
        Self(None)
    }

    fn lazy_load(&mut self) -> Result<&FileLocations, Error> {
        if let None = self.0 {
            let files = FileLocations::default()?;
            self.0 = Some(files);
        }
        Ok(self.0.as_ref().unwrap())
    }

    pub fn config(&mut self) -> Result<PathBuf, Error> {
        Ok(self.lazy_load()?.config())
    }

    pub fn peer_key(&mut self) -> Result<PathBuf, Error> {
        Ok(self.lazy_load()?.peer_key())
    }

    pub fn database(&mut self, network: NetworkId) -> Result<PathBuf, Error> {
        Ok(self.lazy_load()?.database(network))
    }
}
