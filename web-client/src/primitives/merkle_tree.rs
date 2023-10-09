use js_sys::Uint8Array;
use nimiq_serde::Serialize;
use nimiq_utils::merkle::compute_root_from_content;
use wasm_bindgen::prelude::wasm_bindgen;

/// The Merkle tree is a data structure that allows for efficient verification of the membership of an element in a set.
#[wasm_bindgen]
pub struct MerkleTree;

#[wasm_bindgen]
impl MerkleTree {
    /// Computes the root of a Merkle tree from a list of Uint8Arrays.
    #[wasm_bindgen(js_name = computeRoot)]
    pub fn compute_root(values: Vec<Uint8Array>) -> Vec<u8> {
        let mut values: Vec<_> = values.iter().map(|u| u.to_vec()).collect();
        values.sort_unstable();

        let root = compute_root_from_content::<nimiq_hash::Blake2bHasher, Vec<u8>>(&values);
        root.serialize_to_vec()
    }
}

#[cfg(test)]
mod tests {
    use js_sys::Uint8Array;
    use wasm_bindgen_test::wasm_bindgen_test;

    use crate::primitives::merkle_tree::MerkleTree;

    #[wasm_bindgen_test]
    pub fn it_computes_the_same_root_for_any_array_order() {
        let value1 = Uint8Array::new_with_length(8);
        value1.copy_from(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let value2 = Uint8Array::new_with_length(8);
        value2.copy_from(&[9, 10, 11, 12, 13, 14, 15, 16]);

        let root_ordered = MerkleTree::compute_root(vec![value1.clone(), value2.clone()]);
        let root_unordered = MerkleTree::compute_root(vec![value2.clone(), value1.clone()]);

        assert_eq!(root_ordered, root_unordered);
    }
}
