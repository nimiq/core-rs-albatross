use std::iter::Sum;

use js_sys::Array;
use nimiq_keys::multisig::{CommitmentsBuilder, MUSIG2_PARAMETER_V};
use nimiq_serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

use super::random_secret::RandomSecret;
use crate::primitives::public_key::{PublicKey, PublicKeyAnyArrayType};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
enum SumMuSig2Error {
    #[error("The number of public keys must match the number of commitment groups")]
    LengthMismatch,
    #[error("At least one public key must be provided")]
    EmptyPublicKeys,
    #[error("Number of commitments in each group must be 2")]
    InvalidCommitmentGroupSize,
}

/// A cryptographic commitment to a {@link RandomSecret}. The commitment is public, while the secret is, well, secret.
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct Commitment {
    inner: nimiq_keys::multisig::commitment::Commitment,
}

#[wasm_bindgen]
impl Commitment {
    #[wasm_bindgen(getter = SIZE)]
    pub fn size() -> usize {
        nimiq_keys::multisig::commitment::Commitment::SIZE
    }

    #[wasm_bindgen(getter = serializedSize)]
    pub fn serialized_size(&self) -> usize {
        Commitment::size()
    }

    /// Derives a commitment from an existing random secret.
    pub fn derive(random_secret: &RandomSecret) -> Commitment {
        let nonce = *random_secret.native_ref();
        let commitment = nonce.commit();
        Commitment::from(commitment)
    }

    /// Sums up multiple commitments into one aggregated commitment.
    ///
    /// Attention: This is a simple summation, not a MuSig2 aggregation! For MuSig2 aggregation, use {@link Commitment.sumMuSig2}.
    pub fn sum(commitments: &CommitmentAnyArrayType) -> Result<Commitment, JsError> {
        let commitments = Commitment::unpack_commitments(commitments)?;
        if commitments.is_empty() {
            return Err(JsError::new("No commitments provided"));
        }

        let aggregate_commitment =
            nimiq_keys::multisig::commitment::Commitment::sum(commitments.into_iter());
        Ok(Commitment::from(aggregate_commitment))
    }

    /// Aggregates commitments into one aggregated commitment using the MuSig2 scheme.
    ///
    /// - Each commitment group must correspond to the public key at the same index in the `publicKeys` array.
    /// - The number of commitment groups and public keys must be the same.
    /// - Each commitment group must contain exactly `MUSIG2_PARAMETER_V = 2` commitments.
    /// - The `data` parameter is the same data that will be signed using the aggregated commitment, e.g. the serialized content of a transaction.
    ///
    /// Returns the aggregated commitment.
    #[wasm_bindgen(js_name = sumMuSig2)]
    pub fn sum_musig2(
        public_keys: &PublicKeyAnyArrayType,
        commitment_groups: &CommitmentAnyArrayArrayType,
        data: &[u8],
    ) -> Result<Commitment, JsError> {
        let public_keys = PublicKey::unpack_public_keys(public_keys)?;
        let commitment_groups = Commitment::unpack_commitments_list(commitment_groups)?;
        let commitment_pairs = Self::pack_commitment_pairs(&public_keys, &commitment_groups)
            .map_err(|error| JsError::from(&error))?;

        let mut builder =
            CommitmentsBuilder::with_public_commitments(public_keys[0], commitment_pairs[0]);

        public_keys[1..]
            .iter()
            .zip(commitment_pairs[1..].iter())
            .for_each(|(&public_key, &commitments)| {
                builder.push_signer(public_key, commitments);
            });

        let commitment_data = builder.build(data)?;

        Ok(Commitment::from(commitment_data.aggregate_commitment))
    }

    /// Parses a commitment from a {@link Commitment} instance, a hex string representation, or a byte array.
    ///
    /// Throws when a Commitment cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(commitment: &CommitmentAnyType) -> Result<Commitment, JsError> {
        let js_value: &JsValue = commitment.unchecked_ref();

        if let Ok(commitment) = Commitment::try_from(js_value) {
            return Ok(commitment);
        }

        if let Ok(string) = serde_wasm_bindgen::from_value::<String>(js_value.to_owned()) {
            Ok(Commitment::from_hex(&string)?)
        } else if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(js_value.to_owned()) {
            Ok(Commitment::deserialize(&bytes)?)
        } else {
            Err(JsError::new("Could not parse commitment"))
        }
    }

    /// Deserializes a commitment from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<Commitment, JsError> {
        let commitment = nimiq_keys::multisig::commitment::Commitment::deserialize_from_vec(bytes)?;
        Ok(Commitment::from(commitment))
    }

    /// Creates a new commitment from a byte array.
    ///
    /// Throws when the byte array is not exactly 32 bytes long.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<Commitment, JsError> {
        Self::deserialize(bytes)
    }

    /// Serializes the commitment to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.serialize_to_vec()
    }

    /// Parses a commitment from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 32 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<Commitment, JsError> {
        let bytes = hex::decode(hex)?;
        Self::deserialize(&bytes)
    }

    /// Formats the commitment into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }

    /// Returns if this commitment is equal to the other commitment.
    pub fn equals(&self, other: &Commitment) -> bool {
        self.inner == other.inner
    }
}

