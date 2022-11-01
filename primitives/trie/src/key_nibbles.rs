use std::borrow::Cow;
use std::cmp;
use std::fmt;
use std::ops;
use std::str;
use std::usize;

use log::error;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_database_value::AsDatabaseBytes;
use nimiq_keys::Address;

/// A compact representation of a node's key. It stores the key in big endian. Each byte
/// stores up to 2 nibbles. Internally, we assume that a key is represented in hexadecimal form.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct KeyNibbles {
    bytes: [u8; KeyNibbles::MAX_BYTES],
    bytes_length: u8,
    length: u8,
}

impl KeyNibbles {
    const MAX_BYTES: usize = 62;

    /// The root (empty) key.
    pub const ROOT: KeyNibbles = KeyNibbles {
        bytes: [0; KeyNibbles::MAX_BYTES],
        bytes_length: 0,
        length: 0,
    };

    /// Returns the length of the key in nibbles.
    pub fn len(&self) -> usize {
        self.length as usize
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns the nibble at the given index as an usize. The usize represents a hexadecimal
    /// character.
    pub fn get(&self, index: usize) -> Option<usize> {
        if index >= self.len() {
            error!(
                "Index {} exceeds the length of KeyNibbles {}, which has length {}.",
                index,
                self,
                self.len()
            );
            return None;
        }

        let byte = index / 2;

        let nibble = index % 2;

        Some(((self.bytes[byte] >> ((1 - nibble) * 4)) & 0xf) as usize)
    }

    /// Checks if the current key is a prefix of the given key. If the keys are equal it also
    /// returns true.
    pub fn is_prefix_of(&self, other: &KeyNibbles) -> bool {
        if self.length > other.length {
            return false;
        }

        // Get the last byte index and check if the key is an even number of nibbles.
        let end_byte = self.len() / 2;

        let even_nibbles = self.length % 2 == 1;

        // If key ends in the middle of a byte, compare that part.
        if even_nibbles {
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
        if start >= self.len() || end <= start {
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
                new_bytes_length += 1;
            } else {
                new_bytes[new_bytes_length - 1] |= last_nibble >> 4;
            }
        }

        // Return the slice as a new key.
        KeyNibbles {
            bytes: new_bytes,
            bytes_length: new_bytes_length as u8,
            length: (end - start) as u8,
        }
    }

    /// Returns the suffix of the current key starting at the given nibble index.
    #[must_use]
    pub fn suffix(&self, start: u8) -> Self {
        self.slice(start as usize, self.len())
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
            bytes_length: v.len() as u8,
            length: (v.len() * 2) as u8,
        }
    }
}

impl str::FromStr for KeyNibbles {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes: [u8; KeyNibbles::MAX_BYTES] = [0; KeyNibbles::MAX_BYTES];
        if s.len() % 2 == 0 {
            hex::decode_to_slice(s, &mut bytes[..(s.len() / 2)])?;

            Ok(KeyNibbles {
                bytes,
                bytes_length: (s.len() / 2) as u8,
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
                bytes_length: ((s.len() + 1) / 2) as u8,
                length: s.len() as u8,
            })
        }
    }
}

impl fmt::Display for KeyNibbles {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut hex_representation = hex::encode(&self.bytes[..self.bytes_length as usize]);

        // If prefix ends in the middle of a byte, remove last char.
        if self.length % 2 == 1 {
            hex_representation.pop();
        }

        f.write_str(&hex_representation)
    }
}

impl ops::Add<&KeyNibbles> for &KeyNibbles {
    type Output = KeyNibbles;

    fn add(self, other: &KeyNibbles) -> KeyNibbles {
        let mut bytes = self.bytes;
        let mut bytes_length;

        if self.len() % 2 == 0 {
            // Easy case: the lhs ends with a full byte.
            bytes_length = self.bytes_length + other.bytes_length;
            bytes[self.bytes_length as usize..bytes_length as usize]
                .copy_from_slice(&other.bytes[..other.bytes_length as usize]);
        } else {
            // Complex case: the lhs ends in the middle of a byte.
            let mut next_byte = bytes[(self.bytes_length - 1) as usize];
            bytes_length = self.bytes_length - 1;

            for (count, byte) in other.bytes[..other.bytes_length as usize]
                .iter()
                .enumerate()
            {
                let left_nibble = byte >> 4;
                bytes[(self.bytes_length - 1) as usize + count] = next_byte | left_nibble;
                bytes_length += 1;
                next_byte = (byte & 0xf) << 4;
            }

            if other.length % 2 == 0 {
                // Push next_byte
                bytes[bytes_length as usize] = next_byte;
                bytes_length += 1;
            }
        }

        KeyNibbles {
            bytes,
            bytes_length,
            length: self.length + other.length,
        }
    }
}

impl AsDatabaseBytes for KeyNibbles {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        // TODO: Improve KeyNibbles, so that no serialization is needed.
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl Serialize for KeyNibbles {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 2;
        writer.write_u8(self.length)?;
        writer.write_u8(self.bytes_length)?;
        size += writer.write(&self.bytes[..self.bytes_length as usize])?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        2 + self.bytes_length as usize
    }
}

impl Deserialize for KeyNibbles {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let length = reader.read_u8()?;
        let bytes_length = reader.read_u8()?;
        let mut bytes = [0; KeyNibbles::MAX_BYTES];
        reader.read_exact(&mut bytes[..bytes_length as usize])?;
        Ok(KeyNibbles {
            bytes,
            length,
            bytes_length,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nimiq_test_log::test;

    #[test]
    fn to_from_str_works() {
        let key: KeyNibbles = "cfb98637bcae43c13323eaa1731ced2b716962fd".parse().unwrap();
        assert_eq!(key.to_string(), "cfb98637bcae43c13323eaa1731ced2b716962fd");

        let key: KeyNibbles = "".parse().unwrap();
        assert_eq!(key.to_string(), "");

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
        assert_eq!(key.slice(2, 1).to_string(), "");
        assert_eq!(key.slice(42, 43).to_string(), "");
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
        assert_eq!(key.suffix(40).to_string(), "");
        assert_eq!(key.suffix(42).to_string(), "");
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
}
