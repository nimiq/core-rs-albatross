use std::path::PathBuf;

use directories::UserDirs;

pub fn home() -> PathBuf {
    UserDirs::new().expect("Failed to determine users home directory").home_dir().join(".nimiq")
}

pub fn system() -> PathBuf {
    PathBuf::from("/var/lib/nimiq")
}
