use std::io::Error as IoError;

use native_tls::Error as NativeTlsError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    IoError(#[from] IoError),
    #[error("{0}")]
    NativeTlsError(#[from] NativeTlsError),
}
