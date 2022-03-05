use std::borrow::Cow;
use std::cmp::Ordering;
use std::error;
use std::fmt;
use std::io;
use std::io::Write;
use std::ops::Deref;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_hash::{Blake3Hash, HashOutput, Hasher, SerializeContent};

use crate::math::CeilingDiv;

pub mod incremental;
pub mod partial;

pub fn compute_root_from_content<D: Hasher, T: SerializeContent>(values: &[T]) -> D::Output {
    compute_root_from_content_slice::<D, T>(values)
}

pub fn compute_root_from_content_slice<D: Hasher, T: SerializeContent>(values: &[T]) -> D::Output {
    let mut v: Vec<D::Output> = Vec::with_capacity(values.len());
    for h in values {
        let mut hasher = D::default();
        h.serialize_content(&mut hasher).unwrap();
        v.push(hasher.finish());
    }
    compute_root_from_hashes::<D::Output>(&v)
}

pub fn compute_root_from_hashes<T: HashOutput>(values: &[T]) -> T {
    compute_root_from_slice::<T>(values).into_owned()
}

pub fn compute_root_from_slice<T: HashOutput>(values: &[T]) -> Cow<T> {
    let mut hasher = T::Builder::default();
    match values.len() {
        0 => {
            hasher.write_all(&[]).unwrap();
        }
        1 => {
            return Cow::Borrowed(&values[0]);
        }
        len => {
            let mid = len.ceiling_div(2);
            let left_hash = compute_root_from_slice::<T>(&values[..mid]);
            let right_hash = compute_root_from_slice::<T>(&values[mid..]);
            hasher.hash(left_hash.deref());
            hasher.hash(right_hash.deref());
        }
    };
    Cow::Owned(hasher.finish())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MerklePath<H: HashOutput> {
    nodes: Vec<MerklePathNode<H>>,
}

impl<H> MerklePath<H>
where
    H: HashOutput,
{
    pub fn empty() -> Self {
        MerklePath { nodes: Vec::new() }
    }

    pub fn new<D: Hasher<Output = H>, T: SerializeContent>(
        values: &[T],
        leaf_value: &T,
    ) -> MerklePath<H> {
        let leaf_hash = D::default().chain(leaf_value).finish();
        let mut path: Vec<MerklePathNode<D::Output>> = Vec::new();
        MerklePath::<H>::compute::<D, T>(values, &leaf_hash, &mut path);
        MerklePath { nodes: path }
    }

    fn compute<D: Hasher<Output = H>, T: SerializeContent>(
        values: &[T],
        leaf_hash: &D::Output,
        path: &mut Vec<MerklePathNode<H>>,
    ) -> (bool, H) {
        let mut hasher = D::default();
        let mut contains_leaf = false;
        match values.len() {
            0 => {
                hasher.write_all(&[]).unwrap();
            }
            1 => {
                hasher.hash(&values[0]);
                let hash = hasher.finish();
                return (hash.eq(leaf_hash), hash);
            }
            len => {
                let mid = len.ceiling_div(2);
                let (contains_left, left_hash) =
                    MerklePath::<H>::compute::<D, T>(&values[..mid], leaf_hash, path);
                let (contains_right, right_hash) =
                    MerklePath::<H>::compute::<D, T>(&values[mid..], leaf_hash, path);
                hasher.hash(&left_hash);
                hasher.hash(&right_hash);

                if contains_left {
                    path.push(MerklePathNode {
                        hash: right_hash,
                        left: false,
                    });
                    contains_leaf = true;
                } else if contains_right {
                    path.push(MerklePathNode {
                        hash: left_hash,
                        left: true,
                    });
                    contains_leaf = true;
                }
            }
        };
        (contains_leaf, hasher.finish())
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
        root
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn hashes(&self) -> Vec<H> {
        self.nodes.iter().map(|node| node.hash.clone()).collect()
    }

    /// Compress "left" field of every node in the MerklePath to a bit vector.
    fn compress(&self) -> Vec<u8> {
        // There are 3 items in the MerkleProofOperation enum, so we need 2 bits to encode them.
        let num_bytes = self.nodes.len().ceiling_div(8);
        let mut left_bits: Vec<u8> = vec![0; num_bytes];
        for (i, node) in self.nodes.iter().enumerate() {
            if node.left {
                left_bits[i / 8] |= 0x80 >> (i % 8);
            }
        }
        left_bits
    }

    /// Decompress "left" field of every node in the MerklePath to a bit vector.
    #[inline]
    fn decompress(node_index: usize, left_bits: &[u8]) -> bool {
        left_bits[node_index / 8] & (0x80 >> (node_index % 8)) != 0
    }
}

impl<H: HashOutput> Serialize for MerklePath<H> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;
        size += Serialize::serialize(&(self.nodes.len() as u8), writer)?;
        let compressed = self.compress();
        size += writer.write(compressed.as_slice())?;

        for node in self.nodes.iter() {
            size += Serialize::serialize(&node.hash, writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*count*/ 1;
        size += self.nodes.len().ceiling_div(8);
        size += self
            .nodes
            .iter()
            .fold(0, |acc, node| acc + Serialize::serialized_size(&node.hash));
        size
    }
}

impl<H: HashOutput> Deserialize for MerklePath<H> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let count: u8 = Deserialize::deserialize(reader)?;
        let count = count as usize;

        let left_bits_size = count.ceiling_div(8);
        let mut left_bits: Vec<u8> = vec![0; left_bits_size];
        reader.read_exact(left_bits.as_mut_slice())?;

        let mut nodes: Vec<MerklePathNode<H>> = Vec::with_capacity(count);
        for i in 0..count {
            nodes.push(MerklePathNode {
                left: MerklePath::<H>::decompress(i, &left_bits),
                hash: Deserialize::deserialize(reader)?,
            });
        }
        Ok(MerklePath { nodes })
    }
}

