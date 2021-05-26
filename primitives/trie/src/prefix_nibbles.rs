use std::borrow::Cow;
use std::cmp;
use std::fmt;
use std::io;
use std::ops;
use std::str;
use std::usize;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_database::AsDatabaseBytes;
use nimiq_hash::{Hash, SerializeContent};
use nimiq_keys::Address;

// Stores a compact representation of length nibbles.
// Each u8 stores up to 2 nibbles.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct PrefixNibbles {
    bytes: Vec<u8>,
    length: u8,
}

impl PrefixNibbles {
    pub fn empty() -> PrefixNibbles {
        PrefixNibbles {
            bytes: Vec::new(),
            length: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.length as usize
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        if index >= self.len() {
            return None;
        }

        let byte = index / 2;

        let nibble = index % 2;

        Some(((self.bytes[byte] >> ((1 - nibble) * 4)) & 0xf) as usize)
    }

    pub fn is_prefix_of(&self, other: &PrefixNibbles) -> bool {
        // Prefix must be shorter or equal in length.
        if self.length > other.length {
            return false;
        }

        let ends_in_byte = self.length % 2 == 1;

        let end = self.len() / 2;

        // If prefix ends in the middle of a byte, compare that part as well.
        if ends_in_byte {
            let own_nibble = (self.bytes[end] >> 4) & 0xf;

            let other_nibble = (other.bytes[end] >> 4) & 0xf;

            if own_nibble != other_nibble {
                return false;
            }
        }

        self.bytes[..end] == other.bytes[..end]
    }

    pub fn common_prefix(&self, other: &PrefixNibbles) -> PrefixNibbles {
        let min_len = cmp::min(self.len(), other.len());

        let byte_len = min_len / 2 + (min_len % 2);

        let mut first_difference_nibble = min_len;

        for j in 0..byte_len {
            if self.bytes[j] != other.bytes[j] {
                first_difference_nibble = if self.get(j * 2) != other.get(j * 2) {
                    j * 2
                } else {
                    j * 2 + 1
                };
                break;
            }
        }

        self.slice(0, first_difference_nibble)
    }

    pub fn slice(&self, start: usize, end: usize) -> PrefixNibbles {
        if start >= self.len() || end <= start {
            return PrefixNibbles::empty();
        }

        let end = cmp::min(end, self.len());

        let byte_start = start / 2;
        let byte_end = end / 2;

        // Easy case, starts at the beginning of a byte.
        let mut new_bytes = if start % 2 == 0 {
            self.bytes[byte_start..byte_end].to_vec()
        } else {
            let mut new_bytes: Vec<u8> = Vec::new();

            let mut current_byte = (self.bytes[byte_start] & 0xf) << 4; // Right nibble.

            for i in (byte_start + 1)..byte_end {
                let tmp_byte = self.bytes[i];

                let left_nibble = (tmp_byte >> 4) & 0xf;

                new_bytes.push(current_byte | left_nibble);

                current_byte = (tmp_byte & 0xf) << 4;
            }
            new_bytes.push(current_byte);

            new_bytes
        };

        if end % 2 == 1 {
            let last_nibble = self.bytes[byte_end] & 0xf0;

            if start % 2 == 0 {
                new_bytes.push(last_nibble);
            } else {
                let last_byte = new_bytes.pop().unwrap();
                new_bytes.push(last_byte | (last_nibble >> 4));
            }
        }

        PrefixNibbles {
            bytes: new_bytes,
            length: (end - start) as u8,
        }
    }

    pub fn suffix(&self, start: u8) -> PrefixNibbles {
        self.slice(start as usize, self.len())
    }
}

impl From<&Address> for PrefixNibbles {
    fn from(address: &Address) -> Self {
        PrefixNibbles::from(address.as_bytes())
    }
}

impl From<&[u8]> for PrefixNibbles {
    fn from(v: &[u8]) -> Self {
        PrefixNibbles {
            bytes: v.to_vec(),
            length: (v.len() * 2) as u8,
        }
    }
}

impl str::FromStr for PrefixNibbles {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() % 2 == 0 {
            let v: Vec<u8> = hex::decode(s)?;

            Ok(PrefixNibbles::from(v.as_slice()))
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

            Ok(PrefixNibbles {
                bytes: v,
                length: s.len() as u8,
            })
        }
    }
}

