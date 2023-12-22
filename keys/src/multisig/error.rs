use std::fmt;

use thiserror::Error;

/// Scalars of 0 or 1 are not allowed for nonces and this error is returned.
#[derive(Debug, Clone, Copy)]
pub struct InvalidScalarError;

impl fmt::Display for InvalidScalarError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Generated scalar was invalid (0 or 1).")
    }
}

impl std::error::Error for InvalidScalarError {
    fn description(&self) -> &str {
        "Generated scalar was invalid (0 or 1)."
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        None
    }
}

/// Errors that can occur during partial signature generation.
#[derive(Debug, Error)]
pub enum PartialSignatureError {
    #[error("Missing nonces")]
    MissingNonces,
}
