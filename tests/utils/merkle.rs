use core_rs::utils::merkle::{compute_root, compute_root_from_slice};
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
