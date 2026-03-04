use std::{borrow::Cow, cmp::Ordering, error, fmt, io::Write, marker::PhantomData};

use nimiq_hash::{Blake2bHash, HashOutput, Hasher, SerializeContent};
#[cfg(test)]
use nimiq_serde::Deserialize as NimiqDeserialize;
use serde::{
    de::{Error, SeqAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};

pub mod incremental;
pub mod partial;

pub fn compute_root_from_content<D: Hasher, T: SerializeContent>(values: &[T]) -> D::Output {
    compute_root_from_content_internal::<D, T>(values, false)
}

pub fn compute_root_from_content_sorted<D: Hasher, T: SerializeContent>(values: &[T]) -> D::Output {
    compute_root_from_content_internal::<D, T>(values, true)
}

fn compute_root_from_content_internal<D: Hasher, T: SerializeContent>(
    values: &[T],
    sorted: bool,
) -> D::Output {
    let mut v: Vec<D::Output> = Vec::with_capacity(values.len());
    for h in values {
        let mut hasher = D::default();
        h.serialize_content::<_, D::Output>(&mut hasher).unwrap();
        v.push(hasher.finish());
    }
    if sorted {
        compute_root_from_hashes_sorted::<D::Output>(&v).into_owned()
    } else {
        compute_root_from_hashes::<D::Output>(&v).into_owned()
    }
}

pub fn compute_root_from_hashes<T: HashOutput>(values: &[T]) -> Cow<'_, T> {
    compute_root_from_hashes_internal::<T>(values, false)
}

pub fn compute_root_from_hashes_sorted<T: HashOutput>(values: &[T]) -> Cow<'_, T> {
    compute_root_from_hashes_internal::<T>(values, true)
}

fn compute_root_from_hashes_internal<T: HashOutput>(values: &[T], sorted: bool) -> Cow<'_, T> {
    let mut hasher = T::Builder::default();
    match values.len() {
        0 => {
            hasher.write_all(&[]).unwrap();
        }
        1 => {
            return Cow::Borrowed(&values[0]);
        }
        len => {
            let mid = len.div_ceil(2);
            let left_hash = compute_root_from_hashes_internal::<T>(&values[..mid], sorted);
            let right_hash = compute_root_from_hashes_internal::<T>(&values[mid..], sorted);

            if sorted {
                // Sort hashes lexicographically for OpenZeppelin compatibility
                if left_hash.as_bytes() <= right_hash.as_bytes() {
                    hasher.hash(&*left_hash);
                    hasher.hash(&*right_hash);
                } else {
                    hasher.hash(&*right_hash);
                    hasher.hash(&*left_hash);
                }
            } else {
                hasher.hash(&*left_hash);
                hasher.hash(&*right_hash);
            }
        }
    };
    Cow::Owned(hasher.finish())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MerklePath<H: HashOutput> {
    nodes: Vec<MerklePathNode<H>>,
}

impl<H: HashOutput> MerklePath<H> {
    pub fn empty() -> Self {
        MerklePath { nodes: Vec::new() }
    }

    pub fn new<D: Hasher<Output = H>, T: SerializeContent>(
        values: &[T],
        leaf_value: &T,
    ) -> MerklePath<H> {
        Self::new_internal::<D, T>(values, leaf_value, false)
    }

    pub fn new_sorted<D: Hasher<Output = H>, T: SerializeContent>(
        values: &[T],
        leaf_value: &T,
    ) -> MerklePath<H> {
        Self::new_internal::<D, T>(values, leaf_value, true)
    }

    fn new_internal<D: Hasher<Output = H>, T: SerializeContent>(
        values: &[T],
        leaf_value: &T,
        sorted: bool,
    ) -> MerklePath<H> {
        let leaf_hash = D::default().chain(leaf_value).finish();
        let mut path: Vec<MerklePathNode<D::Output>> = Vec::new();
        MerklePath::<H>::compute_internal::<D, T>(values, &leaf_hash, &mut path, sorted);
        MerklePath { nodes: path }
    }

