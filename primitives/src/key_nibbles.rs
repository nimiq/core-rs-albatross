use std::{
    cmp::{self, Ordering},
    fmt, io, ops, str,
};

use byteorder::WriteBytesExt;
use log::error;
#[cfg(feature = "serde-derive")]
use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::{HashOutput, SerializeContent};
use nimiq_keys::Address;

/// A compact representation of a node's key. It stores the key in big endian. Each byte
/// stores up to 2 nibbles. Internally, we assume that a key is represented in hexadecimal form.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde-derive", derive(DbSerializable))]
pub struct KeyNibbles {
    /// Invariant: Unused nibbles are always zeroed.
    bytes: [u8; KeyNibbles::MAX_BYTES],
    length: u8,
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
        length: 9,
    };

    /// Returns the length of the key in nibbles.
    pub fn len(&self) -> usize {
        self.length as usize
    }

    fn bytes_len(&self) -> usize {
        (self.len() + 1) / 2
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

        let byte = index / 2;

        let nibble = index % 2;

        Some(((self.bytes[byte] >> ((1 - nibble) * 4)) & 0xf) as usize)
    }

    /// Returns the next possible `KeyNibbles` in the ordering relation.
    ///
    /// This is always the same, but with a single `0` digit appended.
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

        // Get the last byte index and check if the key is an even number of nibbles.
        let end_byte = self.len() / 2;

        // If key ends in the middle of a byte, compare that part.
        if self.length % 2 == 1 {
            let own_nibble = (self.bytes[end_byte] >> 4) & 0xf;

            let other_nibble = (other.bytes[end_byte] >> 4) & 0xf;

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
        let byte_len = min_len / 2 + (min_len % 2);

        // Find the nibble index where the two keys first differ.
        let mut first_difference_nibble = min_len;

        for j in 0..byte_len {
            if self.bytes[j] != other.bytes[j] {
                if self.get(j * 2) != other.get(j * 2) {
                    first_difference_nibble = j * 2
                } else {
                    first_difference_nibble = j * 2 + 1
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

        // Get the start and end in bytes (rounded down).
        let byte_start = start / 2;
        let byte_end = end / 2;

        // Get the nibbles for the slice.
        let mut new_bytes = [0; KeyNibbles::MAX_BYTES];
        let mut new_bytes_length = 0;

        // If the slice starts at the beginning of a byte, then it's an easy case.
        if start % 2 == 0 {
            new_bytes_length = byte_end - byte_start;
            new_bytes[0..new_bytes_length].copy_from_slice(&self.bytes[byte_start..byte_end]);
        }
        // Otherwise we need to shift everything by one nibble.
        else {
            let mut current_byte = (self.bytes[byte_start] & 0xf) << 4; // Right nibble.

            for (count, byte) in self.bytes[(byte_start + 1)..byte_end].iter().enumerate() {
                let tmp_byte = byte;

                let left_nibble = (tmp_byte >> 4) & 0xf;

                new_bytes[count] = current_byte | left_nibble;
                new_bytes_length += 1;

                current_byte = (tmp_byte & 0xf) << 4;
            }

            new_bytes[new_bytes_length] = current_byte;
            new_bytes_length += 1;
        };

        // If we have an odd number of nibbles we add the last nibble now.
        if end % 2 == 1 {
            let last_nibble = self.bytes[byte_end] & 0xf0;

            if start % 2 == 0 {
                new_bytes[new_bytes_length] = last_nibble;
            } else {
                new_bytes[new_bytes_length - 1] |= last_nibble >> 4;
            }
        }

        // Return the slice as a new key.
        KeyNibbles {
            bytes: new_bytes,
            length: (end - start) as u8,
        }
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
        let mut bytes = self.bytes;

        if self.len() % 2 == 0 {
            // Easy case: the lhs ends with a full byte.
            bytes[self.bytes_len()..self.bytes_len() + other.bytes_len()]
                .copy_from_slice(&other.bytes[..other.bytes_len()]);
        } else {
            // Complex case: the lhs ends in the middle of a byte.
            let mut next_byte = bytes[self.bytes_len() - 1];
            let mut bytes_length = self.bytes_len() - 1;

            for (count, byte) in other.bytes[..other.bytes_len()].iter().enumerate() {
                let left_nibble = byte >> 4;
                bytes[(self.bytes_len() - 1) + count] = next_byte | left_nibble;
                bytes_length += 1;
                next_byte = (byte & 0xf) << 4;
            }

            if other.length % 2 == 0 {
                // Push next_byte
                bytes[bytes_length] = next_byte;
            }
        }

        KeyNibbles {
            bytes,
            length: self.length + other.length,
        }
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::fmt;

    use serde::{
        de::{Deserialize, Deserializer, Error, SeqAccess, Unexpected, Visitor},
        ser::{Serialize, SerializeStruct, Serializer},
    };

    use super::KeyNibbles;

    const FIELDS: &[&str] = &["length", "bytes"];
    struct KeyNibblesVisitor;

    impl<'de> Visitor<'de> for KeyNibblesVisitor {
        type Value = KeyNibbles;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("struct MerklePath")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let length: u8 = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(0, &self))?;
            if length > KeyNibbles::MAX_BYTES as u8 * 2 {
                return Err(A::Error::invalid_length(length as usize, &self)); // length too high
            }
            let bytes_length = (length as usize + 1) / 2;
            let bytes: Vec<u8> = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(1, &self))?;
            if bytes.len() > bytes_length {
                return Err(A::Error::invalid_length(bytes_length, &self)); // bytes length too high
            }
            if length % 2 == 1 && bytes[bytes_length - 1] & 0x0f != 0 {
                return Err(A::Error::invalid_value(
                    Unexpected::Other("Unused nubble not zeroed"),
                    &self,
                ));
            }
            let mut nibble_bytes = [0u8; KeyNibbles::MAX_BYTES];
            nibble_bytes[..bytes_length].copy_from_slice(&bytes);
            Ok(KeyNibbles {
                bytes: nibble_bytes,
                length,
            })
        }
    }

    impl Serialize for KeyNibbles {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut state = serializer.serialize_struct("KeyNibbles", FIELDS.len())?;
            state.serialize_field(FIELDS[0], &self.length)?;
            state.serialize_field(FIELDS[1], &self.bytes[..self.bytes_len()])?;
            state.end()
        }
    }

    impl<'de> Deserialize<'de> for KeyNibbles {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_struct("KeyNibbles", FIELDS, KeyNibblesVisitor)
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
