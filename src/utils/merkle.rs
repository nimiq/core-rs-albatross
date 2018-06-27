use consensus::base::primitive::hash::Hash;
use consensus::base::primitive::hash::Hasher;
use beserial::{Serialize, Deserialize};
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

pub struct MerklePath<H: HashOutput> {
    nodes: Vec<MerklePathNode<H>>
}

impl<H> MerklePath<H> where H: HashOutput {
    pub fn new<D: Hasher<Output=H>, T: Hash<D>>(values: &Vec<T>, leaf_value: &T) -> MerklePath<D::Output> where D::Output: Hash<D> {
        let leaf_hash = D::default().chain(leaf_value).finish();
        let mut path: Vec<MerklePathNode<D::Output>> = vec![];
        MerklePath::<D::Output>::compute(values.as_slice(), &leaf_hash, &mut path);
        return MerklePath { nodes: path };
    }

    fn compute<D: Hasher<Output=H>, T: Hash<D>>(values: &[T], leaf_hash: &D::Output, path: &mut Vec<MerklePathNode<D::Output>>) -> (bool, D::Output) where D::Output: Hash<D> {
        let mut hasher = D::default();
        let mut contains_leaf = false;
        match values.len() {
            0 => {
                hasher.write(&[]);
            },
            1 => {
                hasher.hash(&values[0]);
                let hash = hasher.finish();
                return (hash.eq(leaf_hash), hash);
            },
            len => {
                let mid = (len + 1) / 2; // Equivalent to round(len / 2.0)
                let (contains_left, left_hash) = MerklePath::<D::Output>::compute(&values[..mid], &leaf_hash, path);
                let (contains_right, right_hash) = MerklePath::<D::Output>::compute(&values[mid..], &leaf_hash, path);
                hasher.hash(&left_hash);
                hasher.hash(&right_hash);

                if contains_left {
                    path.push(MerklePathNode { hash: right_hash, left: false });
                    contains_leaf = true;
                } else if contains_right {
                    path.push(MerklePathNode { hash: left_hash, left: true });
                    contains_leaf = true;
                }
            },
        };
        return (contains_leaf, hasher.finish());
    }

    pub fn compute_root<T>(&self, leaf_value: &T) -> <H::Builder as Hasher>::Output where T: Hash<H::Builder> {
        let mut root = H::Builder::default().chain(leaf_value).finish();
        for node in self.nodes.iter() {
            let mut h = H::Builder::default();
            if node.left {
                h.hash(&node.hash);
            }
            h.hash(&root);
            if !node.left {
                h.hash(&node.hash);
            }
            root = h.finish();
        }
        return root;
    }

    #[inline]
    pub fn len(&self) -> usize {
        return self.nodes.len();
    }
}

struct MerklePathNode<H: HashOutput> {
    hash: H,
    left: bool
}