impl fmt::Display for PrefixNibbles {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut hex_representation = hex::encode(&self.bytes);

        // If prefix ends in the middle of a byte, remove last char.
        if self.length % 2 == 1 {
            hex_representation.pop();
        }

        f.write_str(&hex_representation)
    }
}

impl ops::Add<&PrefixNibbles> for &PrefixNibbles {
    type Output = PrefixNibbles;

    fn add(self, other: &PrefixNibbles) -> PrefixNibbles {
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

        PrefixNibbles {
            bytes,
            length: self.length + other.length,
        }
    }
}

impl AsDatabaseBytes for PrefixNibbles {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        // TODO: Improve PrefixNibbles, so that no serialization is needed.
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl Serialize for PrefixNibbles {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let size = SerializeWithLength::serialize::<u8, W>(&self.to_string(), writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        1 + self.len()
    }
}

impl Deserialize for PrefixNibbles {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let hex_repr: String = DeserializeWithLength::deserialize::<u8, R>(reader)?;
        let pot_address: Result<PrefixNibbles, hex::FromHexError> = hex_repr.parse();

        match pot_address {
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e).into()),
            Ok(address) => Ok(address),
        }
    }
}

impl SerializeContent for PrefixNibbles {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = SerializeWithLength::serialize::<u8, W>(&self.to_string(), writer)?;
        Ok(size)
    }
}

impl Hash for PrefixNibbles {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_convert_and_access_nibbles() {
        let address = Address::from(
            hex::decode("cfb98637bcae43c13323eaa1731ced2b716962fd")
                .unwrap()
                .as_slice(),
        );

        let an = PrefixNibbles::from(&address);
        assert_eq!(an.bytes, address.as_bytes());
        assert_eq!(an.get(0), Some(12));
        assert_eq!(an.get(1), Some(15));
        assert_eq!(an.get(2), Some(11));
        assert_eq!(an.get(3), Some(9));
        assert_eq!(an.to_string(), "cfb98637bcae43c13323eaa1731ced2b716962fd");

        // Test slicing
        assert_eq!(an.slice(0, 1).to_string(), "c");
        assert_eq!(an.slice(0, 2).to_string(), "cf");
        assert_eq!(an.slice(0, 3).to_string(), "cfb");
        assert_eq!(an.slice(1, 3).to_string(), "fb");
        assert_eq!(
            an.slice(0, 41).to_string(),
            "cfb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(
            an.slice(1, 40).to_string(),
            "fb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(an.slice(2, 1).to_string(), "");
        assert_eq!(an.slice(42, 43).to_string(), "");

        // Test suffixes
        assert_eq!(
            an.suffix(0).to_string(),
            "cfb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(
            an.suffix(1).to_string(),
            "fb98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(
            an.suffix(2).to_string(),
            "b98637bcae43c13323eaa1731ced2b716962fd"
        );
        assert_eq!(an.suffix(40).to_string(), "");

        let an2: PrefixNibbles = "cfb".parse().unwrap();
        assert_eq!(an2.get(3), None);
        assert_eq!(an2.get(4), None);
        assert_ne!(an, an2);
        assert!(an2.is_prefix_of(&an));
        assert!(!an.is_prefix_of(&an2));
        assert_eq!(an2.to_string(), "cfb");
        assert_eq!((&an2 + &an2).to_string(), "cfbcfb");

        let an1: PrefixNibbles = "1000000000000000000000000000000000000000".parse().unwrap();
        let an2: PrefixNibbles = "1200000000000000000000000000000000000000".parse().unwrap();
        assert_eq!(an1.common_prefix(&an2), an2.common_prefix(&an1));
        assert_eq!(an1.common_prefix(&an2).to_string(), "1");

        let an3: PrefixNibbles = "2dc".parse().unwrap();
        let an4: PrefixNibbles = "2da3183636aae21c2710b5bd4486903f8541fb80".parse().unwrap();
        assert_eq!(an3.common_prefix(&an4), "2d".parse().unwrap());

        let an5: PrefixNibbles = "2da".parse().unwrap();
        let an6: PrefixNibbles = "2da3183636aae21c2710b5bd4486903f8541fb80".parse().unwrap();
        assert_eq!(an5.common_prefix(&an6), "2da".parse().unwrap());
    }
}
