use std::{cmp::Ordering, fmt};

use log::{error, warn};
use nimiq_hash::Hash;
use parking_lot::{
    MappedRwLockReadGuard, RwLock, RwLockReadGuard, RwLockUpgradableReadGuard, RwLockWriteGuard,
};

use crate::{CompressedPublicKey, PublicKey, SigHash, Signature};

/// Spawn blocking if tokio is available.
async fn spawn_blocking<R: Send + 'static, F: FnOnce() -> R + Send + 'static>(f: F) -> R {
    #[cfg(not(target_family = "wasm"))]
    {
        tokio::task::spawn_blocking(f).await.unwrap()
    }

    #[cfg(target_family = "wasm")]
    {
        f()
    }
}

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
        Some(self.cmp(other))
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

    pub fn uncompressed(&self) -> Option<MappedRwLockReadGuard<PublicKey>> {
        let uncompressed: RwLockReadGuard<Option<PublicKey>> = self.cache.read();
        if uncompressed.is_none() {
            error!(compressed = %self.compressed, "Missing uncompressed public key");
            return None;
        }
        Some(RwLockReadGuard::map(uncompressed, |opt| {
            opt.as_ref().unwrap()
        }))
    }

    #[deprecated(note = "Use uncompress(ed) instead")]
    pub fn uncompress_sync(&self) -> Option<MappedRwLockReadGuard<PublicKey>> {
        let read_guard: RwLockReadGuard<Option<PublicKey>>;

        let upgradable = self.cache.upgradable_read();
        if upgradable.is_some() {
            // Fast path, downgrade and return
            read_guard = RwLockUpgradableReadGuard::downgrade(upgradable);
        } else {
            // Slow path, upgrade, write, downgrade and return
            warn!(compressed = %self.compressed, "Uncompressing public key in sync context");
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

    pub async fn uncompress(&self) -> Option<MappedRwLockReadGuard<PublicKey>> {
        let compressed = self.compressed.clone();
        let _uncompressed = spawn_blocking(move || compressed.uncompress()).await;
        None
    }

    pub fn compressed(&self) -> &CompressedPublicKey {
        &self.compressed
    }

    pub fn has_uncompressed(&self) -> bool {
        self.cache.read().is_some()
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

#[cfg(feature = "serde-derive")]
mod serialization {
    use nimiq_serde::SerializedSize;
    use serde::{Deserialize, Serialize};

    use super::*;

    impl SerializedSize for LazyPublicKey {
        const SIZE: usize = CompressedPublicKey::SIZE;
    }

    impl Serialize for LazyPublicKey {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Serialize::serialize(&self.compressed, serializer)
        }
    }

    impl<'de> Deserialize<'de> for LazyPublicKey {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let compressed = CompressedPublicKey::deserialize(deserializer)?;
            Ok(LazyPublicKey::from_compressed(&compressed))
        }
    }
}
