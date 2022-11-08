use std::collections::HashMap;

use crate::{lazy::LazyPublicKey, CompressedPublicKey, PublicKey};

/// An implementation of a max capacity cache using a hashmap for the public keys.
/// The replacement policy in use removes an arbitrary element.
pub struct PublicKeyCache {
    // FIXME: Change to a map with good caching strategy.
    cache: HashMap<CompressedPublicKey, PublicKey>,
    max_capacity: usize,
}

impl PublicKeyCache {
    /// Creates a new cache with the specified maximum capacity.
    pub fn new(max_capacity: usize) -> Self {
        PublicKeyCache {
            cache: HashMap::with_capacity(max_capacity),
            max_capacity,
        }
    }

    /// Gets the corresponding uncompressed key by retriving it from the cache.
    /// If the value isn't cached, it uncompresses the pk and caches it.
    pub fn get_or_uncompress(&mut self, compressed_key: &CompressedPublicKey) -> Option<PublicKey> {
        // First check if we have the uncompressed key cached.
        if let Some(uncompressed_key) = self.cache.get(compressed_key) {
            Some(*uncompressed_key)
        } else {
            // If not, we try uncompressing it.
            let uncompressed_key = compressed_key.uncompress().ok();
            if let Some(uncompressed_key) = uncompressed_key {
                // Upon success, we store the uncompressed key.
                self.put_if_absent(compressed_key.clone(), uncompressed_key);
            }
            uncompressed_key
        }
    }

    /// Gets the corresponding uncompressed key by retriving it from the lazy key cache and storing it on this cache.
    /// If there is no lazy cached uncompressed pk, it will do the same behavior as in `uncompress`.
    pub fn get_or_uncompress_lazy_public_key(&mut self, compressed_key: &LazyPublicKey) {
        let mut uncompressed_key = compressed_key.cache.write();

        // If the lazy public key is already uncompressed, we store the result in our cache.
        if let Some(key) = uncompressed_key.as_ref() {
            self.put_if_absent(compressed_key.compressed.clone(), *key);
        } else {
            *uncompressed_key = self.get_or_uncompress(&compressed_key.compressed);
        }
    }

    /// Put if absent for the uncompressed key.
    fn put_if_absent(&mut self, compressed_key: CompressedPublicKey, uncompressed_key: PublicKey) {
        // Only add to cache if not present yet.
        if !self.cache.contains_key(&compressed_key) {
            // If the capacity is reached, we delete a key.
            // Currently, it is just the first key in the iterator.
            if self.cache.len() == self.max_capacity {
                let key_to_delete = self.cache.keys().next().unwrap().clone();
                self.cache.remove(&key_to_delete);
            }
            self.cache.insert(compressed_key, uncompressed_key);
        }
    }

    /// Returns the number of elements inside the cache.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns true if cache has no elements inside.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}