pub type Blake3MerklePath = MerklePath<Blake3Hash>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct MerklePathNode<H: HashOutput> {
    hash: H,
    left: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleProof<H: HashOutput> {
    nodes: Vec<H>,
    operations: Vec<MerkleProofOperation>,
}

impl<H> MerkleProof<H>
where
    H: HashOutput,
{
    pub fn new(hashes: &[H], hashes_to_proof: &[H]) -> Self {
        let mut nodes: Vec<H> = Vec::new();
        let mut operations: Vec<MerkleProofOperation> = Vec::new();
        MerkleProof::compute(hashes, hashes_to_proof, &mut nodes, &mut operations);
        MerkleProof { nodes, operations }
    }

    pub fn from_values<T: SerializeContent>(values: &[T], values_to_proof: &[T]) -> Self {
        let hashes: Vec<H> = values
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        let hashes_to_proof: Vec<H> = values_to_proof
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        MerkleProof::new(&hashes, &hashes_to_proof)
    }

    pub fn with_absence<T: SerializeContent + Ord + Clone>(
        values: &[T],
        values_to_proof: &[T],
    ) -> Self {
        let mut final_values_to_proof: Vec<T> = Vec::new();
        let mut values_to_proof: Vec<&T> = values_to_proof.iter().collect();
        values_to_proof.sort();
        let mut leaf_index: usize = 0;
        let mut value_index: usize = 0;
        while value_index < values.len() && leaf_index < values_to_proof.len() {
            let value = &values[value_index];
            match value.cmp(values_to_proof[leaf_index]) {
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
        if leaf_index < values_to_proof.len() && !values.is_empty() {
            final_values_to_proof.push(values[values.len() - 1].clone());
        }

        let hashes: Vec<H> = values
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        let hashes_to_proof: Vec<H> = final_values_to_proof
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        MerkleProof::new(&hashes, &hashes_to_proof)
    }

    fn compute(
        hashes: &[H],
        hashes_to_proof: &[H],
        path: &mut Vec<H>,
        operations: &mut Vec<MerkleProofOperation>,
    ) -> (bool, H) {
        let mut hasher = H::Builder::default();
        match hashes.len() {
            0 => {
                hasher.write_all(&[]).unwrap();
                let hash = hasher.finish();
                path.push(hash.clone());
                operations.push(MerkleProofOperation::ConsumeProof);
                (false, hash)
            }
            1 => {
                let hash = hashes[0].clone(); // We assume all values to be hashed once already.
                let is_leaf = hashes_to_proof.contains(&hash);
                if is_leaf {
                    operations.push(MerkleProofOperation::ConsumeInput);
                } else {
                    path.push(hash.clone());
                    operations.push(MerkleProofOperation::ConsumeProof);
                }
                (is_leaf, hash)
            }
            len => {
                let mut sub_path: Vec<H> = Vec::new();
                let mut sub_operations: Vec<MerkleProofOperation> = Vec::new();

                let mid = len.ceiling_div(2);
                let (contains_left, left_hash) = MerkleProof::<H>::compute(
                    &hashes[..mid],
                    hashes_to_proof,
                    &mut sub_path,
                    &mut sub_operations,
                );
                let (contains_right, right_hash) = MerkleProof::<H>::compute(
                    &hashes[mid..],
                    hashes_to_proof,
                    &mut sub_path,
                    &mut sub_operations,
                );
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
                (true, hash)
            }
        }
    }

    pub fn compute_root_from_values<T: SerializeContent>(
        &self,
        leaf_values: &[T],
    ) -> Result<H, InvalidMerkleProofError> {
        let hashes: Vec<H> = leaf_values
            .iter()
            .map(|v| H::Builder::default().chain(v).finish())
            .collect();
        self.compute_root(hashes)
    }

    pub fn compute_root(&self, leaf_hashes: Vec<H>) -> Result<H, InvalidMerkleProofError> {
        let mut stack: Vec<Cow<H>> = Vec::new();
        let mut input_index: usize = 0;
        let mut proof_index: usize = 0;

        for op in self.operations.iter() {
            match *op {
                MerkleProofOperation::ConsumeProof => {
                    if proof_index >= self.len() {
                        return Err(InvalidMerkleProofError(
                            "Found invalid operation.".to_string(),
                        ));
                    }
                    stack.push(Cow::Borrowed(&self.nodes[proof_index]));
                    proof_index += 1;
                }
                MerkleProofOperation::ConsumeInput => {
                    if input_index >= leaf_hashes.len() {
                        return Err(InvalidMerkleProofError(
                            "Found invalid operation.".to_string(),
                        ));
                    }
                    stack.push(Cow::Borrowed(&leaf_hashes[input_index]));
                    input_index += 1;
                }
                MerkleProofOperation::Hash => {
                    let right_hash = match stack.pop() {
                        Some(node) => node,
                        None => {
                            return Err(InvalidMerkleProofError(
                                "Found invalid operation.".to_string(),
                            ));
                        }
                    };
                    let left_hash = match stack.pop() {
                        Some(node) => node,
                        None => {
                            return Err(InvalidMerkleProofError(
                                "Found invalid operation.".to_string(),
                            ));
                        }
                    };
                    let hash = H::Builder::default()
                        .chain(&*left_hash)
                        .chain(&*right_hash)
                        .finish();
                    stack.push(Cow::Owned(hash));
                }
            }
        }

        // Everything but the root needs to be consumed.
        if stack.len() != 1 || proof_index < self.len() || input_index < leaf_hashes.len() {
            return Err(InvalidMerkleProofError(
                "Did not consume all nodes.".to_string(),
            ));
        }

        let hash = stack.remove(0);
        Ok(hash.into_owned())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Compress vector of MerkleProofOperations in the MerkleProof to a bit vector.
    fn compress(&self) -> Vec<u8> {
        // There are 3 items in the MerkleProofOperation enum, so we need 2 bits to encode them.
        let num_bytes = self.operations.len().ceiling_div(4);
        let mut operation_bits: Vec<u8> = vec![0; num_bytes];
        for (i, operation) in self.operations.iter().enumerate() {
            let op = *operation as u8; // By definition, this can only take up to two bits.
            operation_bits[i / 4] |= op << ((i % 4) * 2);
        }
        operation_bits
    }

    /// Decompress a bit vector into a vector of MerkleProofOperations.
    fn decompress(
        num_operations: usize,
        operation_bits: Vec<u8>,
    ) -> Option<Vec<MerkleProofOperation>> {
        let mut operations: Vec<MerkleProofOperation> = Vec::with_capacity(num_operations);

        for i in 0..num_operations {
            let op = (operation_bits[i / 4] >> ((i % 4) * 2)) & 0x3;
            operations.push(MerkleProofOperation::from_u8(op)?);
        }

        Some(operations)
    }
}

impl<H: HashOutput> Serialize for MerkleProof<H> {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;
        size += Serialize::serialize(&(self.operations.len() as u16), writer)?;
        let compressed = self.compress();
        size += writer.write(compressed.as_slice())?;

        size += SerializeWithLength::serialize::<u16, W>(&self.nodes, writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = /*operations*/ 2;
        size += self.operations.len().ceiling_div(4);
        size += SerializeWithLength::serialized_size::<u16>(&self.nodes);
        size
    }
}

impl<H: HashOutput> Deserialize for MerkleProof<H> {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let count: u16 = Deserialize::deserialize(reader)?;
        let count = count as usize;

        let operations_size = count.ceiling_div(4);
        let mut operation_bits: Vec<u8> = vec![0; operations_size];
        reader.read_exact(operation_bits.as_mut_slice())?;
        let operations = match MerkleProof::<H>::decompress(count, operation_bits) {
            Some(operations) => operations,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "invalid operation in bitvector",
                )
                .into())
            }
        };

        Ok(MerkleProof {
            operations,
            nodes: DeserializeWithLength::deserialize::<u16, R>(reader)?,
        })
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
// The implementation assumes the operations to take at most 2 bits.
enum MerkleProofOperation {
    ConsumeProof = 0,
    ConsumeInput = 1,
    Hash = 2,
}

impl MerkleProofOperation {
    fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(MerkleProofOperation::ConsumeProof),
            1 => Some(MerkleProofOperation::ConsumeInput),
            2 => Some(MerkleProofOperation::Hash),
            _ => None,
        }
    }
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

    fn cause(&self) -> Option<&dyn error::Error> {
        None
    }
}

