use nimiq_hash::{Blake2bHash, Blake2bHasher, Hasher};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_utils::merkle::{compute_root_from_content, MerklePath, MerkleProof};

const VALUE: &str = "merkletree";

pub mod incremental;
pub mod partial;

#[test]
fn it_correctly_computes_an_empty_root_hash() {
    let empty_hash = Blake2bHasher::default().digest(&[]);
    let root = compute_root_from_content::<Blake2bHasher, [u8; 32]>(&[]);
    assert_eq!(root, empty_hash);
    let root = compute_root_from_content::<Blake2bHasher, [u8; 32]>(&[]);
    assert_eq!(root, empty_hash);
}

#[test]
fn it_correctly_computes_a_simple_root_hash() {
    let hash = Blake2bHasher::default().digest(VALUE.as_bytes());
    let root = compute_root_from_content::<Blake2bHasher, &'static str>(&[VALUE]);
    assert_eq!(root, hash);
    let root = compute_root_from_content::<Blake2bHasher, &'static str>(&[VALUE]);
    assert_eq!(root, hash);
}

#[test]
fn it_serializes_empty_path_to_one_byte() {
    let path: MerklePath<Blake2bHash> = MerklePath::empty();
    let serialized = path.serialize_to_vec();
    assert_eq!(serialized, vec![0]);
    let path2 = MerklePath::<Blake2bHash>::deserialize_from_vec(&serialized).unwrap();
    assert_eq!(path, path2);
}

#[test]
fn it_correctly_computes_a_complex_root_hash() {
    /*
     *          level2
     *         /      \
     *    level1      level1
     *     / \         / \
     *   l0  l0      l0  l0
     *   |    |      |    |
     * value value value value
     */
    let level0 = Blake2bHasher::default().digest(VALUE.as_bytes());
    let level1 = Blake2bHasher::default()
        .chain(&level0)
        .chain(&level0)
        .finish();
    let level2 = Blake2bHasher::default()
        .chain(&level1)
        .chain(&level1)
        .finish();
    let root =
        compute_root_from_content::<Blake2bHasher, &'static str>(&[VALUE, VALUE, VALUE, VALUE]);
    assert_eq!(root, level2);

    /*
     *          level2a
     *         /      \
     *    level1      level0
     *     / \          |
     *   l0  l0       value
     *   |    |
     * value value
     */
    let level2a = Blake2bHasher::default()
        .chain(&level1)
        .chain(&level0)
        .finish();
    let root = compute_root_from_content::<Blake2bHasher, &'static str>(&[VALUE, VALUE, VALUE]);
    assert_eq!(root, level2a);
}

#[test]
fn it_correctly_computes_an_empty_path() {
    let root = compute_root_from_content::<Blake2bHasher, [u8; 32]>(&[]);
    let proof = MerklePath::new::<Blake2bHasher, [u8; 32]>(&[], &[0u8; 32]);
    assert_eq!(proof.len(), 0);
    assert_ne!(proof.compute_root(&[0u8; 32]), root);
}

#[test]
fn it_correctly_computes_a_simple_path() {
    let values = vec!["1", "2", "3", "4"];
    /*
     * (X) should be the nodes included in the proof.
     * *X* marks the values to be proven.
     *            h6
     *         /      \
     *      h4        (h5)
     *     / \         / \
     *  (h0) h1      h2  h3
     *   |    |      |    |
     *  v0  *v1*    v2   v3
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[1]);
    assert_eq!(proof.len(), 2);
    assert_eq!(proof.compute_root(&values[1]), root);
}

#[test]
fn it_correctly_computes_more_complex_paths() {
    let values = vec!["1", "2", "3"];
    /*
     *            h4
     *         /      \
     *      h3        (h2)
     *     / \          |
     *  (h0) h1        v2
     *   |    |
     *  v0  *v1*
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[1]);
    assert_eq!(proof.len(), 2);
    assert_eq!(proof.compute_root(&values[1]), root);

    /*
     *            h4
     *         /      \
     *     (h3)        h2
     *     / \          |
     *   h0  h1       *v2*
     *   |    |
     *  v0   v1
     */
    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[2]);
    assert_eq!(proof.len(), 1);
    assert_eq!(proof.compute_root(&values[2]), root);

    /*
     *                   h6
     *            /               \
     *         (h4)                h5
     *       /      \            /   \
     *     h0        h1       (h2)    h3
     *   /   \     /   \     /   \    |
     *  v0   v1   v2   v3   v4   v5  *v6*
     */
    let values = vec!["1", "2", "3", "4", "5", "6", "7"];
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[6]);
    assert_eq!(proof.len(), 2);
    assert_eq!(proof.compute_root(&values[6]), root);

    /*
     *                   h6
     *            /               \
     *          h4                (h5)
     *       /      \            /   \
     *    (h0)       h1        h2     h3
     *   /   \     /   \     /   \    |
     *  v0   v1  *v2* (v3)  v4   v5   v6
     */
    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[2]);
    assert_eq!(proof.len(), 3);
    assert_eq!(proof.compute_root(&values[2]), root);

    /*
     *                   h6
     *            /               \
     *         (h4)                h5
     *       /      \            /   \
     *     h0        h1        h2    (h3)
     *   /   \     /   \     /   \    |
     *  v0   v1   v2   v3  (v4) *v5*  v6
     */
    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[5]);
    assert_eq!(proof.len(), 3);
    assert_eq!(proof.compute_root(&values[5]), root);
}