    fn compute_internal<D: Hasher<Output = H>, T: SerializeContent>(
        values: &[T],
        leaf_hash: &D::Output,
        path: &mut Vec<MerklePathNode<H>>,
        sorted: bool,
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
                let mid = len.div_ceil(2);
                let (contains_left, left_hash) = MerklePath::<H>::compute_internal::<D, T>(
                    &values[..mid],
                    leaf_hash,
                    path,
                    sorted,
                );
                let (contains_right, right_hash) = MerklePath::<H>::compute_internal::<D, T>(
                    &values[mid..],
                    leaf_hash,
                    path,
                    sorted,
                );

                if sorted {
                    // For sorted mode, hash in lexicographic order
                    if left_hash.as_bytes() <= right_hash.as_bytes() {
                        hasher.hash(&left_hash);
                        hasher.hash(&right_hash);
                    } else {
                        hasher.hash(&right_hash);
                        hasher.hash(&left_hash);
                    }
                } else {
                    hasher.hash(&left_hash);
                    hasher.hash(&right_hash);
                }

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
        self.compute_root_internal::<T>(leaf_value, false)
    }

    pub fn compute_root_sorted<T: SerializeContent>(&self, leaf_value: &T) -> H {
        self.compute_root_internal::<T>(leaf_value, true)
    }

    fn compute_root_internal<T: SerializeContent>(&self, leaf_value: &T, sorted: bool) -> H {
        let mut root = H::Builder::default().chain(leaf_value).finish();
        for node in self.nodes.iter() {
            let mut h = H::Builder::default();
            if sorted {
                // For sorted mode, always sort the hashes lexicographically
                if root.as_bytes() <= node.hash.as_bytes() {
                    h.hash(&root);
                    h.hash(&node.hash);
                } else {
                    h.hash(&node.hash);
                    h.hash(&root);
                }
            } else {
                // For unsorted mode, use the left flag
                if node.left {
                    h.hash(&node.hash);
                }
                h.hash(&root);
                if !node.left {
                    h.hash(&node.hash);
                }
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

    /// Create a MerklePath from sibling hashes and path bits.
    ///
    /// This is useful for constructing proofs from external sources (e.g., Polygon/Ethereum)
    /// that provide traditional Merkle proofs as sibling hashes with left/right indicators.
    ///
    /// # Arguments
    /// * `sibling_hashes` - List of sibling node hashes from leaf to root
    /// * `path_bits` - List of booleans indicating position (false = left, true = right)
    ///
    /// # Example
    /// ```ignore
    /// let siblings = vec![hash1, hash2, hash3];
    /// let path = vec![false, false, true]; // left, left, right
    /// let merkle_path = MerklePath::from_sibling_hashes(siblings, path);
    /// ```
    pub fn from_sibling_hashes(sibling_hashes: Vec<H>, path_bits: Vec<bool>) -> Self {
        if sibling_hashes.len() != path_bits.len() {
            panic!(
                "Sibling hashes ({}) and path bits ({}) must have the same length",
                sibling_hashes.len(),
                path_bits.len()
            );
        }

        let nodes = sibling_hashes
            .into_iter()
            .zip(path_bits.into_iter())
            .map(|(hash, left)| MerklePathNode { hash, left })
            .collect();

        MerklePath { nodes }
    }

    /// Compress "left" field of every node in the MerklePath to a bit vector.
    fn compress(&self) -> Vec<u8> {
        // There are 3 items in the MerkleProofOperation enum, so we need 2 bits to encode them.
        let num_bytes = self.nodes.len().div_ceil(8);
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
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.is_empty() {
            let mut state = serializer.serialize_struct("MerklePath", 1)?;
            state.serialize_field("nodes_len", &(self.nodes.len() as u8))?;
            state.end()
        } else {
            let mut state = serializer.serialize_struct("MerklePath", 3)?;
            state.serialize_field("nodes_len", &(self.nodes.len() as u8))?;
            let compressed = self.compress();
            state.serialize_field("compressed", &compressed)?;
            state.serialize_field(
                "node_hashes",
                &self
                    .nodes
                    .iter()
                    .map(|node| node.hash.clone())
                    .collect::<Vec<H>>(),
            )?;
            state.end()
        }
    }
}

struct MerklePathVisitor<H: HashOutput> {
    phantom: PhantomData<H>,
}

impl<'de, H: HashOutput> Visitor<'de> for MerklePathVisitor<H> {
    type Value = MerklePath<H>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("struct MerklePath")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let count: u8 = seq
            .next_element()?
            .ok_or_else(|| A::Error::invalid_length(0, &self))?;
        if count == 0 {
            return Ok(MerklePath::empty());
        }
        let count = count as usize;
        let left_bits_size = count.div_ceil(8);
        let left_bits: Vec<u8> = seq
            .next_element()?
            .ok_or_else(|| A::Error::invalid_length(1, &self))?;
        let node_hashes: Vec<H> = seq
            .next_element()?
            .ok_or_else(|| A::Error::invalid_length(2, &self))?;
        if left_bits_size != left_bits.len() {
            return Err(A::Error::invalid_length(left_bits.len(), &self));
        }
        if node_hashes.len() != count {
            return Err(A::Error::invalid_length(node_hashes.len(), &self));
        }
        let mut nodes: Vec<MerklePathNode<H>> = Vec::with_capacity(count);
        for (i, node_hash) in node_hashes.iter().enumerate().take(count) {
            nodes.push(MerklePathNode {
                left: MerklePath::<H>::decompress(i, &left_bits),
                hash: node_hash.clone(),
            });
        }
        Ok(MerklePath { nodes })
    }
}

impl<'de, H: HashOutput> Deserialize<'de> for MerklePath<H> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        const FIELDS: &[&str] = &["nodes_len", "compressed", "node_hashes"];
        deserializer.deserialize_struct(
            "MerklePath",
            FIELDS,
            MerklePathVisitor {
                phantom: PhantomData,
            },
        )
    }
}

