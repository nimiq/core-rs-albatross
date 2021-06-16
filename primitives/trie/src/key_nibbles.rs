use std::borrow::Cow;
use std::cmp;
use std::fmt;
use std::io;
use std::ops;
use std::str;
use std::usize;

use log::error;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_database::AsDatabaseBytes;
use nimiq_hash::{Hash, SerializeContent};
use nimiq_keys::Address;

/// A compact representation of a node's key. It stores the key in big endian. Each byte
/// stores up to 2 nibbles. Internally, we assume that a key is represented in hexadecimal form.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct KeyNibbles {
    bytes: Vec<u8>,
    length: u8,
}

impl KeyNibbles {
    /// Create an empty key.
    pub fn empty() -> KeyNibbles {
        KeyNibbles {
            bytes: Vec::new(),
            length: 0,
        }
    }

    /// Returns the length of the key in nibbles.
    pub fn len(&self) -> usize {
        self.length as usize
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
            error!("Key {} must be shorter or equal in length than the other key {}. Otherwise it can't be a prefix evidently.", self, other);
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
    pub fn common_prefix(&self, other: &KeyNibbles) -> KeyNibbles {
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
    pub fn slice(&self, start: usize, end: usize) -> KeyNibbles {
        // Do some basic sanity checks.
        if start >= self.len() || end <= start {
            error!(
                "Slice parameters don't make sense! Key length {}, start index {}, end index {}.",
                self.len(),
                start,
                end
            );
            return KeyNibbles::empty();
        }

        // Calculate the end nibble index (it can't exceed the key length).
        let end = cmp::min(end, self.len());

        // Get the start and end in bytes (rounded down).
        let byte_start = start / 2;
        let byte_end = end / 2;

        // Get the nibbles for the slice.
        let mut new_bytes = Vec::new();

        // If the slice starts at the beginning of a byte, then it's an easy case.
        if start % 2 == 0 {
            new_bytes = self.bytes[byte_start..byte_end].to_vec()
        }
        // Otherwise we need to shift everything by one nibble.
        else {
            let mut current_byte = (self.bytes[byte_start] & 0xf) << 4; // Right nibble.

            for i in (byte_start + 1)..byte_end {
                let tmp_byte = self.bytes[i];

                let left_nibble = (tmp_byte >> 4) & 0xf;

                new_bytes.push(current_byte | left_nibble);

                current_byte = (tmp_byte & 0xf) << 4;
            }

            new_bytes.push(current_byte);
        };

        // If we have an odd number of nibbles we add the last nibble now.
        if end % 2 == 1 {
            let last_nibble = self.bytes[byte_end] & 0xf0;

            if start % 2 == 0 {
                new_bytes.push(last_nibble);
            } else {
                let last_byte = new_bytes.pop().unwrap();
                new_bytes.push(last_byte | (last_nibble >> 4));
            }
        }

        // Return the slice as a new key.
        KeyNibbles {
            bytes: new_bytes,
            length: (end - start) as u8,
        }
    }

    /// Returns the suffix of the current key starting at the given nibble index.
    pub fn suffix(&self, start: u8) -> KeyNibbles {
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
        KeyNibbles {
            bytes: v.to_vec(),
            length: (v.len() * 2) as u8,
        }
    }
}

impl str::FromStr for KeyNibbles {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() % 2 == 0 {
            let v: Vec<u8> = hex::decode(s)?;

            Ok(KeyNibbles::from(v.as_slice()))
        } else {
            let mut v: Vec<u8> = hex::decode(&s[..s.len() - 1])?;

            let last_nibble = s.chars().last().unwrap();

            let last_nibble =
                last_nibble
                    .to_digit(16)
                    .ok_or_else(|| hex::FromHexError::InvalidHexCharacter {
                        c: last_nibble,
                        index: s.len() - 1,
                    })?;

            v.push((last_nibble as u8) << 4);

            Ok(KeyNibbles {
                bytes: v,
                length: s.len() as u8,
            })
        }
    }
}

impl fmt::Display for KeyNibbles {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut hex_representation = hex::encode(&self.bytes);

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
        let mut bytes = self.bytes.clone();

        if self.len() % 2 == 0 {
            // Easy case: the lhs ends with a full byte.
            bytes.extend(&other.bytes);
        } else {
            // Complex case: the lhs ends in the middle of a byte.
            let mut next_byte = bytes.pop().unwrap();

            for byte in other.bytes.iter() {
                let left_nibble = byte >> 4;
                bytes.push(next_byte | left_nibble);
                next_byte = (byte & 0xf) << 4;
            }

            if other.length % 2 == 0 {
                bytes.push(next_byte);
            }
        }

        KeyNibbles {
            bytes,
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
        let size = SerializeWithLength::serialize::<u8, W>(&self.to_string(), writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        1 + self.len()
    }
}

impl Deserialize for KeyNibbles {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let hex_repr: String = DeserializeWithLength::deserialize::<u8, R>(reader)?;
        let pot_address: Result<KeyNibbles, hex::FromHexError> = hex_repr.parse();

        match pot_address {
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e).into()),
            Ok(address) => Ok(address),
        }
    }
}

impl SerializeContent for KeyNibbles {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = SerializeWithLength::serialize::<u8, W>(&self.to_string(), writer)?;
        Ok(size)
    }
}

impl Hash for KeyNibbles {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_from_str_works() {
        let key: KeyNibbles = "cfb98637bcae43c13323eaa1731ced2b716962fd".parse().unwrap();

        assert_eq!(key.to_string(), "cfb98637bcae43c13323eaa1731ced2b716962fd");
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