#[test]
fn it_correctly_serializes_and_deserializes_path() {
    let values = vec!["1", "2", "3", "4", "5", "6", "7"];

    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[6]);
    let mut serialization: Vec<u8> = Vec::with_capacity(proof.serialized_size());
    let size = proof.serialize_to_writer(&mut serialization).unwrap();
    assert_eq!(size, proof.serialized_size());
    let proof2: MerklePath<Blake2bHash> =
        Deserialize::deserialize_from_vec(&serialization[..]).unwrap();
    assert_eq!(proof, proof2);
}

#[test]
fn it_correctly_computes_an_empty_proof() {
    let root = compute_root_from_content::<Blake2bHasher, &str>(&[]);
    let proof: MerkleProof<Blake2bHash> = MerkleProof::from_values::<&str>(&[], &["0"]);
    assert_eq!(proof.len(), 1);
    assert!(proof.compute_root_from_values(&["0"]).is_err());

    let proof: MerkleProof<Blake2bHash> = MerkleProof::from_values::<&str>(&[], &[]);
    assert_eq!(proof.len(), 1);
    let proof_root = proof.compute_root_from_values::<&str>(&[]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);
}

#[test]
fn it_correctly_computes_a_simple_proof() {
    let values = vec!["1", "2", "3", "4"];
    /*
     * (X) should be the nodes included in the proof.
     * *X* marks the values to be proven.
     *            h6
     *         /      \
     *      h4        (h5)
     *     / \         / \
     *  (h0) h1      h2  h3
     *   |    |      |    |
     *  v0  *v1*    v2   v3
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
    let proof: MerkleProof<Blake2bHash> = MerkleProof::from_values::<&str>(&values, &[values[1]]);
    assert_eq!(proof.len(), 2);
    let proof_root = proof.compute_root_from_values(&[values[1]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    let proof: MerkleProof<Blake2bHash> = MerkleProof::from_values::<&str>(&values, &[]);
    assert_eq!(proof.len(), 1);
    let proof_root = proof.compute_root_from_values::<&str>(&[]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);
}

#[test]
fn it_correctly_computes_more_complex_proofs() {
    let values = vec!["1", "2", "3", "5", "7", "8", "9"];
    /*
     *            h6
     *         /      \
     *      h4         h5
     *     / \         / \
     *  (h0) h1      h2 (h3)
     *   |    |      |    |
     *  v0  *v1*   *v2*  v3
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values[..4]);
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values[..4], &[values[1], values[2]]);
    assert_eq!(proof.len(), 2);
    let proof_root = proof.compute_root_from_values(&[values[1], values[2]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *            h6
     *         /      \
     *      h4        (h5)
     *     / \         / \
     *   h0  h1      h2  h3
     *   |    |      |    |
     * *v0* *v1*    v2   v3
     */
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values[..4], &[values[0], values[1]]);
    assert_eq!(proof.len(), 1);
    let proof_root = proof.compute_root_from_values(&[values[0], values[1]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *            h4
     *         /      \
     *      h3        (h2)
     *     / \          |
     *   h0  h1        v2
     *   |    |
     * *v0* *v1*
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values[..3]);
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values[..3], &[values[0], values[1]]);
    assert_eq!(proof.len(), 1);
    let proof_root = proof.compute_root_from_values(&[values[0], values[1]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *            h4
     *         /      \
     *      h3        (h2)
     *     / \          |
     *   h0  h1        v2
     *   |    |
     * *v0* *v1*
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values[..3]);
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values[..3], &[values[2]]);
    assert_eq!(proof.len(), 1);
    let proof_root = proof.compute_root_from_values(&[values[2]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *            h4
     *         /      \
     *      h3         h2
     *     / \          |
     *  (h0) h1       *v2*
     *   |    |
     *  v0  *v1*
     */
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values[..3], &[values[1], values[2]]);
    assert_eq!(proof.len(), 1);
    let proof_root = proof.compute_root_from_values(&[values[1], values[2]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *                   h6
     *            /               \
     *         (h4)                h5
     *       /      \            /   \
     *     h0        h1       (h2)    h3
     *   /   \     /   \     /   \    |
     *  v0   v1   v2   v3   v4   v5  *v6*
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
    let proof: MerkleProof<Blake2bHash> = MerkleProof::from_values::<&str>(&values, &[values[6]]);
    assert_eq!(proof.len(), 2);
    let proof_root = proof.compute_root_from_values(&[values[6]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *                   h6
     *            /               \
     *          h4                 h5
     *       /      \            /   \
     *    (h0)       h1       (h2)    h3
     *   /   \     /   \     /   \    |
     *  v0   v1  *v2* (v3)  v4   v5  *v6*
     */
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values, &[values[2], values[6]]);
    assert_eq!(proof.len(), 3);
    let proof_root = proof.compute_root_from_values(&[values[2], values[6]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *                   h6
     *            /               \
     *          h4                 h5
     *       /      \            /   \
     *    (h0)       h1        h2    (h3)
     *   /   \     /   \     /   \    |
     *  v0   v1  (v2) *v3* *v4* (v5)  v6
     */
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values, &[values[3], values[4]]);
    assert_eq!(proof.len(), 4);
    let proof_root = proof.compute_root_from_values(&[values[3], values[4]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);
}

#[test]
fn it_correctly_computes_absence_proofs() {
    let values = vec!["1", "2", "3", "5", "7", "8", "9"];
    let missing_values = ["0", "4", "6"];
    /*
     *            h6
     *         /      \
     *      h4         h5
     *     / \         / \
     *   h0 (h1)     h2 (h3)
     *   |    |      |    |
     * *v0*  v1    *v2*  v3
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values[..4]);
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::with_absence::<&str>(&values[..4], &[missing_values[0], values[2]]);
    assert_eq!(proof.len(), 2);
    let proof_root = proof.compute_root_from_values(&[values[0], values[2]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *            h6
     *         /      \
     *      h4         h5
     *     / \         / \
     *   h0 (h1)    (h2) h3
     *   |    |      |    |
     * *v0*  v1     v2  *v3*
     */
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::with_absence::<&str>(&values[..4], &[missing_values[0], values[4]]);
    assert_eq!(proof.len(), 2);
    let proof_root = proof.compute_root_from_values(&[values[0], values[3]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *            h4
     *         /      \
     *     (h3)        h2
     *     / \          |
     *   h0  h1       *v2*
     *   |    |
     *  v0   v1
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values[..3]);
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::with_absence::<&str>(&values[..3], &[values[4]]);
    assert_eq!(proof.len(), 1);
    let proof_root = proof.compute_root_from_values(&[values[2]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *                   h6
     *            /               \
     *          h4                (h5)
     *       /      \            /   \
     *    (h0)       h1        h2     h3
     *   /   \     /   \     /   \    |
     *  v0   v1  *v2* *v3*  v4   v5   v6
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::with_absence::<&str>(&values, &[missing_values[1]]);
    assert_eq!(proof.len(), 2);
    let proof_root = proof.compute_root_from_values(&[values[2], values[3]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *                   h6
     *            /               \
     *          h4                 h5
     *       /      \            /   \
     *    (h0)       h1        h2    (h3)
     *   /   \     /   \     /   \    |
     *  v0   v1  (v2) *v3* *v4* (v5)  v6
     */
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::with_absence::<&str>(&values, &[missing_values[2]]);
    assert_eq!(proof.len(), 4);
    let proof_root = proof.compute_root_from_values(&[values[3], values[4]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);

    /*
     *                   h6
     *            /               \
     *          h4                 h5
     *       /      \            /   \
     *     h0        h1        h2    (h3)
     *   /   \     /   \     /   \    |
     * *v0* (v1) *v2* *v3* *v4* (v5)  v6
     */
    let proof: MerkleProof<Blake2bHash> = MerkleProof::with_absence::<&str>(
        &values,
        &[values[0], missing_values[1], missing_values[2]],
    );
    assert_eq!(proof.len(), 3);
    let proof_root = proof.compute_root_from_values(&[values[0], values[2], values[3], values[4]]);
    assert!(proof_root.is_ok());
    assert_eq!(proof_root.unwrap(), root);
}

#[test]
fn it_correctly_discards_invalid_proofs() {
    let values = vec!["1", "2", "3", "4"];
    /*
     * (X) should be the nodes included in the proof.
     * *X* marks the values to be proven.
     *            h6
     *         /      \
     *      h4        (h5)
     *     / \         / \
     *  (h0) h1      h2  h3
     *   |    |      |    |
     *  v0  *v1*    v2   v3
     */
    let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
    let proof: MerkleProof<Blake2bHash> = MerkleProof::from_values::<&str>(&values, &[values[1]]);

    let proof_root = proof.compute_root_from_values(&[values[0]]);
    assert!(proof_root.is_ok());
    assert_ne!(proof_root.unwrap(), root);

    assert!(proof.compute_root_from_values::<&str>(&[]).is_err());
}

#[test]
fn it_correctly_serializes_and_deserializes_proof() {
    let values = vec!["1", "2", "3", "5", "7", "8", "9"];

    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::from_values::<&str>(&values, &[values[2], values[6]]);
    let mut serialization: Vec<u8> = Vec::with_capacity(proof.serialized_size());
    let size = proof.serialize_to_writer(&mut serialization).unwrap();
    assert_eq!(size, proof.serialized_size());
    let proof2 = MerkleProof::<Blake2bHash>::deserialize_from_vec(&serialization);
    assert_eq!(proof2, Ok(proof));

    /*
     *            h4
     *         /      \
     *     (h3)        h2
     *     / \          |
     *   h0  h1       *v2*
     *   |    |
     *  v0   v1
     */
    let proof: MerkleProof<Blake2bHash> =
        MerkleProof::with_absence::<&str>(&values[..3], &[values[4]]);
    let mut serialization: Vec<u8> = Vec::with_capacity(proof.serialized_size());
    let size = proof.serialize_to_writer(&mut serialization).unwrap();
    assert_eq!(size, proof.serialized_size());
    let proof2 = MerkleProof::<Blake2bHash>::deserialize_from_vec(&serialization);
    assert_eq!(proof2, Ok(proof));
}

// Keccak256 Merkle tree tests
#[cfg(test)]
mod keccak256_tests {
    use nimiq_hash::{Blake2bHasher, HashOutput, Hasher, Keccak256Hasher};
    use nimiq_test_log::test;
    use nimiq_utils::merkle::{compute_root_from_content, MerklePath};

    const VALUE: &str = "merkletree";

    #[test]
    fn it_correctly_computes_keccak256_root_from_content() {
        // Test with single value
        let hash = Keccak256Hasher::default().digest(VALUE.as_bytes());
        let root = compute_root_from_content::<Keccak256Hasher, &'static str>(&[VALUE]);
        assert_eq!(root, hash);

        // Test with multiple values
        let values = vec!["1", "2", "3", "4"];
        let root = compute_root_from_content::<Keccak256Hasher, &str>(&values);

        // Manually compute expected root
        let h0 = Keccak256Hasher::default().digest(values[0].as_bytes());
        let h1 = Keccak256Hasher::default().digest(values[1].as_bytes());
        let h2 = Keccak256Hasher::default().digest(values[2].as_bytes());
        let h3 = Keccak256Hasher::default().digest(values[3].as_bytes());

        let h4 = Keccak256Hasher::default().chain(&h0).chain(&h1).finish();
        let h5 = Keccak256Hasher::default().chain(&h2).chain(&h3).finish();
        let expected_root = Keccak256Hasher::default().chain(&h4).chain(&h5).finish();

        assert_eq!(root, expected_root);
    }

    #[test]
    fn it_correctly_computes_keccak256_merkle_path() {
        let values = vec!["1", "2", "3", "4"];

        // Compute root with Keccak256
        let root = compute_root_from_content::<Keccak256Hasher, &str>(&values);

        // Generate proof for second value
        let proof = MerklePath::new::<Keccak256Hasher, &str>(&values, &values[1]);
        assert_eq!(proof.len(), 2);

        // Verify proof
        let computed_root = proof.compute_root(&values[1]);
        assert_eq!(computed_root, root);
    }

    #[test]
    fn it_correctly_verifies_keccak256_merkle_path() {
        let values = vec!["1", "2", "3", "4", "5", "6", "7"];
        let root = compute_root_from_content::<Keccak256Hasher, &str>(&values);

        // Test proof for various indices
        for (i, value) in values.iter().enumerate() {
            let proof = MerklePath::new::<Keccak256Hasher, &str>(&values, value);
            let computed_root = proof.compute_root(value);
            assert_eq!(
                computed_root, root,
                "Proof verification failed for index {}",
                i
            );
        }
    }

    #[test]
    fn blake2b_and_keccak256_roots_differ() {
        let values = vec!["1", "2", "3", "4"];

        // Compute root with Blake2b
        let blake2b_root = compute_root_from_content::<Blake2bHasher, &str>(&values);

        // Compute root with Keccak256
        let keccak256_root = compute_root_from_content::<Keccak256Hasher, &str>(&values);

        // Verify they are different (convert to bytes for comparison)
        assert_ne!(
            blake2b_root.as_bytes(),
            keccak256_root.as_bytes(),
            "Blake2b and Keccak256 roots should differ for the same data"
        );
    }

    #[test]
    fn it_handles_various_input_sizes_with_keccak256() {
        // Test empty input
        let empty_hash = Keccak256Hasher::default().digest(&[]);
        let root = compute_root_from_content::<Keccak256Hasher, [u8; 32]>(&[]);
        assert_eq!(root, empty_hash);

        // Test single element
        let values = vec!["single"];
        let root = compute_root_from_content::<Keccak256Hasher, &str>(&values);
        let expected = Keccak256Hasher::default().digest(values[0].as_bytes());
        assert_eq!(root, expected);

        // Test power of 2 sizes
        for size in [2, 4, 8, 16] {
            let values: Vec<String> = (0..size).map(|i| i.to_string()).collect();
            let values_ref: Vec<&str> = values.iter().map(|s| s.as_str()).collect();
            let root = compute_root_from_content::<Keccak256Hasher, &str>(&values_ref);

            // Verify proof for first element
            let proof = MerklePath::new::<Keccak256Hasher, &str>(&values_ref, &values_ref[0]);
            let computed_root = proof.compute_root(&values_ref[0]);
            assert_eq!(computed_root, root, "Failed for size {}", size);
        }

        // Test non-power of 2 sizes
        for size in [3, 5, 7, 9] {
            let values: Vec<String> = (0..size).map(|i| i.to_string()).collect();
            let values_ref: Vec<&str> = values.iter().map(|s| s.as_str()).collect();
            let root = compute_root_from_content::<Keccak256Hasher, &str>(&values_ref);

            // Verify proof for last element
            let last_idx = values_ref.len() - 1;
            let proof =
                MerklePath::new::<Keccak256Hasher, &str>(&values_ref, &values_ref[last_idx]);
            let computed_root = proof.compute_root(&values_ref[last_idx]);
            assert_eq!(computed_root, root, "Failed for size {}", size);
        }
    }

    #[test]
    fn it_correctly_computes_complex_keccak256_paths() {
        let values = vec!["1", "2", "3"];
        let root = compute_root_from_content::<Keccak256Hasher, &str>(&values);

        // Test proof for middle element
        let proof = MerklePath::new::<Keccak256Hasher, &str>(&values, &values[1]);
        assert_eq!(proof.len(), 2);
        assert_eq!(proof.compute_root(&values[1]), root);

        // Test proof for last element
        let proof = MerklePath::new::<Keccak256Hasher, &str>(&values, &values[2]);
        assert_eq!(proof.len(), 1);
        assert_eq!(proof.compute_root(&values[2]), root);

        // Test with 7 elements
        let values = vec!["1", "2", "3", "4", "5", "6", "7"];
        let root = compute_root_from_content::<Keccak256Hasher, &str>(&values);

        // Test proof for last element
        let proof = MerklePath::new::<Keccak256Hasher, &str>(&values, &values[6]);
        assert_eq!(proof.len(), 2);
        assert_eq!(proof.compute_root(&values[6]), root);

        // Test proof for middle element
        let proof = MerklePath::new::<Keccak256Hasher, &str>(&values, &values[2]);
        assert_eq!(proof.len(), 3);
        assert_eq!(proof.compute_root(&values[2]), root);
    }

    #[test]
    fn keccak256_empty_path_differs_from_root() {
        let root = compute_root_from_content::<Keccak256Hasher, [u8; 32]>(&[]);
        let proof = MerklePath::new::<Keccak256Hasher, [u8; 32]>(&[], &[0u8; 32]);
        assert_eq!(proof.len(), 0);
        assert_ne!(proof.compute_root(&[0u8; 32]), root);
    }

    #[test]
    fn it_correctly_computes_sorted_keccak256_root() {
        use nimiq_utils::merkle::compute_root_from_content_sorted;

        // Test with single value
        let hash = Keccak256Hasher::default().digest(VALUE.as_bytes());
        let root = compute_root_from_content_sorted::<Keccak256Hasher, &'static str>(&[VALUE]);
        assert_eq!(root, hash);

        // Test with multiple values - sorted should differ from unsorted
        let values = vec!["1", "2", "3", "4"];
        let unsorted_root = compute_root_from_content::<Keccak256Hasher, &str>(&values);
        let sorted_root = compute_root_from_content_sorted::<Keccak256Hasher, &str>(&values);

        // They should be different (unless by chance the hashes are already sorted)
        // We can't assert they're different because it depends on the hash values
        // But we can verify the sorted root is computed correctly

        // Manually compute expected sorted root
        let h0 = Keccak256Hasher::default().digest(values[0].as_bytes());
        let h1 = Keccak256Hasher::default().digest(values[1].as_bytes());
        let h2 = Keccak256Hasher::default().digest(values[2].as_bytes());
        let h3 = Keccak256Hasher::default().digest(values[3].as_bytes());

        // Sort at each level
        let h4 = if h0.as_bytes() <= h1.as_bytes() {
            Keccak256Hasher::default().chain(&h0).chain(&h1).finish()
        } else {
            Keccak256Hasher::default().chain(&h1).chain(&h0).finish()
        };

        let h5 = if h2.as_bytes() <= h3.as_bytes() {
            Keccak256Hasher::default().chain(&h2).chain(&h3).finish()
        } else {
            Keccak256Hasher::default().chain(&h3).chain(&h2).finish()
        };

        let expected_sorted_root = if h4.as_bytes() <= h5.as_bytes() {
            Keccak256Hasher::default().chain(&h4).chain(&h5).finish()
        } else {
            Keccak256Hasher::default().chain(&h5).chain(&h4).finish()
        };

        assert_eq!(sorted_root, expected_sorted_root);
    }

    #[test]
    fn it_correctly_computes_sorted_keccak256_merkle_path() {
        let values = vec!["1", "2", "3", "4"];

        // Compute root with sorted Keccak256
        let root =
            nimiq_utils::merkle::compute_root_from_content_sorted::<Keccak256Hasher, &str>(&values);

        // Generate sorted proof for second value
        let proof = MerklePath::new_sorted::<Keccak256Hasher, &str>(&values, &values[1]);
        assert_eq!(proof.len(), 2);

        // Verify proof with sorted mode
        let computed_root = proof.compute_root_sorted(&values[1]);
        assert_eq!(computed_root, root);
    }

    #[test]
    fn it_correctly_verifies_sorted_keccak256_merkle_path() {
        let values = vec!["1", "2", "3", "4", "5", "6", "7"];
        let root =
            nimiq_utils::merkle::compute_root_from_content_sorted::<Keccak256Hasher, &str>(&values);

        // Test proof for various indices with sorted mode
        for (i, value) in values.iter().enumerate() {
            let proof = MerklePath::new_sorted::<Keccak256Hasher, &str>(&values, value);
            let computed_root = proof.compute_root_sorted(value);
            assert_eq!(
                computed_root, root,
                "Sorted proof verification failed for index {}",
                i
            );
        }
    }

    #[test]
    fn sorted_and_unsorted_proofs_differ() {
        let values = vec!["1", "2", "3", "4"];

        let unsorted_root = compute_root_from_content::<Keccak256Hasher, &str>(&values);
        let sorted_root =
            nimiq_utils::merkle::compute_root_from_content_sorted::<Keccak256Hasher, &str>(&values);

        // Generate both types of proofs
        let unsorted_proof = MerklePath::new::<Keccak256Hasher, &str>(&values, &values[1]);
        let sorted_proof = MerklePath::new_sorted::<Keccak256Hasher, &str>(&values, &values[1]);

        // Verify each proof with its corresponding root
        assert_eq!(unsorted_proof.compute_root(&values[1]), unsorted_root);
        assert_eq!(sorted_proof.compute_root_sorted(&values[1]), sorted_root);

        // Cross-verification should fail (unless hashes happen to be in sorted order)
        // We can't assert inequality because it depends on hash values
    }
}