pub type Blake2bMerklePath = MerklePath<Blake2bHash>;

#[test]
fn it_can_correctly_deserialize_merkle_path() {
    // PoS merkle paths have the u8 count of nodes as the first byte, then the left-bits prefixed by
    // their own varint length (0b01 here), then the node hashes themselves, prefixed by another varint length
    // which is the same as the count in the first byte.
    let bin = hex::decode("02018002de8d7ee7e54f301095294d494024430c8b251b4ebf9b1384922dc7f9dd24422f830e231d26cdc3bbd1f55f1918757568522acae62c21e8046190ea84d6e8ff16")
        .unwrap();
    let _ = MerklePath::<Blake2bHash>::deserialize_all(&bin).unwrap();
}

/// This struct represents the serialization of merkle paths in the Proof-of-Work chain, which was
/// different from the serialization in the Proof-of-Stake chain (it was more efficient but harder to parse).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoWMerklePath<H: HashOutput> {
    nodes: Vec<MerklePathNode<H>>,
}

impl<H: HashOutput> PoWMerklePath<H> {
    pub fn empty() -> Self {
        PoWMerklePath { nodes: Vec::new() }
    }

    pub fn into_pos(self) -> MerklePath<H> {
        MerklePath { nodes: self.nodes }
    }
}

struct PoWMerklePathVisitor<H: HashOutput> {
    phantom: PhantomData<H>,
}

impl<'de, H: HashOutput> Visitor<'de> for PoWMerklePathVisitor<H> {
    type Value = PoWMerklePath<H>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("struct PoWMerklePath")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let count: u8 = seq
            .next_element()?
            .ok_or_else(|| A::Error::invalid_length(0, &self))?;
        if count == 0 {
            return Ok(PoWMerklePath::empty());
        }
        let count = count as usize;
        let left_bits_size = count.div_ceil(8);
        let mut left_bits = Vec::with_capacity(left_bits_size);
        for i in 0..left_bits_size {
            let bits: u8 = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(i, &self))?;
            left_bits.push(bits);
        }
        let mut node_hashes = Vec::with_capacity(count);
        for i in 0..count {
            let hash: H = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(i, &self))?;
            node_hashes.push(hash);
        }
        let mut nodes: Vec<MerklePathNode<H>> = Vec::with_capacity(count);
        for (i, node_hash) in node_hashes.iter().enumerate().take(count) {
            nodes.push(MerklePathNode {
                left: MerklePath::<H>::decompress(i, &left_bits),
                hash: node_hash.clone(),
            });
        }
        Ok(PoWMerklePath { nodes })
    }
}

