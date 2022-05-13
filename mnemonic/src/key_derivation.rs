use nimiq_hash::pbkdf2::Pbkdf2Error;
use nimiq_key_derivation::ExtendedPrivateKey;

use crate::Mnemonic;

pub trait ToExtendedPrivateKey {
    /// Returns the corresponding master extended private key for this mnemonic.
    fn to_master_key(&self, password: Option<&str>) -> Result<ExtendedPrivateKey, Pbkdf2Error>;
}

impl ToExtendedPrivateKey for Mnemonic {
    /// Returns the corresponding master extended private key for this mnemonic.
    fn to_master_key(&self, password: Option<&str>) -> Result<ExtendedPrivateKey, Pbkdf2Error> {
        Ok(ExtendedPrivateKey::from_seed(self.to_seed(password)?))
    }
}

pub trait FromMnemonic: Sized {
    /// Converts a mnemonic into the corresponding master extended private key.
    fn from_mnemonic(mnemonic: &Mnemonic, password: Option<&str>) -> Result<Self, Pbkdf2Error>;
}

impl FromMnemonic for ExtendedPrivateKey {
    /// Converts a mnemonic into the corresponding master extended private key.
    fn from_mnemonic(mnemonic: &Mnemonic, password: Option<&str>) -> Result<Self, Pbkdf2Error> {
        mnemonic.to_master_key(password)
    }
}
