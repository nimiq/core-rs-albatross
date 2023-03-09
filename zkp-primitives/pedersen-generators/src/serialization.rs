use ark_crypto_primitives::crh::pedersen::Parameters;
use ark_ec::CurveGroup;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};

use beserial::{Deserialize, Serialize};

use crate::generators::PedersenParameters;

impl<C: CurveGroup> Serialize for PedersenParameters<C> {
    fn serialize<W: beserial::WriteBytesExt>(
        &self,
        mut writer: &mut W,
    ) -> Result<usize, beserial::SerializingError> {
        let mut size = CanonicalSerialize::uncompressed_size(&self.blinding_factor);
        CanonicalSerialize::serialize_uncompressed(&self.blinding_factor, &mut writer)
            .or(Err(beserial::SerializingError::InvalidValue))?;

        size += CanonicalSerialize::uncompressed_size(&self.parameters.generators);
        CanonicalSerialize::serialize_uncompressed(&self.parameters.generators, &mut writer)
            .or(Err(beserial::SerializingError::InvalidValue))?;

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        CanonicalSerialize::uncompressed_size(&self.blinding_factor)
            + CanonicalSerialize::uncompressed_size(&self.parameters.generators)
    }
}

impl<C: CurveGroup> Deserialize for PedersenParameters<C> {
    fn deserialize<R: beserial::ReadBytesExt>(
        mut reader: &mut R,
    ) -> Result<Self, beserial::SerializingError> {
        let blinding_factor: C =
            CanonicalDeserialize::deserialize_uncompressed_unchecked(&mut reader)
                .or(Err(beserial::SerializingError::InvalidValue))?;

        let generators: Vec<Vec<C>> =
            CanonicalDeserialize::deserialize_uncompressed_unchecked(&mut reader)
                .or(Err(beserial::SerializingError::InvalidValue))?;

        Ok(PedersenParameters {
            parameters: Parameters { generators },
            blinding_factor,
        })
    }
}
