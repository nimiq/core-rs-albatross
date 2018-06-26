use consensus::base::primitive::hash::Hash;
use consensus::base::primitive::hash::Hasher;
use beserial::{Serialize, Deserialize};
use std::marker::PhantomData;
use consensus::base::primitive::hash::HashOutput;

pub fn compute_root<D: Hasher, T: Hash<D>>(values: &Vec<T>) -> D::Output where D::Output: Hash<D> {
    return compute_root_from_slice(values.as_slice());
}

pub fn compute_root_from_slice<D: Hasher, T: Hash<D>>(values: &[T]) -> D::Output where D::Output: Hash<D> {
    let mut hasher = D::default();
    match values.len() {
        0 => {
            hasher.write(&[]);
        },
        1 => {
            hasher.hash(&values[0]);
        },
        len => {
            let mid = (len + 1) / 2; // Equivalent to round(len / 2.0)
            let left_hash = compute_root_from_slice(&values[..mid]);
            let right_hash = compute_root_from_slice(&values[mid..]);
            hasher.hash(&left_hash);
            hasher.hash(&right_hash);
        },
    };
    return hasher.finish();
}

struct MerklePath<H: HashOutput + Hash<<H as HashOutput>::Hasher> + Serialize + Deserialize> {
    nodes: Vec<H>
}

struct MerklePathNode<H: HashOutput + Hash<<H as HashOutput>::Hasher> + Serialize + Deserialize> {
    hash: H,
    left: bool
}
