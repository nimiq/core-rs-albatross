extern crate bit_vec;

use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, WriteBytesExt};
use consensus::base::primitive::hash::{Blake2bHash, Hasher, HashOutput, SerializeContent};
use self::bit_vec::BitVec;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::error;
use std::fmt;
use std::io;
use std::io::Write;
use std::ops::Deref;

pub fn compute_root_from_content<D: Hasher, T: SerializeContent>(values: &Vec<T>) -> D::Output {
    return compute_root_from_content_slice::<D, T>(values.as_slice());
}

pub fn compute_root_from_content_slice<D: Hasher, T: SerializeContent>(values: &[T]) -> D::Output {
    let mut v: Vec<D::Output> = Vec::with_capacity(values.len());
    for h in values {
        let mut hasher = D::default();
        h.serialize_content(&mut hasher).unwrap();
        v.push(hasher.finish());
    }
    return compute_root_from_hashes::<D::Output>(&v);
}

pub fn compute_root_from_hashes<T: HashOutput>(values: &Vec<T>) -> T {
    return compute_root_from_slice::<T>(values.as_slice()).into_owned();
}

pub fn compute_root_from_slice<T: HashOutput>(values: &[T]) -> Cow<T> {
    let mut hasher = T::Builder::default();
    match values.len() {
        0 => {
            hasher.write(&[]).unwrap();
        }
        1 => {
            return Cow::Borrowed(&values[0]);
        }
        len => {
            let mid = (len + 1) / 2; // Equivalent to round(len / 2.0)
            let left_hash = compute_root_from_slice::<T>(&values[..mid]);
            let right_hash = compute_root_from_slice::<T>(&values[mid..]);
            hasher.hash(left_hash.deref());
            hasher.hash(right_hash.deref());
        }
    };
    return Cow::Owned(hasher.finish());
}

#[derive(Debug, Eq, PartialEq)]
pub struct MerklePath<H: HashOutput> {
    nodes: Vec<MerklePathNode<H>>
}

impl<H> MerklePath<H> where H: HashOutput {
    pub fn empty() -> Self {
        return MerklePath {
            nodes: Vec::new()
        };
    }

    pub fn new<D: Hasher<Output=H>, T: SerializeContent>(values: &Vec<T>, leaf_value: &T) -> MerklePath<H> {
        let leaf_hash = D::default().chain(leaf_value).finish();
        let mut path: Vec<MerklePathNode<D::Output>> = Vec::new();
        MerklePath::<H>::compute::<D, T>(values.as_slice(), &leaf_hash, &mut path);
        return MerklePath { nodes: path };
    }

    fn compute<D: Hasher<Output=H>, T: SerializeContent>(values: &[T], leaf_hash: &D::Output, path: &mut Vec<MerklePathNode<H>>) -> (bool, H) {
        let mut hasher = D::default();
        let mut contains_leaf = false;
        match values.len() {
            0 => {
                hasher.write(&[]).unwrap();
            }
            1 => {
                hasher.hash(&values[0]);
                let hash = hasher.finish();
                return (hash.eq(leaf_hash), hash);
            }
            len => {
                let mid = (len + 1) / 2; // Equivalent to round(len / 2.0)
                let (contains_left, left_hash) = MerklePath::<H>::compute::<D, T>(&values[..mid], &leaf_hash, path);
                let (contains_right, right_hash) = MerklePath::<H>::compute::<D, T>(&values[mid..], &leaf_hash, path);
                hasher.hash(&left_hash);
                hasher.hash(&right_hash);

                if contains_left {
                    path.push(MerklePathNode { hash: right_hash, left: false });
                    contains_leaf = true;
                } else if contains_right {
                    path.push(MerklePathNode { hash: left_hash, left: true });
                    contains_leaf = true;
                }
            }
        };
        return (contains_leaf, hasher.finish());
    }

    pub fn compute_root<T: SerializeContent>(&self, leaf_value: &T) -> H {
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

    // Compress "left" field of every node in the MerklePath to a bit vector.
    fn compress(&self) -> BitVec {
        let mut left_bits = BitVec::from_elem(self.len(), false);
        for (i, node) in self.nodes.iter().enumerate() {
            if node.left {
                left_bits.set(i, true);
            }
        }
        return left_bits;
    }
}

impl<H: HashOutput> Serialize for MerklePath<H> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size: usize = 0;
        size += Serialize::serialize(&(self.nodes.len() as u8), writer)?;
        let compressed = self.compress();
        size += writer.write(compressed.to_bytes().as_slice())?;

