use core_rs::utils::merkle::{compute_root, compute_root_from_slice, MerklePath};
use core_rs::consensus::base::primitive::hash::{Hasher, Blake2bHasher};

const VALUE: &'static str = "merkletree";

#[test]
fn it_correctly_computes_an_empty_root_hash() {
    let empty_hash = Blake2bHasher::default().digest(&[]);
    let root = compute_root::<Blake2bHasher, [u8; 32]>(&vec![]);
    assert_eq!(root, empty_hash);
    let root = compute_root_from_slice::<Blake2bHasher, [u8; 32]>(&[]);
    assert_eq!(root, empty_hash);
}

#[test]
fn it_correctly_computes_a_simple_root_hash() {
    let hash = Blake2bHasher::default().digest(VALUE.as_bytes());
    let root = compute_root::<Blake2bHasher, &'static str>(&vec![VALUE]);
    assert_eq!(root, hash);
    let root = compute_root_from_slice::<Blake2bHasher, &'static str>(&[VALUE]);
    assert_eq!(root, hash);
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
    let level1 = Blake2bHasher::default().chain(&level0).chain(&level0).finish();
    let level2 = Blake2bHasher::default().chain(&level1).chain(&level1).finish();
    let root = compute_root::<Blake2bHasher, &'static str>(&vec![VALUE, VALUE, VALUE, VALUE]);
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
    let level2a = Blake2bHasher::default().chain(&level1).chain(&level0).finish();
    let root = compute_root::<Blake2bHasher, &'static str>(&vec![VALUE, VALUE, VALUE]);
    assert_eq!(root, level2a);
}

#[test]
fn it_correctly_computes_an_empty_proof() {
    let root = compute_root::<Blake2bHasher, [u8; 32]>(&vec![]);
    let proof = MerklePath::new::<Blake2bHasher, [u8; 32]>(&vec![], &[0u8; 32]);
    assert_eq!(proof.len(), 0);
    assert_ne!(proof.compute_root(&[0u8; 32]), root);
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
    let root = compute_root::<Blake2bHasher, &str>(&values);
    let proof = MerklePath::new::<Blake2bHasher, &str>(&values, &values[1]);
    assert_eq!(proof.len(), 2);
    assert_eq!(proof.compute_root(&values[1]), root);
}

#[test]
fn it_correctly_computes_more_complex_proofs() {
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
    let root = compute_root::<Blake2bHasher, &str>(&values);
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
    let root = compute_root::<Blake2bHasher, &str>(&values);
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
