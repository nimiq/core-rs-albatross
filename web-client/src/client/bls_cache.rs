use std::cell::RefCell;

use idb::{Database, Error, KeyPath, ObjectStore, TransactionMode};
use nimiq_bls::{LazyPublicKey, PublicKey};
use nimiq_serde::{Deserialize, Serialize};

/// Caches decompressed BlsPublicKeys in an IndexedDB
pub(crate) struct BlsCache {
    db: Option<Database>,
    keys: RefCell<Vec<LazyPublicKey>>,
}

#[derive(Deserialize, Serialize)]
struct BlsKeyEntry {
    #[serde(with = "serde_bytes")]
    public_key: [u8; PublicKey::TRUSTED_SERIALIZATION_SIZE],
}

const BLS_KEYS: &str = "bls_keys";
const PUBLIC_KEY: &str = "public_key";

impl BlsCache {
    pub async fn new() -> Self {
        let db = match Database::builder("nimiq_client_cache")
            .version(1)
            .add_object_store(
                ObjectStore::builder(BLS_KEYS).key_path(Some(KeyPath::new_single(PUBLIC_KEY))),
            )
            .build()
            .await
        {
            Ok(db) => Some(db),
            Err(error) => {
                log::warn!(%error, "idb: Couldn't create database");
                None
            }
        };

        BlsCache {
            db,
            keys: RefCell::new(vec![]),
        }
    }

    /// Add the given keys into IndexedDB.
    ///
    /// The given keys must be correctly decompressed already, otherwise this
    /// function will panic.
    pub async fn add_keys(&self, keys: Vec<LazyPublicKey>) -> Result<(), Error> {
        if let Some(db) = &self.db {
            log::info!(num = keys.len(), "storing keys in idb");
            let transaction = db.transaction(&[BLS_KEYS], TransactionMode::ReadWrite)?;
            let bls_keys_store = transaction.object_store(BLS_KEYS)?;

            for key in keys {
                assert!(key.has_uncompressed());
                let entry = BlsKeyEntry {
                    public_key: key
                        .uncompress()
                        .expect("must not pass invalid keys to `BlsCache::add_keys`")
                        .trusted_serialize(),
                };
                let entry_js_value = serde_wasm_bindgen::to_value(&entry).unwrap();
                bls_keys_store.put(&entry_js_value, None)?.await?;
            }
        } else {
            log::error!(num = keys.len(), "can't store keys in idb");
        }
        Ok(())
    }

    /// Fetches all bls keys from the IndexedDB and stores them, which makes
    /// the decompressed keys available in other places.
    pub async fn init(&self) -> Result<(), Error> {
        if let Some(db) = &self.db {
            let transaction = db.transaction(&[BLS_KEYS], TransactionMode::ReadOnly)?;
            let bls_keys_store = transaction.object_store(BLS_KEYS)?;

            let js_keys = bls_keys_store.get_all(None, None)?.await?;
            log::info!(num = js_keys.len(), "loaded keys from idb");

            {
                let mut keys = self.keys.borrow_mut();
                for js_key in &js_keys {
                    let value: BlsKeyEntry = serde_wasm_bindgen::from_value(js_key.clone())
                        .map_err(|e| Error::UnexpectedJsType("BlsKeyEntry", e.into()))?;
                    let public_key = PublicKey::trusted_deserialize(&value.public_key);
                    keys.push(LazyPublicKey::from(public_key));
                }
            }
            transaction.await?;
        } else {
            log::error!("couldn't load keys from idb");
        }
        Ok(())
    }
}
