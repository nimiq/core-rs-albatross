use std::{
    cmp::{self, Ordering},
    fmt, io,
    ops::{self, Range},
    str,
};

use byteorder::WriteBytesExt;
use log::error;
#[cfg(feature = "serde-derive")]
use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::{HashOutput, SerializeContent};
use nimiq_keys::Address;

/// The nuber of bits in a nibble.
/// The implementation only works for values between 1 and 8.
pub const NIBBLE_BITS: u8 = 5;

/// A compact representation of a node's key. It stores the key in big endian. Each byte
/// stores up to 8/NIBBLE_BITS nibbles.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde-derive", derive(DbSerializable))]
pub struct KeyNibbles {
    /// Invariant: Unused nibbles are always zeroed.
    bytes: [u8; KeyNibbles::MAX_BYTES],
    /// The length in number of nibbles.
    length: u8,
}

trait BitpackingOps {
    /// Returns a value containing just the bits in the given range and shifted to the right.
    /// We count the bits from the most-significant position.
    fn get(&self, range: Range<u8>) -> u8;
    /// Sets the bits in the given range to the given value.
    fn set(&mut self, range: Range<u8>, value: u8);
    /// Concatenates two values.
    fn concat(&self, other: Self, other_len: usize) -> Self;
}

impl BitpackingOps for u8 {
    fn get(&self, range: Range<u8>) -> u8 {
        if range.start >= 8 || range.end > 8 {
            panic!("Invalid range: {:?}", range);
        }
        // We count the bits from the most-significant position.
        (self >> (8 - range.end)) & ((1 << (range.end - range.start)) - 1)
    }

    fn set(&mut self, range: Range<u8>, value: u8) {
        if range.start >= 8 || range.end > 8 {
            panic!("Invalid range: {:?}", range);
        }
        let shift = 8 - range.end;
        // Clear the bits in the range.
        let mask = (1 << (range.end - range.start)) - 1;
        *self &= !(mask << shift);
        // Set the bits to the new value.
        *self |= (value & mask) << shift;
    }

    fn concat(&self, other: Self, other_len: usize) -> Self {
        (self << other_len) | (other & ((1 << other_len) - 1))
    }
}

impl KeyNibbles {
    pub const MAX_BYTES: usize = 63;

    /// The root (empty) key.
    pub const ROOT: KeyNibbles = KeyNibbles {
        bytes: [0; KeyNibbles::MAX_BYTES],
        length: 0,
    };

