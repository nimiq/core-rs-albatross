use bitvec::{BitVec, Cursor};

use crate::{SerializeWithLength, DeserializeWithLength, Serialize, Deserialize, WriteBytesExt, ReadBytesExt, SerializingError};


impl<C: Cursor> SerializeWithLength for BitVec<C, u8> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        // NOTE: serialize the number of bits seperately from the slice
        size += S::from_usize(self.len()).unwrap().serialize(writer)?;
        let slice = self.as_slice();
        size += S::from_usize(slice.len()).unwrap().serialize(writer)?;
        writer.write_all(slice)?;
        size += slice.len();
        Ok(size)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        let mut size = 0;
        size += S::from_usize(self.len()).unwrap().serialized_size();
        let slice = self.as_slice();
        size += S::from_usize(slice.len()).unwrap().serialized_size();
        size += slice.len();
        size
    }
}

impl<C: Cursor> DeserializeWithLength for BitVec<C, u8> {
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(reader: &mut R, limit: Option<usize>) -> Result<Self, SerializingError> {
        let n_bits = D::deserialize(reader)?
            .to_usize().ok_or(SerializingError::Overflow)?;
        let n = D::deserialize(reader)?
            .to_usize().ok_or(SerializingError::Overflow)?;

        // If number of bits is too large, abort.
        if limit.map(|l| l > n_bits).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        let mut data = Vec::with_capacity(n);
        data.resize(n, 0);
        reader.read_exact(data.as_mut_slice())?;
        let mut bitvec: BitVec<C, u8> = BitVec::from(data);
        bitvec.resize(n_bits, false);
        Ok(bitvec)
    }
}
