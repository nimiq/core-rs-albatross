use std::fmt;

use parking_lot::{Mutex, MutexGuard, MappedMutexGuard};

use super::*;


pub struct LazyPublicKey {
    pub(crate) compressed: CompressedPublicKey,
    pub(crate) cache: Mutex<Option<PublicKey>>,
}

impl fmt::Debug for LazyPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "LazyPublicKey({})", &hex::encode(self.compressed.p_pub.as_ref()))
    }
}

impl Clone for LazyPublicKey {
    fn clone(&self) -> Self {
        LazyPublicKey {
            compressed: self.compressed.clone(),
            cache: Mutex::new(self.cache.lock().clone()),
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
            cache: Mutex::new(None),
        }
    }

    pub fn uncompress(&self) -> Option<MappedMutexGuard<PublicKey>> {
        let mut cached = self.cache.lock();
        if cached.is_none() {
            *cached = Some(match self.compressed.uncompress() {
                Ok(p) => p,
                _ => return None,
            });
        }

        Some(MutexGuard::map(cached, |opt| opt.as_mut().unwrap()))
    }

    pub fn uncompress_unchecked(&self) -> MappedMutexGuard<PublicKey> {
        self.uncompress().expect("Invalid public key")
    }

    pub fn compressed(&self) -> &CompressedPublicKey {
        &self.compressed
    }

    pub fn uncompressed(&self) -> Option<PublicKey> {
        // NOTE: We can't use Option::cloned here, as clippy suggests, since MappedMutexGuard
        //       doesn't really implement Clone. But `guard.clone` calls the underlying clone method
        #[allow(clippy::map_clone)]
        self.uncompress().map(|guard| guard.clone())
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
            cache: Mutex::new(Some(key)),
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
            Ok(LazyPublicKey::from_compressed(&Deserialize::deserialize(reader)?))
        }
    }
}
