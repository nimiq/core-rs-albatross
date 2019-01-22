use std::fmt::{self, Display};
use ed25519_dalek::SignatureError;

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct KeysError(pub(crate) SignatureError);

impl Display for KeysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