    pub const BADBADBAD: KeyNibbles = KeyNibbles {
        bytes: [
            0xba, 0xdb, 0xad, 0xba, 0xd0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
        length: (9u8 * 4).div_ceil(NIBBLE_BITS),
    };

    /// Returns the length of the key in nibbles.
    pub fn len(&self) -> usize {
        self.length as usize
    }

    fn bytes_len(&self) -> usize {
        (self.len() * NIBBLE_BITS as usize).div_ceil(8)
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns the nibble at the given index as an usize. The usize represents a hexadecimal
    /// character.
    pub fn get(&self, index: usize) -> Option<usize> {
        if index >= self.len() {
            if index != 0 {
                error!(
                    "Index {} exceeds the length of KeyNibbles {}, which has length {}.",
                    index,
                    self,
                    self.len()
                );
            }
            return None;
        }

        // The nibble can be part of up to two bytes.
        // Calculate the starting byte.
        let start_byte = (index * NIBBLE_BITS as usize) / 8;
        // Calculate the position where the nibble starts in the first byte.
        let byte_offset = (index * NIBBLE_BITS as usize) % 8;
        // Calculate the number of bits in the first byte.
        let bits_in_first_byte = cmp::min(NIBBLE_BITS as usize, 8 - byte_offset);
        // Calculate the number of bits in the second byte.
        let bits_in_second_byte = NIBBLE_BITS as usize - bits_in_first_byte;

        // The nibble is possibly split between two bytes.
        let first_part =
            self.bytes[start_byte].get(byte_offset as u8..(byte_offset + bits_in_first_byte) as u8);
        // We get the remaining bits from the next byte.
        let second_part = if bits_in_second_byte > 0 && start_byte + 1 < Self::MAX_BYTES {
            self.bytes[start_byte + 1].get(0..bits_in_second_byte as u8)
        } else {
            // If we would overflow our bytes, the next part is always 0.
            0
        };
        let nibble = first_part.concat(second_part, bits_in_second_byte);

        Some(nibble as usize)
    }

    fn set(&mut self, index: usize, value: usize) {
        if index > self.len() {
            if index != 0 {
                error!(
                    "Index {} exceeds the length of KeyNibbles {}, which has length {}.",
                    index,
                    self,
                    self.len()
                );
            }
            return;
        }
        // If this is the last index, append.
        else if index == self.len() {
            self.length += 1;
        }

        // The nibble can be part of up to two bytes.
        // Calculate the starting byte.
        let start_byte = (index * NIBBLE_BITS as usize) / 8;
        // Calculate the position where the nibble starts in the first byte.
        let byte_offset = (index * NIBBLE_BITS as usize) % 8;
        // Calculate the number of bits in the first byte.
        let bits_in_first_byte = cmp::min(NIBBLE_BITS as usize, 8 - byte_offset);
        // Calculate the number of bits in the second byte.
        let bits_in_second_byte = NIBBLE_BITS as usize - bits_in_first_byte;

        // The nibble is possibly split between two bytes.
        self.bytes[start_byte].set(
            byte_offset as u8..(byte_offset + bits_in_first_byte) as u8,
            value as u8 >> bits_in_second_byte,
        );
        // We get the remaining bits from the next byte.
        if bits_in_second_byte > 0 && start_byte + 1 < Self::MAX_BYTES {
            self.bytes[start_byte + 1].set(0..bits_in_second_byte as u8, value as u8)
        }
    }

    /// Returns the next possible `KeyNibbles` in the ordering relation.
    ///
    /// This is always the same, but with a single `0` nibble appended.
    pub fn successor(&self) -> Self {
        let mut result = self.clone();
        result.length += 1;
        result
    }

    /// Checks if the current key is a prefix of the given key. If the keys are equal it also
    /// returns true.
    pub fn is_prefix_of(&self, other: &KeyNibbles) -> bool {
        if self.length > other.length {
            return false;
        }

        // Get the last byte index and check if the key is a full number of bytes.
        let end_byte = (self.len() * NIBBLE_BITS as usize) / 8;

        // If key ends in the middle of a byte, compare that part.
        if (self.len() * NIBBLE_BITS as usize) % 8 != 0 {
            let own_nibble = self.get(self.len() - 1).unwrap();
            let other_nibble = other.get(self.len() - 1).unwrap();

            if own_nibble != other_nibble {
                return false;
            }
        }

        // Compare the remaining bytes.
        self.bytes[..end_byte] == other.bytes[..end_byte]
    }

    /// Returns the common prefix between the current key and a given key.
    #[must_use]
    pub fn common_prefix(&self, other: &KeyNibbles) -> Self {
        // Get the smaller length (in nibbles) of the two keys.
        let min_len = cmp::min(self.len(), other.len());

        // Calculate the minimum length (in bytes) rounded up.
        let byte_len = (min_len * NIBBLE_BITS as usize).div_ceil(8);

        // Find the nibble index where the two keys first differ.
        let mut first_difference_nibble = min_len;

        for j in 0..byte_len {
            if self.bytes[j] != other.bytes[j] {
                let first_nibble_in_byte = (j * 8) / NIBBLE_BITS as usize;
                if self.get(first_nibble_in_byte) != other.get(first_nibble_in_byte) {
                    first_difference_nibble = first_nibble_in_byte
                } else {
                    first_difference_nibble = first_nibble_in_byte + 1
                };
                break;
            }
        }

        // Get the slice corresponding to the common prefix.
        self.slice(0, first_difference_nibble)
    }

    /// Returns a slice of the current key. Starting at the given start (inclusive) nibble index and
    /// ending at the given end (exclusive) nibble index.
    #[must_use]
    pub fn slice(&self, start: usize, end: usize) -> Self {
        // Do some basic sanity checks.
        if start > self.len() || end < start {
            error!(
                "Slice parameters don't make sense! Key length {}, start index {}, end index {}.",
                self.len(),
                start,
                end
            );
            return KeyNibbles::ROOT;
        }

        // Calculate the end nibble index (it can't exceed the key length).
        let end = cmp::min(end, self.len());

        // TODO: Optimizations.
        let mut key = KeyNibbles::ROOT;
        for (j, i) in (start..end).enumerate() {
            let nibble = self.get(i).unwrap();
            key.set(j, nibble);
        }

        // Return the slice as a new key.
        key
    }

    /// Returns the suffix of the current key starting at the given nibble index.
    #[must_use]
    pub fn suffix(&self, start: u8) -> Self {
        self.slice(start as usize, self.len())
    }

    pub fn to_address(&self) -> Option<Address> {
        if self.len() != 2 * Address::len() {
            return None;
        }
        Some(Address::from(&self.bytes[..Address::len()]))
    }

    /// Returns whether `self` or `other` comes first in post-order tree traversal.
    pub fn post_order_cmp(&self, other: &Self) -> Ordering {
        match (self.is_prefix_of(other), other.is_prefix_of(self)) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.cmp(other),
        }
    }
}

impl Default for KeyNibbles {
    fn default() -> KeyNibbles {
        KeyNibbles::ROOT
    }
}

impl From<&Address> for KeyNibbles {
    fn from(address: &Address) -> Self {
        KeyNibbles::from(address.as_bytes())
    }
}

impl From<&[u8]> for KeyNibbles {
    fn from(v: &[u8]) -> Self {
        if v.len() > KeyNibbles::MAX_BYTES {
            error!(
                "Array of len {} exceeds the max length of KeyNibbles {}",
                v.len(),
                KeyNibbles::MAX_BYTES,
            );
            return KeyNibbles::ROOT;
        }
        let mut new_bytes = [0; KeyNibbles::MAX_BYTES];
        new_bytes[..v.len()].copy_from_slice(v);
        KeyNibbles {
            bytes: new_bytes,
            length: (v.len() * 2) as u8,
        }
    }
}

impl str::FromStr for KeyNibbles {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "ε" {
            return Ok(KeyNibbles::ROOT);
        }
        let mut bytes: [u8; KeyNibbles::MAX_BYTES] = [0; KeyNibbles::MAX_BYTES];
        if s.len() % 2 == 0 {
            hex::decode_to_slice(s, &mut bytes[..(s.len() / 2)])?;

            Ok(KeyNibbles {
                bytes,
                length: s.len() as u8,
            })
        } else {
            let complete_nibble_range = ..(s.len() - 1);
            let complete_byte_range = ..(s.len() / 2);
            hex::decode_to_slice(&s[complete_nibble_range], &mut bytes[complete_byte_range])?;

            let last_nibble = s.chars().last().unwrap();

            let last_nibble =
                last_nibble
                    .to_digit(16)
                    .ok_or(hex::FromHexError::InvalidHexCharacter {
                        c: last_nibble,
                        index: s.len() - 1,
                    })?;

            bytes[complete_byte_range.end] = (last_nibble as u8) << 4;

            Ok(KeyNibbles {
                bytes,
                length: s.len() as u8,
            })
        }
    }
}