impl<'de, H: HashOutput> Deserialize<'de> for PoWMerklePath<H> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // I have tried many ways to deserialize the PoW merkle path format with serde/postcard, but none
        // allowed me to read the raw bytes and then parse vecs for which I already knew the length of (so
        // that serde/postcard does not try to read a length prefix).
        // I cannot use `deserializer.deserialize_bytes()` or anything similar that deserializes vecs, because
        // they internally read the first byte as the length byte and then only let you work with that number
        // of following bytes.
        // Additionally, the number of fields passed to `deserializer.deserialize_struct()` limits the number
        // of times one can call `seq.next_element()` in the visitor before it throws a `SerdeDeCustom` error.
        // Since we don't know how many nodes we need to parse from the input without looking at the input first
        // (need to read that first byte), we cannot use it.
        // `deserializer.deserialize_tuple()` has the same limitation, but we can work around it by giving it
        // the maximum number of fields we will ever need to parse (length of a merkle path is serialized as a u8,
        // so can be at most 255. That means the highest possible number of elements is 1 length field +
        // 255.div_ceil(8) bitset bytes + 255 nodes = 288 elements). The deserializer doesn't complain when we
        // parse less elements than indicated, so we can return from the visitor at any time when we have parsed
        // all that we need to.
        // (Technically one can do the same with `deserializer.deserialize_struct()`, but one would have to pass in
        // a list of 288 strings for the `fields`. So using `deserialize_tuple()` that takes just a number is much
        // easier.)
        deserializer.deserialize_tuple(
            288,
            PoWMerklePathVisitor {
                phantom: PhantomData,
            },
        )
    }
}

pub type PoWBlake2bMerklePath = PoWMerklePath<Blake2bHash>;

#[test]
fn it_can_correctly_deserialize_pow_merkle_path() {
    // PoW merkle paths have the u8 count of nodes as the first byte, then the left-bits, then immediately the
    // node hashes themselves - no length bytes in between.
    let bin = hex::decode("0280de8d7ee7e54f301095294d494024430c8b251b4ebf9b1384922dc7f9dd24422f830e231d26cdc3bbd1f55f1918757568522acae62c21e8046190ea84d6e8ff16")
        .unwrap();
    let _ = PoWMerklePath::<Blake2bHash>::deserialize_all(&bin).unwrap();
}

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

impl<H: HashOutput> MerkleProof<H> {
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

                let mid = len.div_ceil(2);
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
        let num_bytes = self.operations.len().div_ceil(4);
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
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MerkleProof", 3)?;
        state.serialize_field("operations_len", &(self.operations.len() as u16))?;
        let compressed = self.compress();
        state.serialize_field("compressed", &compressed)?;
        state.serialize_field("nodes", &self.nodes)?;
        state.end()
    }
}

struct MerkleProofVisitor<H: HashOutput> {
    phantom: PhantomData<H>,
}

impl<'de, H: HashOutput> Visitor<'de> for MerkleProofVisitor<H> {
    type Value = MerkleProof<H>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("struct MerkleProof")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let count: u16 = seq
            .next_element()?
            .ok_or_else(|| A::Error::invalid_length(0, &self))?;
        let count = count as usize;
        let operations_size = count.div_ceil(4);
        let operation_bits: Vec<u8> = seq
            .next_element()?
            .ok_or_else(|| A::Error::invalid_length(1, &self))?;
        if operation_bits.len() != operations_size {
            return Err(A::Error::invalid_length(operations_size, &self));
        }
        let nodes: Vec<H> = seq
            .next_element()?
            .ok_or_else(|| A::Error::invalid_length(2, &self))?;
        let operations = match MerkleProof::<H>::decompress(count, operation_bits) {
            Some(operations) => operations,
            None => {
                return Err(Error::custom("invalid operation in bitvector"));
            }
        };

        Ok(MerkleProof { operations, nodes })
    }
}

impl<'de, H: HashOutput> Deserialize<'de> for MerkleProof<H> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        const FIELDS: &[&str] = &["operations_len", "compressed", "nodes"];
        deserializer.deserialize_struct(
            "MerkleProof",
            FIELDS,
            MerkleProofVisitor {
                phantom: PhantomData,
            },
        )
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

pub type Blake2bMerkleProof = MerkleProof<Blake2bHash>;

// Simple test that checks the order of operations.
#[cfg(test)]
mod tests {
    use nimiq_hash::Blake2bHasher;
    use nimiq_test_log::test;

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
        let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
        let proof: MerkleProof<Blake2bHash> =
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
        let root = compute_root_from_content::<Blake2bHasher, &str>(&values);
        let proof: MerkleProof<Blake2bHash> =
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

        let proof: MerkleProof<Blake2bHash> = MerkleProof::from_values::<&str>(&values, &[]);
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
