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
