use bitvec::{
    field::BitField,
    prelude::{BitSlice, BitVec, Msb0},
    view::BitView,
};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};

#[inline]
// NOTE: We can't use `nimiq_utils::math::CeilingDiv`, because it'll create a cyclic dependency.
fn bits_to_bytes(lhs: usize) -> usize {
    (lhs + 8 - 1) / 8
}

impl SerializeWithLength for BitSlice<u8, Msb0> {
    fn serialize<S: Serialize + FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = 0;

        // Serialize the number of bits
        size += S::from_usize(self.len()).unwrap().serialize(writer)?;

        let target_len = bits_to_bytes(self.len());

        // Serialize the BitVec as byte slice
        let chunks = self.chunks_exact(8);
        let remainder = chunks.remainder();
        let base_slice: Vec<u8> = chunks.map(|chunk| chunk.load_be::<u8>()).collect();

        writer.write_all(&base_slice)?;

        // If there is a reminder, append it
        if !remainder.is_empty() {
            let mut byte = 0u8;
            let bits = &mut byte.view_bits_mut::<Msb0>()[..remainder.len()];
            bits.copy_from_bitslice(remainder);
            writer.write_all(&[byte])?;
        }
        size += target_len;

        Ok(size)
    }

    fn serialized_size<S: Serialize + FromPrimitive>(&self) -> usize {
        let mut size = 0;

        // Size of number of bits
        size += S::from_usize(self.len()).unwrap().serialized_size();

        // Number of bytes needed to represent this BitVec
        size += bits_to_bytes(self.len());

        size
    }
}

impl SerializeWithLength for BitVec<u8, Msb0> {
    fn serialize<S: Serialize + FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        self.as_bitslice().serialize::<S, W>(writer)
    }

    fn serialized_size<S: Serialize + FromPrimitive>(&self) -> usize {
        self.as_bitslice().serialized_size::<S>()
    }
}

impl DeserializeWithLength for BitVec<u8, Msb0> {
    fn deserialize_with_limit<D: Deserialize + ToPrimitive, R: ReadBytesExt>(
        reader: &mut R,
        limit: Option<usize>,
    ) -> Result<Self, SerializingError> {
        // Deserialize the number if bits in the BitVec
        let n_bits = D::deserialize(reader)?
            .to_usize()
            .ok_or(SerializingError::Overflow)?;
        let n_bytes = bits_to_bytes(n_bits);

        // If number of bits is too large, abort.
        if limit.map(|l| l > n_bits).unwrap_or(false) {
            return Err(SerializingError::LimitExceeded);
        }

        // Read bytes into Vec
        let mut data = vec![0; n_bytes];
        reader.read_exact(data.as_mut_slice())?;

        let mut bitvec: BitVec<u8, Msb0> = BitVec::from_vec(data);
        bitvec.resize(n_bits, false);
        Ok(bitvec)
    }
}

#[cfg(test)]
mod tests {
    use bitvec::prelude::Msb0;

    use super::*;

    fn reserialize(bits: &BitSlice<u8, Msb0>, raw: &[u8]) {
        let serialized = bits.serialize_to_vec::<u8>();
        assert_eq!(&serialized[..], raw);
        let deserialized_bits =
            BitVec::<u8, Msb0>::deserialize_from_vec::<u8>(&serialized).unwrap();
        assert_eq!(deserialized_bits, bits);
    }

    #[test]
    fn it_correctly_serializes_and_deserializes_empty_bitvec() {
        let bits = BitVec::new();
        reserialize(&bits, &b"\x00"[..]);
    }

    #[test]
    fn it_correctly_serializes_and_deserializes_unaligned_bitslice() {
        let base: Vec<u8> = vec![0x42, 0x04, 0x20, 0x43];
        // Base bytes:   b01000010 x04 x20 b01000011
        // Sliced 4..31: b####0010 x04 x20 b0100001#####
        // Packed 4..31:        x20 x42 x04   x20
        let base_bits = base.view_bits::<Msb0>();
        let bits = &base_bits[4..31];
        reserialize(bits, &b"\x1b\x20\x42\x04\x20"[..]);
    }

    #[test]
    fn it_correctly_serializes_and_deserializes_bitvec() {
        let base: Vec<u8> = vec![0x42, 0x04, 0x20, 0x43];
        let bits = BitVec::<u8, Msb0>::from_slice(&base);
        reserialize(&bits, &b"\x20\x42\x04\x20\x43"[..]);
    }
}
