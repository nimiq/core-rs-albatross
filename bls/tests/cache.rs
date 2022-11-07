use std::cmp;

use nimiq_bls::{cache::PublicKeyCache, lazy::LazyPublicKey, KeyPair};
use nimiq_utils::key_rng::SecureGenerate;
use rand::thread_rng;

#[test]
fn removes_items_on_low_capacity() {
    let mut cache = PublicKeyCache::new(2);

    assert_eq!(cache.len(), 0, "should start empty");

    let rng = &mut thread_rng();

    for i in 0..3 {
        let keypair = KeyPair::generate(rng);
        assert_eq!(
            cache
                .get_or_uncompress(&keypair.public_key.compress())
                .unwrap(),
            keypair.public_key
        );
        assert_eq!(cache.len(), cmp::min(2, i + 1), "should enforce maximum");
    }
}

#[test]
fn can_update_lazy_public_key() {
    let mut cache = PublicKeyCache::new(2);

    assert_eq!(cache.len(), 0, "should start empty");

    let rng = &mut thread_rng();

    let keypair = KeyPair::generate(rng);
    assert_eq!(
        cache
            .get_or_uncompress(&keypair.public_key.compress())
            .unwrap(),
        keypair.public_key
    );

    let lazy_key = LazyPublicKey::from_compressed(&keypair.public_key.compress());

    cache.get_or_uncompress_lazy_public_key(&lazy_key);

    assert!(
        lazy_key.has_uncompressed(),
        "should be uncompressed from cache"
    );
}

#[test]
fn does_not_store_duplicates() {
    let mut cache = PublicKeyCache::new(2);

    assert_eq!(cache.len(), 0, "should start empty");

    let rng = &mut thread_rng();

    let keypair = KeyPair::generate(rng);
    assert_eq!(
        cache
            .get_or_uncompress(&keypair.public_key.compress())
            .unwrap(),
        keypair.public_key
    );

    assert_eq!(
        cache
            .get_or_uncompress(&keypair.public_key.compress())
            .unwrap(),
        keypair.public_key
    );
    assert_eq!(cache.len(), 1, "should not store duplicates");
}
