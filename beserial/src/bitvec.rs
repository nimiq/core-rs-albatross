use bitvec::prelude::{BitVec, BitOrder};

use crate::{SerializeWithLength, DeserializeWithLength, Serialize, Deserialize, WriteBytesExt, ReadBytesExt, SerializingError};


#[inline]
// NOTE: We can't use `nimiq_utils::math::CeilingDiv`, because it'll create a cyclic dependency.
fn bits_to_bytes(lhs: usize) -> usize {
    (lhs + 8 - 1) / 8
}


impl<C: BitOrder> SerializeWithLength for BitVec<C, u8> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;

        // Serialize the number of bits
        size += S::from_usize(self.len()).unwrap().serialize(writer)?;

        // Serialize the BitVec as byte slice
        let slice = self.as_slice();
        writer.write_all(slice)?;
        assert_eq!(slice.len(), bits_to_bytes(self.len()));
        size += slice.len();

        Ok(size)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = 0;

        // Size of number of bits
        size += S::from_usize(self.len()).unwrap().serialized_size();

        // Number of bytes needed to represent this BitVec
        size += bits_to_bytes(self.len());

        size
    }
}

impl<C: BitOrder> DeserializeWithLength for BitVec<C, u8> {
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError> {
        // Deserialize the number of bits in the BitVec
        let n_bits = D::deserialize(reader)?
            .to_usize().ok_or(SerializingError::Overflow)?;
        let n_bytes = bits_to_bytes(n_bits);

        // If number of bits is too large, abort.
        if limit.map(|l| l > n_bits).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        // Read bytes into Vec
        let mut data = Vec::with_capacity(n_bytes);
        data.resize(n_bytes, 0);
        reader.read_exact(data.as_mut_slice())?;

        let mut bitvec: BitVec<C, u8> = BitVec::from_vec(data);
        bitvec.resize(n_bits, false);
        Ok(bitvec)
    }
}
