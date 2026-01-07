use js_sys::Array;
use nimiq_keys::multisig::{CommitmentsBuilder, MUSIG2_PARAMETER_V};
use nimiq_serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_derive::TryFromJsValue;

use super::{
    commitment::{Commitment, CommitmentAnyArrayArrayType},
    commitment_pair::{CommitmentPair, CommitmentPairAnyArrayType},
};
use crate::primitives::{
    private_key::PrivateKey,
    public_key::{PublicKey, PublicKeyAnyArrayType},
    signature::Signature,
};

/// A partial signature is a signature of one of the co-signers in a multisig.
/// Combining all partial signatures yields the full signature (combining is done through summation).
#[derive(TryFromJsValue)]
#[wasm_bindgen]
#[derive(Clone)]
pub struct PartialSignature {
    inner: nimiq_keys::multisig::partial_signature::PartialSignature,
}

#[wasm_bindgen]
impl PartialSignature {
    #[wasm_bindgen(getter = SIZE)]
    pub fn size() -> usize {
        nimiq_keys::multisig::partial_signature::PartialSignature::SIZE
    }

    #[wasm_bindgen(getter = serializedSize)]
    pub fn serialized_size(&self) -> usize {
        PartialSignature::size()
    }

    /// Parses a partial signature from a {@link PartialSignature} instance, a hex string representation, or a byte array.
    ///
    /// Throws when a PartialSignature cannot be parsed from the argument.
    #[wasm_bindgen(js_name = fromAny)]
    pub fn from_any(sig: &PartialSignatureAnyType) -> Result<PartialSignature, JsError> {
        let js_value: &JsValue = sig.unchecked_ref();

        if let Ok(sig) = PartialSignature::try_from(js_value) {
            return Ok(sig);
        }

        if let Ok(string) = serde_wasm_bindgen::from_value::<String>(js_value.to_owned()) {
            Ok(PartialSignature::from_hex(&string)?)
        } else if let Ok(bytes) = serde_wasm_bindgen::from_value::<Vec<u8>>(js_value.to_owned()) {
            Ok(PartialSignature::deserialize(&bytes)?)
        } else {
            Err(JsError::new("Could not parse partial signature"))
        }
    }

    /// Deserializes a partial signature from a byte array.
    ///
    /// Throws when the byte array contains less than 32 bytes.
    pub fn deserialize(bytes: &[u8]) -> Result<PartialSignature, JsError> {
        let buf: [u8; nimiq_keys::multisig::partial_signature::PartialSignature::SIZE] =
            nimiq_serde::FixedSizeByteArray::deserialize_from_vec(bytes)?.into_inner();
        let signature = nimiq_keys::multisig::partial_signature::PartialSignature::from(buf);
        Ok(PartialSignature::from(signature))
    }