        for node in self.nodes.iter() {
            size += Serialize::serialize(&node.hash, writer)?;
        }
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*count*/ 1;
        size += (self.nodes.len() + 7) / 8; // For rounding up: (num + divisor - 1) / divisor
        size += self.nodes.iter().fold(0, |acc, node| acc + Serialize::serialized_size(&node.hash));
        return size;
    }
}

impl<H: HashOutput> Deserialize for MerklePath<H> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let count: u8 = Deserialize::deserialize(reader)?;

        let left_bits_size = (count + 7) / 8; // For rounding up: (num + divisor - 1) / divisor
        let mut left_bits: Vec<u8> = vec![0; left_bits_size as usize];
        reader.read_exact(left_bits.as_mut_slice())?;
        let left_bits = BitVec::from_bytes(&left_bits);

        let mut nodes: Vec<MerklePathNode<H>> = Vec::with_capacity(count as usize);
        for i in 0..count {
            if let Some(left) = left_bits.get(i as usize) {
                nodes.push(MerklePathNode {
                    left,
                    hash: Deserialize::deserialize(reader)?,
                });
            } else {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "failed to read left bits"));
            }
        }
        return Ok(MerklePath { nodes });
    }
}

pub type Blake2bMerklePath = MerklePath<Blake2bHash>;

#[derive(Debug, Eq, PartialEq)]
struct MerklePathNode<H: HashOutput> {
    hash: H,
    left: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub struct MerkleProof<H: HashOutput> {
    nodes: Vec<H>,
    operations: Vec<MerkleProofOperation>,
}

impl<H> MerkleProof<H> where H: HashOutput {
    pub fn new<D: Hasher<Output=H>, T: SerializeContent>(values: &[T], values_to_proof: &[T]) -> Self {
        let hashes_to_proof: Vec<D::Output> = values_to_proof.iter().map(|v| { D::default().chain(v).finish() }).collect();
        let mut nodes: Vec<D::Output> = Vec::new();
        let mut operations: Vec<MerkleProofOperation> = Vec::new();
        MerkleProof::compute::<D, T>(values, hashes_to_proof.as_slice(), &mut nodes, &mut operations);
        return MerkleProof {
            nodes,
            operations,
        };
    }

