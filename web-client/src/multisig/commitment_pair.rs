use js_sys::Array;
use nimiq_keys::SecureGenerate;
use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

use super::{commitment::Commitment, random_secret::RandomSecret};

/// A structure holding both a random secret and its corresponding public commitment.
/// This is similar to a `KeyPair`.
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct CommitmentPair {
    inner: nimiq_keys::multisig::commitment::CommitmentPair,
}

#[wasm_bindgen]
impl CommitmentPair {
    #[wasm_bindgen(getter = SIZE)]
    pub fn size() -> usize {
        nimiq_keys::multisig::commitment::Nonce::SIZE
            + nimiq_keys::multisig::commitment::Commitment::SIZE
    }

    #[wasm_bindgen(getter = serializedSize)]
    pub fn serialized_size(&self) -> usize {
        CommitmentPair::size()
    }

    /// Parses a commitment pair from a {@link CommitmentPair} instance, a hex string representation, or a byte array.
    ///
    /// Throws when a CommitmentPair cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(pair: &CommitmentPairAnyType) -> Result<CommitmentPair, JsError> {
        let js_value: &JsValue = pair.unchecked_ref();

        if let Ok(pair) = CommitmentPair::try_from(js_value) {
            return Ok(pair);
        }

        if let Ok(string) = serde_wasm_bindgen::from_value::<String>(js_value.to_owned()) {
            Ok(CommitmentPair::from_hex(&string)?)
        } else if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(js_value.to_owned()) {
            Ok(CommitmentPair::deserialize(&bytes)?)
        } else {
            Err(JsError::new("Could not parse commitment pair"))
        }
    }

    /// Deserializes a commitment pair from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<CommitmentPair, JsError> {
        let pair = nimiq_keys::multisig::commitment::CommitmentPair::deserialize_from_vec(bytes)?;
        Ok(CommitmentPair::from(pair))
    }

    pub fn generate() -> CommitmentPair {
        CommitmentPair::from(
            nimiq_keys::multisig::commitment::CommitmentPair::generate_default_csprng(),
        )
    }

    /// Derives a commitment pair from an existing random secret.
    pub fn derive(random_secret: &RandomSecret) -> CommitmentPair {
        let nonce = *random_secret.native_ref();
        let commitment = nonce.commit();
        let commitment_pair =
            nimiq_keys::multisig::commitment::CommitmentPair::new(nonce, commitment);
        CommitmentPair::from(commitment_pair)
    }

    #[wasm_bindgen(constructor)]
    pub fn new(random_secret: &RandomSecret, commitment: &Commitment) -> CommitmentPair {
        let commitment_pair = nimiq_keys::multisig::commitment::CommitmentPair::new(
            *random_secret.native_ref(),
            *commitment.native_ref(),
        );
        CommitmentPair::from(commitment_pair)
    }

    /// Serializes the commitment pair to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Parses a commitment pair from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 32 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<CommitmentPair, JsError> {
        let bytes = hex::decode(hex)?;
        Self::deserialize(&bytes)
    }

    /// Formats the commitment pair into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }

    #[wasm_bindgen(getter)]
    pub fn secret(&self) -> RandomSecret {
        RandomSecret::from(self.inner.nonce())
    }

    #[wasm_bindgen(getter)]
    pub fn commitment(&self) -> Commitment {
        Commitment::from(self.inner.commitment())
    }

    /// Returns if this commitment pair is equal to the other commitment pair.
    pub fn equals(&self, other: &CommitmentPair) -> bool {
        self.inner == other.inner
    }
}

impl From<nimiq_keys::multisig::commitment::CommitmentPair> for CommitmentPair {
    fn from(pair: nimiq_keys::multisig::commitment::CommitmentPair) -> CommitmentPair {
        CommitmentPair { inner: pair }
    }
}

impl CommitmentPair {
    pub fn native_ref(&self) -> &nimiq_keys::multisig::commitment::CommitmentPair {
        &self.inner
    }

    pub fn take_native(self) -> nimiq_keys::multisig::commitment::CommitmentPair {
        self.inner
    }

    pub fn unpack_commitment_pairs(
        pairs: &CommitmentPairAnyArrayType,
    ) -> Result<Vec<nimiq_keys::multisig::commitment::CommitmentPair>, JsError> {
        // Unpack the array of pairs
        let js_value: &JsValue = pairs.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`pairs` must be an array"))?;

        let mut pairs = Vec::<_>::with_capacity(array.length().try_into()?);
        for any in array.iter() {
            let pair = CommitmentPair::from_any(&any.into())?.take_native();
            pairs.push(pair);
        }

        Ok(pairs)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "CommitmentPair | string | Uint8Array")]
    pub type CommitmentPairAnyType;

    #[wasm_bindgen(typescript_type = "(CommitmentPair | string | Uint8Array)[]")]
    pub type CommitmentPairAnyArrayType;
}