impl fmt::Display for KeyNibbles {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.length == 0 {
            return f.write_str("ε");
        }
        let mut hex_representation = hex::encode(&self.bytes[..self.bytes_len()]);

        // If prefix ends in the middle of a byte, remove last char.
        if self.length % 2 == 1 {
            hex_representation.pop();
        }

        f.write_str(&hex_representation)
    }
}

impl fmt::Debug for KeyNibbles {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl ops::Add<&KeyNibbles> for &KeyNibbles {
    type Output = KeyNibbles;

    fn add(self, other: &KeyNibbles) -> KeyNibbles {
        let mut new_key = self.clone();

        for i in 0..other.len() {
            new_key.set(i + self.len(), other.get(i).unwrap());
        }

        new_key
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::fmt;

    use serde::{
        de::{Deserializer, Error, Unexpected, Visitor},
        ser::Serializer,
        Deserialize, Serialize,
    };

    use super::NIBBLE_BITS;

    impl Serialize for super::KeyNibbles {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            KeyNibbles {
                len: self.len() as u8,
                bytes: KeyNibblesBytes {
                    len: self.bytes_len() as u8,
                    bytes: self.bytes,
                },
            }
            .serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for super::KeyNibbles {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let result: KeyNibbles = Deserialize::deserialize(deserializer)?;
            if result.len > super::KeyNibbles::MAX_BYTES as u8 * 8 / NIBBLE_BITS {
                // length too high
                return Err(D::Error::invalid_length(
                    result.len as usize,
                    &"fewer nibbles",
                ));
            }
            // TODO: Fix
            // if result.bytes.len != (result.len + 1) / 2 {
            //     // bytes length incorrect
            //     return Err(D::Error::invalid_length(
            //         result.bytes.len as usize,
            //         &"matching nibble/byte length",
            //     ));
            // }
            // if result.len % 2 == 1 && result.bytes.bytes[result.bytes.len as usize - 1] & 0x0f != 0
            // {
            //     return Err(D::Error::invalid_value(
            //         Unexpected::Other("Unused nibble not zeroed"),
            //         &"unused nibbles being zeroed",
            //     ));
            // }
            Ok(super::KeyNibbles {
                length: result.len,
                bytes: result.bytes.bytes,
            })
        }
    }

    #[derive(Deserialize, Serialize)]
    struct KeyNibbles {
        len: u8,
        bytes: KeyNibblesBytes,
    }

    struct KeyNibblesBytes {
        len: u8,
        bytes: [u8; super::KeyNibbles::MAX_BYTES],
    }

    struct KeyNibblesBytesVisitor;

    impl<'de> Visitor<'de> for KeyNibblesBytesVisitor {
        type Value = KeyNibblesBytes;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(
                formatter,
                "maximum of {} key nibble bytes",
                super::KeyNibbles::MAX_BYTES
            )
        }

        fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<KeyNibblesBytes, E> {
            if v.len() > super::KeyNibbles::MAX_BYTES {
                return Err(E::invalid_length(v.len(), &self)); // length too high
            }
            let mut result = KeyNibblesBytes {
                len: v.len() as u8,
                bytes: [0; super::KeyNibbles::MAX_BYTES],
            };
            result.bytes[..v.len()].copy_from_slice(v);
            Ok(result)
        }
    }

