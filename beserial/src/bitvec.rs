use bitvec::prelude::{AsBits, BitSlice, BitVec, Msb0};

use crate::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};

#[inline]
// NOTE: We can't use `nimiq_utils::math::CeilingDiv`, because it'll create a cyclic dependency.
fn bits_to_bytes(lhs: usize) -> usize {
    (lhs + 8 - 1) / 8
}

impl SerializeWithLength for BitSlice<Msb0, u8> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        let mut size = 0;

        // Serialize the number of bits
        size += S::from_usize(self.len()).unwrap().serialize(writer)?;

        // Serialize the BitVec as byte slice
        let base_slice = self.as_slice();
        let target_len = bits_to_bytes(self.len());
        if base_slice.len() == target_len {
            // Fast path: The underlying slice is aligned with the BitSlice edges
            writer.write_all(base_slice)?;
        } else {
            // Slow path: Slice is unaligned, iterate over chunks of 8 bits
            // TODO Copy instead via to_vec()?
            let mut chunks = self.chunks_exact(8);
            for chunk in &mut chunks {
                let mut byte = 0u8;
                byte.bits_mut().copy_from_slice(chunk);
                writer.write_all(&[byte])?;
            }
            let remainder = chunks.remainder();
            if !remainder.is_empty() {
                let mut byte = 0u8;
                let bits = &mut byte.bits_mut::<Msb0>()[..remainder.len()];
                bits.copy_from_slice(remainder);
                writer.write_all(&[byte])?;
            }
        }
        size += target_len;

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

impl SerializeWithLength for BitVec<Msb0, u8> {
    fn serialize<S: Serialize + num::FromPrimitive, W: WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, SerializingError> {
        self.as_bitslice().serialize::<S, W>(writer)
    }

    fn serialized_size<S: Serialize + num::FromPrimitive>(&self) -> usize {
        self.as_bitslice().serialized_size::<S>()
    }
}

impl DeserializeWithLength for BitVec<Msb0, u8> {
    fn deserialize_with_limit<D: Deserialize + num::ToPrimitive, R: ReadBytesExt>(
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

        let mut bitvec: BitVec<Msb0, u8> = BitVec::from_vec(data);
        bitvec.resize(n_bits, false);
        Ok(bitvec)
    }
}

#[cfg(test)]
mod tests {
    use bitvec::prelude::Msb0;

    use super::*;

    fn reserialize(bits: &BitSlice<Msb0, u8>, raw: &[u8]) {
        let serialized = bits.serialize_to_vec::<u8>();
        assert_eq!(&serialized[..], raw);
        let deserialized_bits =
            BitVec::<Msb0, u8>::deserialize_from_vec::<u8>(&serialized).unwrap();
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
        let base_bits = base.bits::<Msb0>();
        let bits = &base_bits[4..31];
        reserialize(bits, &b"\x1b\x20\x42\x04\x20"[..]);
    }

    #[test]
    fn it_correctly_serializes_and_deserializes_bitvec() {
        let base: Vec<u8> = vec![0x42, 0x04, 0x20, 0x43];
        let bits = BitVec::<Msb0, u8>::from_slice(&base);
        reserialize(&bits, &b"\x20\x42\x04\x20\x43"[..]);
    }
}
