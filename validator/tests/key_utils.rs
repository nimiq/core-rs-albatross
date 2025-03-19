use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_utils::key_rng::SecureGenerate;
use nimiq_validator::key_utils::VotingKeys;
use rand::{rngs::StdRng, SeedableRng};

#[test]
fn test_voting_keys() {
    let mut rng = StdRng::from_rng(&mut rand::rng());

    // Initialize the VotingKeys
    let key1 = BlsKeyPair::generate(&mut rng);
    let key2 = BlsKeyPair::generate(&mut rng);
    let mut bls_keys = Vec::new();
    bls_keys.push(key1.clone());
    bls_keys.push(key2.clone());

    let mut voting_keys = VotingKeys::new(bls_keys.clone());
    assert_eq!(voting_keys.get_keys().len(), 2);
    for bls_key in &bls_keys {
        assert!(voting_keys.get_keys().contains(bls_key));
    }

    // Updating to a non-available key produces an error
    let additional_key = BlsKeyPair::generate(&mut rng);
    assert_eq!(
        voting_keys.update_current_key(&additional_key.public_key.compress()),
        Err(())
    );

    // Add a key
    voting_keys.add_key(additional_key.clone());
    bls_keys.push(additional_key.clone());
    assert_eq!(voting_keys.get_keys().len(), 3);
    for bls_key in &bls_keys {
        assert!(voting_keys.get_keys().contains(bls_key));
    }

    // Verify that the current key is correctly set/returned
    assert_eq!(
        voting_keys.update_current_key(&additional_key.public_key.compress()),
        Ok(())
    );
    assert_eq!(voting_keys.get_current_key(), additional_key);
    assert_eq!(
        voting_keys.update_current_key(&key1.public_key.compress()),
        Ok(())
    );
    assert_eq!(voting_keys.get_current_key(), key1);
}