pub type Blake3MerkleProof = MerkleProof<Blake3Hash>;

// Simple test that checks the order of operations.
#[cfg(test)]
mod tests {
    use nimiq_hash::Blake3Hasher;

    use super::*;

    #[test]
    fn it_correctly_computes_a_simple_proof() {
        let values = vec!["1", "2", "3"];
        /*
         * (X) should be the nodes included in the proof.
         * *X* marks the values to be proven.
         *            h4
         *         /      \
         *     (h3)        h2
         *     / \          |
         *   h0  h1       *v2*
         *   |    |
         *  v0   v1
         */
        let root = compute_root_from_content::<Blake3Hasher, &str>(&values);
        let proof: MerkleProof<Blake3Hash> =
            MerkleProof::from_values::<&str>(&values, &[values[2]]);
        assert_eq!(proof.len(), 1);

        // Check proof internals.
        assert_eq!(
            &proof.operations,
            &vec![
                MerkleProofOperation::ConsumeProof,
                MerkleProofOperation::ConsumeInput,
                MerkleProofOperation::Hash,
            ]
        );
        // Check serialization.
        assert_eq!(proof.compress(), vec![36]);

        let proof_root = proof.compute_root_from_values(&[values[2]]);
        assert!(proof_root.is_ok());
        assert_eq!(proof_root.unwrap(), root);

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
        let root = compute_root_from_content::<Blake3Hasher, &str>(&values);
        let proof: MerkleProof<Blake3Hash> =
            MerkleProof::from_values::<&str>(&values, &[values[1]]);
        assert_eq!(proof.len(), 2);

        // Check proof internals.
        assert_eq!(
            &proof.operations,
            &vec![
                MerkleProofOperation::ConsumeProof,
                MerkleProofOperation::ConsumeInput,
                MerkleProofOperation::Hash,
                MerkleProofOperation::ConsumeProof,
                MerkleProofOperation::Hash,
            ]
        );
        // Check serialization.
        assert_eq!(proof.compress(), vec![36, 2]);

        let proof_root = proof.compute_root_from_values(&[values[1]]);
        assert!(proof_root.is_ok());
        assert_eq!(proof_root.unwrap(), root);

        let proof: MerkleProof<Blake3Hash> = MerkleProof::from_values::<&str>(&values, &[]);
        assert_eq!(proof.len(), 1);

        // Check proof internals.
        assert_eq!(
            &proof.operations,
            &vec![MerkleProofOperation::ConsumeProof,]
        );

        let proof_root = proof.compute_root_from_values::<&str>(&[]);
        assert!(proof_root.is_ok());
        assert_eq!(proof_root.unwrap(), root);
    }
}
