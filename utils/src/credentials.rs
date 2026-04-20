use nimiq_hash::{Blake2bHash, Hash};
use subtle::ConstantTimeEq;

use crate::Sensitive;

/// HTTP Basic Auth credentials used by the JSON-RPC and metrics servers. The
/// password is stored as a Blake2b hash and checked in constant time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password_hash: Sensitive<Blake2bHash>,
}

impl Credentials {
    pub fn new<U: AsRef<str>, P: AsRef<str>>(username: U, password: P) -> Self {
        Self {
            username: username.as_ref().to_owned(),
            password_hash: Sensitive(password.as_ref().hash()),
        }
    }

    pub fn check<U: AsRef<str>, P: AsRef<str>>(&self, username: U, password: P) -> bool {
        (self.username.as_bytes().ct_eq(username.as_ref().as_bytes())
            & self.password_hash.0.ct_eq(&password.as_ref().hash()))
        .into()
    }
}