    pub fn new_with_absence<D: Hasher<Output=H>, T: SerializeContent + Ord + Clone>(values: &[T], values_to_proof: &[T]) -> Self {
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
                }
                // Leave should already have been there, so it is missing.
                Ordering::Greater => {
                    // Use both, prevValue and value, as a proof of absence.
                    // Special case: prevValue unknown as we're at the first value.
                    if value_index > 0 {
                        final_values_to_proof.push(values[value_index - 1].clone());
                    }
                    final_values_to_proof.push(value.clone());
                    leaf_index += 1;
                }
                // This value is not interesting for us, skip it.
                Ordering::Less => {
                    value_index += 1;
                }
            }
        }
        // If we processed all values but not all leaves, these are missing. Add last value as proof.
        if leaf_index < values_to_proof.len() && values.len() > 0 {
            final_values_to_proof.push(values[values.len() - 1].clone());
        }
        return MerkleProof::new::<D, T>(values, final_values_to_proof.as_slice());
    }

    fn compute<D: Hasher<Output=H>, T: SerializeContent>(values: &[T], hashes_to_proof: &[D::Output], path: &mut Vec<H>, operations: &mut Vec<MerkleProofOperation>) -> (bool, H) {
        let mut hasher = D::default();
        match values.len() {
            0 => {
                hasher.write(&[]).unwrap();
                let hash = hasher.finish();
                path.push(hash.clone());
                operations.push(MerkleProofOperation::ConsumeProof);
                return (false, hash);
            }
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
            }
            len => {
                let mut sub_path: Vec<D::Output> = Vec::new();
                let mut sub_operations: Vec<MerkleProofOperation> = Vec::new();

                let mid = (len + 1) / 2; // Equivalent to round(len / 2.0)
                let (contains_left, left_hash) = MerkleProof::<D::Output>::compute::<D, T>(&values[..mid], &hashes_to_proof, &mut sub_path, &mut sub_operations);
                let (contains_right, right_hash) = MerkleProof::<D::Output>::compute::<D, T>(&values[mid..], &hashes_to_proof, &mut sub_path, &mut sub_operations);
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
            }
        };
    }

    pub fn compute_root<T: SerializeContent>(&self, leaf_values: &[T]) -> Result<H, InvalidMerkleProofError> {
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
                }
                MerkleProofOperation::ConsumeInput => {
                    if input_index >= inputs.len() {
                        return Err(InvalidMerkleProofError("Found invalid operation.".to_string()));
                    }
                    stack.push(Cow::Borrowed(&inputs[input_index]));
                    input_index += 1;
                }
                MerkleProofOperation::Hash => {
                    let right_hash = match stack.pop() {
                        Some(node) => { node }
                        None => {
                            return Err(InvalidMerkleProofError("Found invalid operation.".to_string()));
                        }
                    };
                    let left_hash = match stack.pop() {
                        Some(node) => { node }
                        None => {
                            return Err(InvalidMerkleProofError("Found invalid operation.".to_string()));
                        }
                    };
                    let hash = H::Builder::default().chain(&*left_hash).chain(&*right_hash).finish();
                    stack.push(Cow::Owned(hash));
                }
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

    // Compress Vector of MerkleProofOperation's in the MerkleProof to a bit vector.
    fn compress(&self) -> BitVec {
        // There are 3 items in the MerkleProofOperation enum, so we need 2 bits to encode them,
        // hence the .len() * 2 in the capacity of the BitVec.
        let mut operations_bits = BitVec::from_elem(self.operations.len() * 2, false);
        for (i, operation) in self.operations.iter().enumerate() {
            match *operation {
                MerkleProofOperation::ConsumeProof => {
                    operations_bits.set(i * 2, false);
                    operations_bits.set(i * 2 + 1, false);
                }
                MerkleProofOperation::ConsumeInput => {
                    operations_bits.set(i * 2, false);
                    operations_bits.set(i * 2 + 1, true);
                }
                MerkleProofOperation::Hash => {
                    operations_bits.set(i * 2, true);
                    operations_bits.set(i * 2 + 1, false);
                }
            }
        }
        return operations_bits;
    }
}

impl<H: HashOutput> Serialize for MerkleProof<H> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let mut size: usize = 0;
        size += Serialize::serialize(&(self.operations.len() as u16), writer)?;
        let compressed = self.compress();
        size += writer.write(compressed.to_bytes().as_slice())?;

        size += SerializeWithLength::serialize::<u16, W>(&self.nodes, writer)?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*operations*/ 2;
        size += (self.operations.len() + 3) / 4; // For rounding up: (num + divisor - 1) / divisor
        size += SerializeWithLength::serialized_size::<u16>(&self.nodes);
        return size;
    }
}

impl<H: HashOutput> Deserialize for MerkleProof<H> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let count: u16 = Deserialize::deserialize(reader)?;

        let operations_size = (count + 3) / 4; // For rounding up: (num + divisor - 1) / divisor
        let mut operation_bits: Vec<u8> = vec![0; operations_size as usize];
        reader.read_exact(operation_bits.as_mut_slice())?;
        let operation_bits = BitVec::from_bytes(&operation_bits);

        let mut operations: Vec<MerkleProofOperation> = Vec::with_capacity(count as usize);
        for i in 0..(count as usize) {
            let op = match (operation_bits.get(i * 2), operation_bits.get(i * 2 + 1)) {
                (Some(false), Some(false)) => MerkleProofOperation::ConsumeProof,
                (Some(false), Some(true)) => MerkleProofOperation::ConsumeInput,
                (Some(true), Some(false)) => MerkleProofOperation::Hash,
                _ => { return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "invalid operation in bitvector")); }
            };
            operations.push(op);
        }

        return Ok(MerkleProof {
            operations,
            nodes: DeserializeWithLength::deserialize::<u16, R>(reader)?,
        });
    }
}

#[derive(Debug, Eq, PartialEq)]
enum MerkleProofOperation {
    ConsumeProof,
    ConsumeInput,
    Hash,
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