    impl Serialize for KeyNibblesBytes {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_bytes(&self.bytes[..self.len as usize])
        }
    }

    impl<'de> Deserialize<'de> for KeyNibblesBytes {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_bytes(KeyNibblesBytesVisitor)
        }
    }
}

impl SerializeContent for KeyNibbles {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.length)?;
        writer.write_all(&self.bytes[..self.bytes_len()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn to_from_str_works() {
        let key: KeyNibbles = "cfb98637bcae43c13323eaa1731ced2b716962fd".parse().unwrap();
        assert_eq!(key.to_string(), "cfb98637bcae43c13323eaa1731ced2b716962fd");

        let key: KeyNibbles = "".parse().unwrap();
        assert_eq!(key.to_string(), "ε");

        let key: KeyNibbles = "ε".parse().unwrap();
        assert_eq!(key.to_string(), "ε");

        let key: KeyNibbles = "1".parse().unwrap();
        assert_eq!(key.to_string(), "1");

        let key: KeyNibbles = "23".parse().unwrap();
        assert_eq!(key.to_string(), "23");
    }

    #[test]
    fn sum_works() {
        let key1: KeyNibbles = "cfb".parse().unwrap();
        let key2: KeyNibbles = "986".parse().unwrap();

        assert_eq!((&key1 + &key2).to_string(), "cfb986");
    }

    #[test]
    fn nibbles_get_works() {
        let key: KeyNibbles = "cfb98637bcae43c13323eaa1731ced2b716962fd".parse().unwrap();

        assert_eq!(key.get(0), Some(12));
        assert_eq!(key.get(1), Some(15));
        assert_eq!(key.get(2), Some(11));
        assert_eq!(key.get(3), Some(9));
        assert_eq!(key.get(41), None);
        assert_eq!(key.get(42), None);
    }

    #[test]
    fn nibbles_slice_works() {
        let key: KeyNibbles = "cfb98637bcae43c13323eaa1731ced2b716962fd".parse().unwrap();

        assert_eq!(key.slice(0, 1).to_string(), "c");
        assert_eq!(key.slice(0, 2).to_string(), "cf");
        assert_eq!(key.slice(0, 3).to_string(), "cfb");
        assert_eq!(key.slice(1, 3).to_string(), "fb");
        assert_eq!(
            key.slice(0, 41).to_string(),
            "cfb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(
            key.slice(1, 40).to_string(),
            "fb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(key.slice(2, 1).to_string(), "ε");
        assert_eq!(key.slice(42, 43).to_string(), "ε");
    }

    #[test]
    fn nibbles_suffix_works() {
        let key: KeyNibbles = "cfb98637bcae43c13323eaa1731ced2b716962fd".parse().unwrap();

        assert_eq!(
            key.suffix(0).to_string(),
            "cfb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(
            key.suffix(1).to_string(),
            "fb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(
            key.suffix(2).to_string(),
            "b98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(key.suffix(40).to_string(), "ε");
        assert_eq!(key.suffix(42).to_string(), "ε");
    }

    #[test]
    fn nibbles_is_prefix_of_works() {
        let key1: KeyNibbles = "cfb98637bcae43c13323eaa1731ced2b716962fd".parse().unwrap();
        let key2: KeyNibbles = "cfb".parse().unwrap();

        assert!(key2.is_prefix_of(&key1));
        assert!(!key1.is_prefix_of(&key2));
    }

    #[test]
    fn nibbles_common_prefix_works() {
        let key1: KeyNibbles = "1000000000000000000000000000000000000000".parse().unwrap();
        let key2: KeyNibbles = "1200000000000000000000000000000000000000".parse().unwrap();
        assert_eq!(key1.common_prefix(&key2), key2.common_prefix(&key1));
        assert_eq!(key1.common_prefix(&key2).to_string(), "1");

        let key3: KeyNibbles = "2dc".parse().unwrap();
        let key4: KeyNibbles = "2da3183636aae21c2710b5bd4486903f8541fb80".parse().unwrap();
        assert_eq!(key3.common_prefix(&key4), "2d".parse().unwrap());

        let key5: KeyNibbles = "2da".parse().unwrap();
        let key6: KeyNibbles = "2da3183636aae21c2710b5bd4486903f8541fb80".parse().unwrap();
        assert_eq!(key5.common_prefix(&key6), "2da".parse().unwrap());
    }

    #[test]
    fn successor_works() {
        let key1: KeyNibbles = "".parse().unwrap();
        let key1s: KeyNibbles = "0".parse().unwrap();
        assert_eq!(key1.successor(), key1s);

        let key2: KeyNibbles = "123".parse().unwrap();
        let key2s: KeyNibbles = "1230".parse().unwrap();
        assert_eq!(key2.successor(), key2s);
    }

    #[test]
    fn post_order_cmp_works() {
        let keys: Vec<KeyNibbles> = ["cfb986f5a", "cfb98e0f6", "cfb98e0f", "cfb98", ""]
            .into_iter()
            .map(|s| s.parse().unwrap())
            .collect();
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                assert_eq!(
                    keys[i].post_order_cmp(&keys[j]),
                    Ordering::Less,
                    "{} < {}",
                    keys[i],
                    keys[j]
                );
                assert_eq!(
                    keys[j].post_order_cmp(&keys[i]),
                    Ordering::Greater,
                    "{} > {}",
                    keys[i],
                    keys[j]
                );
            }
        }
        for key in &keys {
            assert_eq!(
                key.post_order_cmp(key),
                Ordering::Equal,
                "{} == {}",
                key,
                key
            );
        }
    }
}

#[cfg(all(test, feature = "serde-derive"))]
mod serde_tests {
    use nimiq_serde::{Deserialize, Serialize};

    use super::KeyNibbles;

    #[test]
    fn empty() {
        assert_eq!(
            KeyNibbles::deserialize_all(b"\x00\x00").unwrap(),
            KeyNibbles::ROOT,
        );
        assert_eq!(KeyNibbles::ROOT.serialize_to_vec(), b"\x00\x00");
    }

    #[test]
    fn one() {
        let one0: KeyNibbles = "0".parse().unwrap();
        let one1: KeyNibbles = "1".parse().unwrap();
        let onef: KeyNibbles = "f".parse().unwrap();
        assert_eq!(KeyNibbles::deserialize_all(b"\x01\x01\x00").unwrap(), one0);
        assert_eq!(KeyNibbles::deserialize_all(b"\x01\x01\x10").unwrap(), one1);
        assert_eq!(KeyNibbles::deserialize_all(b"\x01\x01\xf0").unwrap(), onef);
        assert_eq!(one0.serialize_to_vec(), b"\x01\x01\x00");
        assert_eq!(one1.serialize_to_vec(), b"\x01\x01\x10");
        assert_eq!(onef.serialize_to_vec(), b"\x01\x01\xf0");
    }

    #[test]
    fn two() {
        let two: KeyNibbles = "9a".parse().unwrap();
        assert_eq!(KeyNibbles::deserialize_all(b"\x02\x01\x9a").unwrap(), two);
        assert_eq!(two.serialize_to_vec(), b"\x02\x01\x9a");
    }

    #[test]
    fn longer() {
        let longer1: KeyNibbles = "68656c6c6f2c20776f726c6421".parse().unwrap();
        let longer2: KeyNibbles = "68656c6c6f2c20776f726c64215".parse().unwrap();
        assert_eq!(
            KeyNibbles::deserialize_all(b"\x1a\x0dhello, world!").unwrap(),
            longer1,
        );
        assert_eq!(
            KeyNibbles::deserialize_all(b"\x1b\x0ehello, world!\x50").unwrap(),
            longer2,
        );
        assert_eq!(longer1.serialize_to_vec(), b"\x1a\x0dhello, world!");
        assert_eq!(longer2.serialize_to_vec(), b"\x1b\x0ehello, world!\x50");
    }

    #[test]
    fn error() {
        assert!(KeyNibbles::deserialize_from_vec(b"").is_err());
        assert!(KeyNibbles::deserialize_from_vec(b"\x00").is_err());
        assert!(KeyNibbles::deserialize_from_vec(b"\xff").is_err());
        assert!(KeyNibbles::deserialize_from_vec(b"\x00\x01\x00").is_err());
        assert!(KeyNibbles::deserialize_from_vec(b"\x01\x00\x00").is_err());
    }
}
