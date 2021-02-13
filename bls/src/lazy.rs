use std::{cmp::Ordering, fmt};

use parking_lot::{
    MappedRwLockReadGuard, RwLock, RwLockReadGuard, RwLockUpgradableReadGuard, RwLockWriteGuard,
};

use nimiq_hash::Hash;

use crate::{CompressedPublicKey, PublicKey, SigHash, Signature};

pub struct LazyPublicKey {
    pub(crate) compressed: CompressedPublicKey,
    pub(crate) cache: RwLock<Option<PublicKey>>,
}

impl fmt::Debug for LazyPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "LazyPublicKey({})", self.compressed)
    }
}

impl fmt::Display for LazyPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.compressed, f)
    }
}

impl Clone for LazyPublicKey {
    fn clone(&self) -> Self {
        LazyPublicKey {
            compressed: self.compressed.clone(),
            cache: RwLock::new(*self.cache.read()),
        }
    }
}

impl PartialEq for LazyPublicKey {
    fn eq(&self, other: &LazyPublicKey) -> bool {
        self.compressed.eq(&other.compressed)
    }
}

impl Eq for LazyPublicKey {}

impl PartialOrd<LazyPublicKey> for LazyPublicKey {
    fn partial_cmp(&self, other: &LazyPublicKey) -> Option<Ordering> {
        self.compressed.partial_cmp(&other.compressed)
    }
}

impl Ord for LazyPublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compressed.cmp(&other.compressed)
    }
}

impl AsRef<[u8]> for LazyPublicKey {
    fn as_ref(&self) -> &[u8] {
        self.compressed.as_ref()
    }
}

impl LazyPublicKey {
    pub fn from_compressed(compressed: &CompressedPublicKey) -> Self {
        LazyPublicKey {
            compressed: compressed.clone(),
            cache: RwLock::new(None),
        }
    }

    pub fn uncompress(&self) -> Option<MappedRwLockReadGuard<PublicKey>> {
        let read_guard: RwLockReadGuard<Option<PublicKey>>;

        let upgradable = self.cache.upgradable_read();
        if upgradable.is_some() {
            // Fast path, downgrade and return
            read_guard = RwLockUpgradableReadGuard::downgrade(upgradable);
        } else {
            // Slow path, upgrade, write, downgrade and return
            let mut upgraded = RwLockUpgradableReadGuard::upgrade(upgradable);
            *upgraded = Some(match self.compressed.uncompress() {
                Ok(p) => p,
                _ => return None,
            });
            read_guard = RwLockWriteGuard::downgrade(upgraded);
        }

        Some(RwLockReadGuard::map(read_guard, |opt| {
            opt.as_ref().unwrap()
        }))
    }

    pub fn uncompress_unchecked(&self) -> MappedRwLockReadGuard<PublicKey> {
        self.uncompress().expect("Invalid public key")
    }

    pub fn compressed(&self) -> &CompressedPublicKey {
        &self.compressed
    }

    pub fn verify<M: Hash>(&self, msg: &M, signature: &Signature) -> bool {
        let cached = self.uncompress();
        if let Some(public_key) = cached.as_ref() {
            return public_key.verify(msg, signature);
        }
        false
    }

    pub fn verify_hash(&self, hash: SigHash, signature: &Signature) -> bool {
        let cached = self.uncompress();
        if let Some(public_key) = cached.as_ref() {
            return public_key.verify_hash(hash, signature);
        }
        false
    }
}

impl From<PublicKey> for LazyPublicKey {
    fn from(key: PublicKey) -> Self {
        LazyPublicKey {
            compressed: key.compress(),
            cache: RwLock::new(Some(key)),
        }
    }
}

impl From<CompressedPublicKey> for LazyPublicKey {
    fn from(key: CompressedPublicKey) -> Self {
        LazyPublicKey::from_compressed(&key)
    }
}

impl From<LazyPublicKey> for CompressedPublicKey {
    fn from(key: LazyPublicKey) -> Self {
        key.compressed
    }
}

#[cfg(feature = "beserial")]
mod serialization {
    use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

    use super::*;

    impl Serialize for LazyPublicKey {
        fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
            self.compressed.serialize(writer)
        }

        fn serialized_size(&self) -> usize {
            self.compressed.serialized_size()
        }
    }

    impl Deserialize for LazyPublicKey {
        fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
            Ok(LazyPublicKey::from_compressed(&Deserialize::deserialize(
                reader,
            )?))
        }
    }
}
