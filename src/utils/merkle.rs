use consensus::base::primitive::hash::Hash;
use consensus::base::primitive::hash::Hasher;
use beserial::{Serialize, Deserialize};
use consensus::base::primitive::hash::HashOutput;
use std::cmp::Ordering;
use std::error;
use std::fmt;
use std::borrow::Cow;

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

#[derive(Debug)]
pub struct MerklePath<H: HashOutput> {
    nodes: Vec<MerklePathNode<H>>
}

impl<H> MerklePath<H> where H: HashOutput {
    pub fn new<D: Hasher<Output=H>, T: Hash<D>>(values: &Vec<T>, leaf_value: &T) -> MerklePath<D::Output> where D::Output: Hash<D> {
        let leaf_hash = D::default().chain(leaf_value).finish();
        let mut path: Vec<MerklePathNode<D::Output>> = Vec::new();
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

#[derive(Debug)]
struct MerklePathNode<H: HashOutput> {
    hash: H,
    left: bool
}

#[derive(Debug)]
pub struct MerkleProof<H: HashOutput> {
    nodes: Vec<H>,
    operations: Vec<MerkleProofOperation>
}

impl<H> MerkleProof<H> where H: HashOutput {
    pub fn new<D: Hasher<Output=H>, T: Hash<D>>(values: &[T], values_to_proof: &[T]) -> Self where D::Output: Hash<D> {
        let hashes_to_proof: Vec<D::Output> = values_to_proof.iter().map(|v| { D::default().chain(v).finish() }).collect();
        let mut nodes: Vec<D::Output> = Vec::new();
        let mut operations: Vec<MerkleProofOperation> = Vec::new();
        MerkleProof::compute(values, hashes_to_proof.as_slice(), &mut nodes, &mut operations);
        return MerkleProof {
            nodes,
            operations
        };
    }

    pub fn new_with_absence<D: Hasher<Output=H>, T: Hash<D> + Ord + Clone>(values: &[T], values_to_proof: &[T]) -> Self where D::Output: Hash<D> {
        let mut final_values_to_proof: Vec<T> = Vec::new();
        let mut values_to_proof: Vec<&T> = values_to_proof.iter().map(|v| { v }).collect();
        values_to_proof.sort();
        let mut leaf_index: usize = 0;
        let mut value_index: usize = 0;
        while value_index < values.len() && leaf_index < values_to_proof.len() {
            let value = &values[value_index];
            match value.cmp(&values_to_proof[leaf_index]) {
                // Leaf is included.
                Ordering::Equal => {
                    final_values_to_proof.push(values_to_proof[leaf_index].clone());
                    leaf_index += 1;
                },
                // Leave should already have been there, so it is missing.
                Ordering::Greater => {
                    // Use both, prevValue and value, as a proof of absence.
                    // Special case: prevValue unknown as we're at the first value.
                    if value_index > 0 {
                        final_values_to_proof.push(values[value_index - 1].clone());
                    }
                    final_values_to_proof.push(value.clone());
                    leaf_index += 1;
                },
                // This value is not interesting for us, skip it.
                Ordering::Less => {
                    value_index += 1;
                },
            }
        }
        // If we processed all values but not all leaves, these are missing. Add last value as proof.
        if leaf_index < values_to_proof.len() && values.len() > 0 {
            final_values_to_proof.push(values[values.len() - 1].clone());
        }
        return MerkleProof::new(values, final_values_to_proof.as_slice());
    }

    fn compute<D: Hasher<Output=H>, T: Hash<D>>(values: &[T], hashes_to_proof: &[D::Output], path: &mut Vec<D::Output>, operations: &mut Vec<MerkleProofOperation>) -> (bool, D::Output) where D::Output: Hash<D> {
        let mut hasher = D::default();
        match values.len() {
            0 => {
                hasher.write(&[]);
                let hash = hasher.finish();
                path.push(hash.clone());
                operations.push(MerkleProofOperation::ConsumeProof);
                return (false, hash);
            },
            1 => {
                hasher.hash(&values[0]);
                let hash = hasher.finish();
                let is_leaf = hashes_to_proof.contains(&hash);
                if is_leaf {
                    operations.push(MerkleProofOperation::ConsumeInput);
                } else {
                    path.push(hash.clone());
                    operations.push(MerkleProofOperation::ConsumeProof);
                }
                return (is_leaf, hash);
            },
            len => {
                let mut sub_path: Vec<D::Output> = Vec::new();
                let mut sub_operations: Vec<MerkleProofOperation> = Vec::new();

                let mid = (len + 1) / 2; // Equivalent to round(len / 2.0)
                let (contains_left, left_hash) = MerkleProof::<D::Output>::compute(&values[..mid], &hashes_to_proof, &mut sub_path, &mut sub_operations);
                let (contains_right, right_hash) = MerkleProof::<D::Output>::compute(&values[mid..], &hashes_to_proof, &mut sub_path, &mut sub_operations);
                hasher.hash(&left_hash);
                hasher.hash(&right_hash);

                let hash = hasher.finish();
                if !contains_left && !contains_right {
                    path.push(hash.clone());
                    operations.push(MerkleProofOperation::ConsumeProof);
                    return (false, hash);
                }

                path.extend(sub_path);
                operations.extend(sub_operations);
                operations.push(MerkleProofOperation::Hash);
                return (true, hash);
            },
        };
    }

    pub fn compute_root<T>(&self, leaf_values: &[T]) -> Result<H, InvalidMerkleProofError> where T: Hash<H::Builder> {
        let inputs: Vec<H> = leaf_values.iter().map(|v| { H::Builder::default().chain(v).finish() }).collect();
        let mut stack: Vec<Cow<H>> = Vec::new();
        let mut input_index: usize = 0;
        let mut proof_index: usize = 0;

        for op in self.operations.iter() {
            match *op {
                MerkleProofOperation::ConsumeProof => {
                    if proof_index >= self.len() {
                        return Err(InvalidMerkleProofError("Found invalid operation.".to_string()));
                    }
                    stack.push(Cow::Borrowed(&self.nodes[proof_index]));
                    proof_index += 1;
                },
                MerkleProofOperation::ConsumeInput => {
                    if input_index >= inputs.len() {
                        return Err(InvalidMerkleProofError("Found invalid operation.".to_string()));
                    }
                    stack.push(Cow::Borrowed(&inputs[input_index]));
                    input_index += 1;
                },
                MerkleProofOperation::Hash => {
                    let right_hash = match stack.pop() {
                        Some(node) => { node },
                        None => {
                            return Err(InvalidMerkleProofError("Found invalid operation.".to_string()));
                        },
                    };
                    let left_hash = match stack.pop() {
                        Some(node) => { node },
                        None => {
                            return Err(InvalidMerkleProofError("Found invalid operation.".to_string()));
                        },
                    };
                    let hash = H::Builder::default().chain(&*left_hash).chain(&*right_hash).finish();
                    stack.push(Cow::Owned(hash));
                },
            }
        }

        // Everything but the root needs to be consumed.
        if stack.len() != 1 || proof_index < self.len() || input_index < inputs.len() {
            return Err(InvalidMerkleProofError("Did not consume all nodes.".to_string()));
        }

        let hash = stack.remove(0);
        return Ok(hash.into_owned());
    }

    #[inline]
    pub fn len(&self) -> usize {
        return self.nodes.len();
    }
}

#[derive(Debug)]
enum MerkleProofOperation {
    ConsumeProof,
    ConsumeInput,
    Hash
}

#[derive(Debug, Clone)]
pub struct InvalidMerkleProofError(String);

impl fmt::Display for InvalidMerkleProofError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl error::Error for InvalidMerkleProofError {
    fn description(&self) -> &str {
        &self.0
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}