    /// Creates a new partial signature from a byte array.
    ///
    /// Throws when the byte array is not exactly 32 bytes long.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<PartialSignature, JsError> {
        Self::deserialize(bytes)
    }

    pub fn combine(
        partial_signatures: PartialSignatureAnyArrayType,
    ) -> Result<PartialSignature, JsError> {
        let sigs = PartialSignature::unpack_partial_signatures(&partial_signatures)?;
        let aggregated_signature =
            sigs.iter()
                .sum::<nimiq_keys::multisig::partial_signature::PartialSignature>();
        Ok(PartialSignature::from(aggregated_signature))
    }

    #[wasm_bindgen(js_name = toSignature)]
    pub fn to_signature(&self, aggregated_commitment: Commitment) -> Signature {
        let sig = self.inner.to_signature(aggregated_commitment.native_ref());
        sig.into()
    }

    pub fn create(
        own_private_key: &PrivateKey,
        own_public_key: &PublicKey,
        own_commitment_pairs: &CommitmentPairAnyArrayType,
        other_public_keys: &PublicKeyAnyArrayType,
        other_commitments: &CommitmentAnyArrayArrayType,
        data: &[u8],
    ) -> Result<PartialSignature, JsError> {
        let own_private_key = own_private_key.native_ref();
        let own_public_key = own_public_key.native_ref();
        let own_commitment_pairs = CommitmentPair::unpack_commitment_pairs(own_commitment_pairs)?;
        let other_public_keys = PublicKey::unpack_public_keys(other_public_keys)?;
        let other_commitments = Commitment::unpack_commitments_list(other_commitments)?;

        // Sanity checks
        if own_commitment_pairs.len() != MUSIG2_PARAMETER_V {
            return Err(JsError::new(&format!(
                "Number of own commitment pairs must be {MUSIG2_PARAMETER_V}"
            )));
        }
        if other_public_keys.len() != other_commitments.len() {
            return Err(JsError::new(
                "Number of other public keys and other commitment groups must match",
            ));
        }
        if other_commitments
            .iter()
            .any(|commitments| commitments.len() != MUSIG2_PARAMETER_V)
        {
            return Err(JsError::new(&format!(
                "Number of commitments in each group must be {MUSIG2_PARAMETER_V}"
            )));
        }

        let own_keypair = nimiq_keys::KeyPair {
            private: own_private_key.clone(),
            public: *own_public_key,
        };

        let mut commitment_pairs =
            [nimiq_keys::multisig::commitment::CommitmentPair::default(); MUSIG2_PARAMETER_V];
        commitment_pairs.copy_from_slice(&own_commitment_pairs);
        let mut commitments_data =
            CommitmentsBuilder::with_private_commitments(*own_public_key, commitment_pairs);

        for i in 0..other_public_keys.len() {
            let mut commitments =
                [nimiq_keys::multisig::commitment::Commitment::default(); MUSIG2_PARAMETER_V];
            commitments.copy_from_slice(&other_commitments[i]);
            commitments_data.push_signer(other_public_keys[i], commitments);
        }

        let signature = own_keypair.partial_sign(&commitments_data.build(data), data)?;

        Ok(PartialSignature::from(signature))
    }

    /// Serializes the partial signature to a byte array.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }

    /// Parses a partial signature from its hex representation.
    ///
    /// Throws when the string is not valid hex format or when it represents less than 32 bytes.
    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<PartialSignature, JsError> {
        let bytes = hex::decode(hex)?;
        Self::deserialize(&bytes)
    }

    /// Formats the partial signature into a hex string.
    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }

    /// Returns if this partial signature is equal to the other partial signature.
    pub fn equals(&self, other: &PartialSignature) -> bool {
        self.inner == other.inner
    }
}

impl From<nimiq_keys::multisig::partial_signature::PartialSignature> for PartialSignature {
    fn from(secret: nimiq_keys::multisig::partial_signature::PartialSignature) -> PartialSignature {
        PartialSignature { inner: secret }
    }
}

impl PartialSignature {
    pub fn native_ref(&self) -> &nimiq_keys::multisig::partial_signature::PartialSignature {
        &self.inner
    }

    pub fn take_native(self) -> nimiq_keys::multisig::partial_signature::PartialSignature {
        self.inner
    }

    pub fn unpack_partial_signatures(
        sigs: &PartialSignatureAnyArrayType,
    ) -> Result<Vec<nimiq_keys::multisig::partial_signature::PartialSignature>, JsError> {
        // Unpack the array of sigs
        let js_value: &JsValue = sigs.unchecked_ref();
        let array: &Array = js_value
            .dyn_ref()
            .ok_or_else(|| JsError::new("`sigs` must be an array"))?;

        let mut sigs = Vec::<_>::with_capacity(array.length().try_into()?);
        for any in array.iter() {
            let sig = PartialSignature::from_any(&any.into())?.take_native();
            sigs.push(sig);
        }

        Ok(sigs)
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PartialSignature | string | Uint8Array")]
    pub type PartialSignatureAnyType;

    #[wasm_bindgen(typescript_type = "(PartialSignature | string | Uint8Array)[]")]
    pub type PartialSignatureAnyArrayType;
}
