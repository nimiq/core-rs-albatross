use js_sys::Uint8Array;
use nimiq_hash::Blake2bHash;
use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct MerklePath {
    inner: nimiq_utils::merkle::Blake2bMerklePath,
}

#[wasm_bindgen]
impl MerklePath {
    #[wasm_bindgen(getter)]
    pub fn hashes(&self) -> Vec<Uint8Array> {
        self.inner
            .hashes()
            .iter()
            .map(|hash| hash.serialize_to_vec().as_slice().into())
            .collect()
    }

    #[wasm_bindgen(getter)]
    pub fn length(&self) -> usize {
        self.inner.len()
    }

    #[wasm_bindgen(js_name = computeRoot)]
    pub fn compute_root(&self, leaf: &[u8]) -> Result<Vec<u8>, JsError> {
        match Blake2bHash::deserialize_from_vec(leaf) {
            Ok(leaf_hash) => Ok(self.inner.compute_root(&leaf_hash).serialize_to_vec()),
            Err(_) => Err(JsError::new(
                "Failed to deserialize leaf: not a Blake2b hash",
            )),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    pub fn deserialize(data: &[u8]) -> Result<MerklePath, JsError> {
        match nimiq_utils::merkle::Blake2bMerklePath::deserialize_from_vec(data) {
            Ok(path) => Ok(path.into()),
            Err(e) => Err(JsError::new(&format!(
                "Failed to deserialize MerklePath: {}",
                e
            ))),
        }
    }
}

impl From<nimiq_utils::merkle::Blake2bMerklePath> for MerklePath {
    fn from(path: nimiq_utils::merkle::Blake2bMerklePath) -> Self {
        MerklePath { inner: path }
    }
}