impl From<nimiq_keys::multisig::commitment::Commitment> for Commitment {
    fn from(commitment: nimiq_keys::multisig::commitment::Commitment) -> Commitment {
        Commitment { inner: commitment }
    }
}

impl Commitment {
    pub fn native_ref(&self) -> &nimiq_keys::multisig::commitment::Commitment {
        &self.inner
    }

    pub fn take_native(self) -> nimiq_keys::multisig::commitment::Commitment {
        self.inner
    }

    pub fn unpack_commitments(
        commitments: &CommitmentAnyArrayType,
    ) -> Result<Vec<nimiq_keys::multisig::commitment::Commitment>, JsError> {
        // Unpack the array of commitments
        let js_value: &JsValue = commitments.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`commitments` must be an array"))?;

        let mut commitments = Vec::<_>::with_capacity(array.length().try_into()?);
        for any in array.iter() {
            let commitment = Commitment::from_any(&any.into())?.take_native();
            commitments.push(commitment);
        }

        Ok(commitments)
    }

    pub fn unpack_commitments_list(
        commitments_list: &CommitmentAnyArrayArrayType,
    ) -> Result<Vec<Vec<nimiq_keys::multisig::commitment::Commitment>>, JsError> {
        // Unpack the outer array
        let js_value: &JsValue = commitments_list.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`commitments_list` must be an array"))?;

        let mut commitments_list = Vec::<_>::with_capacity(array.length().try_into()?);
        for any in array.iter() {
            let commitments = Commitment::unpack_commitments(&any.into())?;
            commitments_list.push(commitments);
        }

        Ok(commitments_list)
    }

    fn pack_commitment_pairs(
        public_keys: &[nimiq_keys::Ed25519PublicKey],
        commitment_groups: &[Vec<nimiq_keys::multisig::commitment::Commitment>],
    ) -> Result<
        Vec<[nimiq_keys::multisig::commitment::Commitment; MUSIG2_PARAMETER_V]>,
        SumMuSig2Error,
    > {
        if public_keys.len() != commitment_groups.len() {
            return Err(SumMuSig2Error::LengthMismatch);
        }
        if public_keys.is_empty() {
            return Err(SumMuSig2Error::EmptyPublicKeys);
        }
        if commitment_groups
            .iter()
            .any(|commitments| commitments.len() != MUSIG2_PARAMETER_V)
        {
            return Err(SumMuSig2Error::InvalidCommitmentGroupSize);
        }

        let mut commitment_pairs = Vec::with_capacity(commitment_groups.len());
        for group in commitment_groups {
            let mut commitment_pair =
                [nimiq_keys::multisig::commitment::Commitment::default(); MUSIG2_PARAMETER_V];
            commitment_pair.copy_from_slice(group);
            commitment_pairs.push(commitment_pair);
        }

        Ok(commitment_pairs)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Commitment | string | Uint8Array")]
    pub type CommitmentAnyType;

    #[wasm_bindgen(typescript_type = "(Commitment | string | Uint8Array)[]")]
    pub type CommitmentAnyArrayType;

    #[wasm_bindgen(typescript_type = "(Commitment | string | Uint8Array)[][]")]
    pub type CommitmentAnyArrayArrayType;
}

#[cfg(test)]
mod tests {
    use nimiq_keys::{KeyPair, SecureGenerate};

    use super::{Commitment, SumMuSig2Error};

    #[test]
    fn it_rejects_empty_musig2_inputs() {
        assert_eq!(
            Commitment::pack_commitment_pairs(&[], &[]),
            Err(SumMuSig2Error::EmptyPublicKeys)
        );
    }

    #[test]
    fn it_rejects_malformed_musig2_commitment_groups() {
        let signer = KeyPair::generate_default_csprng();
        assert_eq!(
            Commitment::pack_commitment_pairs(
                &[signer.public],
                &[vec![nimiq_keys::multisig::commitment::Commitment::default()]],
            ),
            Err(SumMuSig2Error::InvalidCommitmentGroupSize)
        )
    }

    #[test]
    fn it_rejects_mismatched_signer_and_commitment_group_counts() {
        let signer = KeyPair::generate_default_csprng();
        assert_eq!(
            Commitment::pack_commitment_pairs(&[signer.public], &[]),
            Err(SumMuSig2Error::LengthMismatch)
        );
    }
}
