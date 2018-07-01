use beserial::{Serialize, Deserialize, WriteBytesExt, ReadBytesExt, SerializeWithLength, DeserializeWithLength};
use consensus::base::primitive::hash::{Hash, SerializeContent};
use consensus::base::primitive::Address;
use std::ops;
use std::usize;
use std::fmt;
use std::str;
use std::io;
use std::cmp;
use hex;

// Stores a compact representation of length nibbles.
// Each u8 stores up to 2 nibbles.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Hash)]
pub (in super) struct AddressNibbles {
    bytes: Vec<u8>,
    length: u8
}

impl AddressNibbles {
    pub (in super) fn empty() -> AddressNibbles {
        return AddressNibbles {
            bytes: Vec::new(),
            length: 0,
        };
    }

    #[inline]
    pub (in super) fn len(&self) -> usize {
        return self.length as usize;
    }

    pub (in super) fn get(&self, index: usize) -> Option<usize> {
        if index >= self.len() {
            return None;
        }
        let byte = index / 2;
        let nibble = index % 2;
        return Some(((self.bytes[byte] >> ((1 - nibble) * 4)) & 0xf) as usize);
    }

    pub (in super) fn is_prefix_of(&self, other: &AddressNibbles) -> bool {
        // Prefix must be shorter or equal in length.
        if self.length > other.length {
            return false;
        }
        let ends_in_byte = self.length % 2 == 1;
        let end = self.len() / 2;
        // If prefix ends in the middle of a byte, compare that part as well.
        if ends_in_byte {
            let own_nibble = (self.bytes[end] >> 4) & 0xf;
            let other_nibble = (self.bytes[end] >> 4) & 0xf;
            if own_nibble != other_nibble {
                return false;
            }
        }
        return self.bytes[..end] == other.bytes[..end];
    }

    pub (in super) fn common_prefix(&self, other: &AddressNibbles) -> AddressNibbles {
        let min_len = cmp::min(self.len(), other.len());
        let byte_len = min_len / 2;
        let mut i = byte_len;
        for j in 0..byte_len {
            if self.bytes[j] != other.bytes[j] {
                i = j + 1;
                break;
            }
        }
        // We checked full bytes, so convert i from bytes to nibbles.
        i *= 2;
        // Check first half of last byte if necessary (all bytes where the same and there is a nibble left).
        if i == byte_len * 2 && min_len % 2 == 1 && self.get(min_len - 1) == other.get(min_len - 1) {
            i += 1;
        }
        return AddressNibbles {
            bytes: self.bytes[..i].to_vec(),
            length: i as u8,
        };
    }
}

impl<'a> From<&'a Address> for AddressNibbles {
    fn from(address: &'a Address) -> Self {
        return AddressNibbles::from(address.as_bytes());
    }
}

impl<'a> From<&'a [u8]> for AddressNibbles {
    fn from(v: &'a [u8]) -> Self {
        return AddressNibbles {
            bytes: v.to_vec(),
            length: (v.len() * 2) as u8
        };
    }
}

impl fmt::Display for AddressNibbles {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut hex_representation = hex::encode(&self.bytes);
        // If prefix ends in the middle of a byte, remove last char.
        if self.length % 2 == 1 {
            hex_representation.pop();
        }
        return f.write_str(&hex_representation);
    }
}

impl str::FromStr for AddressNibbles {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() % 2 == 0 {
            let v: Vec<u8> = hex::decode(s)?;
            return Ok(AddressNibbles::from(v.as_slice()));
        } else {
            let mut v: Vec<u8> = hex::decode(&s[..s.len() - 1])?;
            let last_nibble = s.chars().last().unwrap();
            let last_nibble = last_nibble.to_digit(16)
                .ok_or_else(|| hex::FromHexError::InvalidHexCharacter { c: last_nibble, index: s.len() - 1 })?;
            v.push((last_nibble as u8) << 4);
            return Ok(AddressNibbles {
                bytes: v,
                length: s.len() as u8
            });
        }
    }
}

impl SerializeContent for AddressNibbles {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = SerializeWithLength::serialize::<u8, W>(&self.to_string(), writer)?;
        return Ok(size);
    }
}

impl Hash for AddressNibbles {}

impl<'a, 'b> ops::Add<&'b AddressNibbles> for &'a AddressNibbles {
    type Output = AddressNibbles;
    fn add(self, other: &'b AddressNibbles) -> AddressNibbles {
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

        return AddressNibbles {
            bytes,
            length: self.length + other.length
        };
    }
}

impl<'b> ops::Add<&'b AddressNibbles> for AddressNibbles {
    type Output = AddressNibbles;
    fn add(self, rhs: &'b AddressNibbles) -> AddressNibbles {
        &self + rhs
    }
}

impl<'a> ops::Add<AddressNibbles> for &'a AddressNibbles {
    type Output = AddressNibbles;
    fn add(self, rhs: AddressNibbles) -> AddressNibbles {
        self + &rhs
    }
}

impl ops::Add<AddressNibbles> for AddressNibbles {
    type Output = AddressNibbles;
    fn add(self, rhs: AddressNibbles) -> AddressNibbles {
        &self + &rhs
    }
}

impl Serialize for AddressNibbles {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> io::Result<usize> {
        let size = SerializeWithLength::serialize::<u8, W>(&self.to_string(), writer)?;
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        return /*length*/ 1 + self.len();
    }
}

impl Deserialize for AddressNibbles {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let hex_repr: String = DeserializeWithLength::deserialize::<u8, R>(reader)?;
        let pot_address: Result<AddressNibbles, hex::FromHexError> = hex_repr.parse();

        return match pot_address {
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            Ok(address) => Ok(address),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn it_can_convert_and_access_nibbles() {
        let address = Address::from(hex::decode("cfb98637bcae43c13323eaa1731ced2b716962fd").unwrap().as_slice());
        let an = AddressNibbles::from(&address);
        assert_eq!(an.bytes, address.as_bytes());
        assert_eq!(an.get(0), Some(12));
        assert_eq!(an.get(1), Some(15));
        assert_eq!(an.get(2), Some(11));
        assert_eq!(an.get(3), Some(9));
        assert_eq!(an.to_string(), "cfb98637bcae43c13323eaa1731ced2b716962fd");

        let an2: AddressNibbles = "cfb".parse().unwrap();
        assert_eq!(an2.get(3), None);
        assert_eq!(an2.get(4), None);
        assert_ne!(an, an2);
        assert!(an2.is_prefix_of(&an));
        assert!(!an.is_prefix_of(&an2));
        assert_eq!(an2.to_string(), "cfb");
        assert_eq!((&an2 + &an2).to_string(), "cfbcfb");
    }
}
