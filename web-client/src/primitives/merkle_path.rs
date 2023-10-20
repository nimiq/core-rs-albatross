use js_sys::Uint8Array;
use nimiq_serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

/// A Merkle path is a list of hashes that allows to prove the membership of an element in a set.
#[wasm_bindgen]
pub struct MerklePath {
    inner: nimiq_utils::merkle::Blake2bMerklePath,
}

#[wasm_bindgen]
impl MerklePath {
    /// Computes a Merkle path for one of the values in a list of Uint8Arrays.
    pub fn compute(values: Vec<Uint8Array>, leaf_value: &Uint8Array) -> MerklePath {
        let mut values: Vec<_> = values.iter().map(|u| u.to_vec()).collect();
        values.sort_unstable();

        Self {
            inner: nimiq_utils::merkle::Blake2bMerklePath::new::<nimiq_hash::Blake2bHasher, Vec<u8>>(
                &values,
                &leaf_value.to_vec(),
            ),
        }
    }

    /// Computes the root of the Merkle tree from the leaf value for which the Merkle path was constructed.
    #[wasm_bindgen(js_name = computeRoot)]
    pub fn compute_root(&self, leaf_value: &Uint8Array) -> Vec<u8> {
        self.inner
            .compute_root(&leaf_value.to_vec())
            .serialize_to_vec()
    }

    /// The number of nodes (steps) of the Merkle path.
    #[wasm_bindgen(getter)]
    pub fn length(&self) -> usize {
        self.inner.len()
    }
}

impl MerklePath {
    pub fn from_native(merkle_path: nimiq_utils::merkle::Blake2bMerklePath) -> MerklePath {
        MerklePath { inner: merkle_path }
    }

    pub fn native_ref(&self) -> &nimiq_utils::merkle::Blake2bMerklePath {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use js_sys::Uint8Array;
    use wasm_bindgen_test::wasm_bindgen_test;

    use crate::primitives::{merkle_path::MerklePath, merkle_tree::MerkleTree};

    #[wasm_bindgen_test]
    pub fn it_can_compute_a_path_and_its_root() {
        let value1 = Uint8Array::new_with_length(8);
        value1.copy_from(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let value2 = Uint8Array::new_with_length(8);
        value2.copy_from(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let value3 = Uint8Array::new_with_length(8);
        value3.copy_from(&[17, 18, 19, 20, 21, 22, 23, 24]);
        let value4 = Uint8Array::new_with_length(8);
        value4.copy_from(&[25, 26, 27, 28, 29, 30, 31, 32]);

        // Create a merkle path for value3
        let path = MerklePath::compute(
            vec![
                value1.clone(),
                value2.clone(),
                value3.clone(),
                value4.clone(),
            ],
            &value3,
        );
        assert_eq!(path.length(), 2);

        // Compute the merkle root from value3
        let path_root = path.compute_root(&value3);

        // Compute the reference root directly with MerkleTree
        let root = MerkleTree::compute_root(vec![
            value1.clone(),
            value2.clone(),
            value3.clone(),
            value4.clone(),
        ]);
        assert_eq!(root, path_root);

        // Test that it computes a different root for a different start value
        let wrong_root = path.compute_root(&value1);
        assert_ne!(root, wrong_root);
    }
}
