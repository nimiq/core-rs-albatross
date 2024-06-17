use std::{borrow::Cow, char, convert::From, fmt, io, iter::Iterator, str::FromStr};

use hex::FromHex;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::{hash_typed_array, Blake2bHash, Blake2bHasher, Hasher};
use nimiq_macros::create_typed_array;
use thiserror::Error;

use crate::{key_pair::KeyPair, ES256PublicKey, Ed25519PublicKey, PublicKey};

create_typed_array!(Address, u8, 20);
hash_typed_array!(Address);

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AddressParseError {
    // User-friendly
    #[error("Wrong country code")]
    WrongCountryCode,
    #[error("Wrong length")]
    WrongLength,
    #[error("Invalid checksum")]
    InvalidChecksum,
    // from Hash
    #[error("Invalid hash")]
    InvalidHash,
    // trying both
    #[error("Unknown format")]
    UnknownFormat,
}

impl Address {
    const CCODE: &'static str = "NQ";
    const NIMIQ_ALPHABET: &'static str = "0123456789ABCDEFGHJKLMNPQRSTUVXY";

    /// The lexicographically first address.
    pub const START_ADDRESS: Address = Address([0x00; 20]);
    /// The lexicographically last address.
    pub const END_ADDRESS: Address = Address([0xff; 20]);

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_user_friendly_address(friendly_addr: &str) -> Result<Address, AddressParseError> {
        let friendly_addr_wospace = str::replace(friendly_addr, " ", "");

        if friendly_addr_wospace.len() != 36 {
            return Err(AddressParseError::WrongLength);
        }
        if &friendly_addr_wospace[0..2] != Address::CCODE {
            return Err(AddressParseError::WrongCountryCode);
        }
        let mut twisted_str = String::with_capacity(friendly_addr_wospace.len());
        twisted_str.push_str(&friendly_addr_wospace[4..]);
        twisted_str.push_str(&friendly_addr_wospace[..4]);
        if Address::iban_check(&twisted_str) != 1 {
            return Err(AddressParseError::InvalidChecksum);
        }

        let mut spec = data_encoding::Specification::new();
        spec.symbols.push_str(Address::NIMIQ_ALPHABET);
        let encoding = spec.encoding().unwrap();

        let b_vec = encoding
            .decode(friendly_addr_wospace[4..].as_bytes())
            .map_err(|_| AddressParseError::UnknownFormat)?;
        let mut b = [0; 20];
        b.copy_from_slice(&b_vec[..b_vec.len()]);
        Ok(Address(b))
    }

    pub fn to_user_friendly_address(&self) -> String {
        let mut spec = data_encoding::Specification::new();
        spec.symbols.push_str(Address::NIMIQ_ALPHABET);
        let encoding = spec.encoding().unwrap();

        let base32 = encoding.encode(&self.0);
        // Fixme: Because of https://github.com/rust-lang/rust/issues/92178 we need to specify `as &str`
        let check_string = "00".to_string()
            + &(98 - Address::iban_check(&(base32.clone() + Address::CCODE + "00"))).to_string()
                as &str;
        let check = check_string
            .chars()
            .skip(check_string.len() - 2)
            .take(2)
            .collect::<String>();
        // Fixme: Because of https://github.com/rust-lang/rust/issues/92178 we need to specify `as &str`
        let friendly_addr = Address::CCODE.to_string() + (&check as &str) + (&base32 as &str);
        let mut friendly_spaces = String::with_capacity(36 + 8);
        for i in 0..9 {
            friendly_spaces.push_str(
                &friendly_addr
                    .chars()
                    .skip(4 * i)
                    .take(4)
                    .collect::<String>(),
            );
            if i != 8 {
                friendly_spaces.push(' ');
            }
        }
        friendly_spaces
    }

    fn iban_check(s: &str) -> u32 {
        let mut num = String::with_capacity(s.len() * 2);
        for c in s.to_uppercase().chars() {
            let code = c as u32;
            if (48..=57).contains(&code) {
                num.push(char::from_u32(code).unwrap());
            } else {
                num.push_str(&(code - 55).to_string());
            }
        }
        let mut tmp: String = "".to_string();
        for i in 0..(f32::ceil(num.len() as f32 / 6.0) as usize) {
            let num_substr = num.chars().skip(i * 6).take(6).collect::<String>();
            // Fixme: Because of https://github.com/rust-lang/rust/issues/92178 we need to specify `as &str`
            let num_tmp_sub = tmp.clone() + &num_substr as &str;
            tmp = (num_tmp_sub.parse::<u32>().unwrap() % 97).to_string();
        }

        tmp.parse::<u32>().unwrap()
    }

    pub fn from_any_str(s: &str) -> Result<Address, AddressParseError> {
        Address::from_user_friendly_address(&String::from(s))
            .or_else(|_| Address::from_hex(s))
            .map_err(|_| AddressParseError::UnknownFormat)
    }

    /// Returns the "burn address". This is an address for which it is extremely unlikely (basically
    /// impossible) that anyone knows the corresponding private key. Consequently this address can
    /// be used to "burn" coins (and is regularly used by Team Nimiq to do so).
    /// To be clear, it's IMPOSSIBLE for ANYONE to use the funds sent to this address.
    pub fn burn_address() -> Address {
        // We use unwrap here because we know this will not produce an error.
        Self::from_user_friendly_address("NQ07 0000 0000 0000 0000 0000 0000 0000 0000").unwrap()
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, AddressParseError> {
        let vec = Vec::from_hex(s).map_err(|_| AddressParseError::InvalidHash)?;
        if vec.len() == Self::SIZE {
            Ok(Self::from(&vec[..]))
        } else {
            Err(AddressParseError::WrongLength)
        }
    }
}

impl From<Blake2bHash> for Address {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        Address::from(&hash_arr[0..Address::len()])
    }
}

impl<'a> From<&'a Ed25519PublicKey> for Address {
    fn from(public_key: &'a Ed25519PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        Address::from(hash)
    }
}

impl<'a> From<&'a ES256PublicKey> for Address {
    fn from(public_key: &'a ES256PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        Address::from(hash)
    }
}

impl<'a> From<&'a PublicKey> for Address {
    fn from(public_key: &'a PublicKey) -> Self {
        match public_key {
            PublicKey::Ed25519(public_key) => Address::from(public_key),
            PublicKey::ES256(public_key) => Address::from(public_key),
        }
    }
}

impl<'a> From<&'a KeyPair> for Address {
    fn from(key_pair: &'a KeyPair) -> Self {
        Address::from(&key_pair.public)
    }
}

impl FromStr for Address {
    type Err = AddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_any_str(s)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_user_friendly_address())
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Address").field(&self.to_hex()).finish()
    }
}

impl AsDatabaseBytes for Address {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(self.as_bytes())
    }
}

impl FromDatabaseValue for Address {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(bytes.into())
    }
}

#[cfg(feature = "serde-derive")]
mod serde_derive {
    use std::borrow::Cow;

    use nimiq_serde::{FixedSizeByteArray, SerializedSize};
    use serde::{
        de::{Deserialize, Deserializer, Error},
        ser::{Serialize, Serializer},
    };

    use super::Address;

    impl SerializedSize for Address {
        const SIZE: usize = Address::SIZE;
    }

    impl Serialize for Address {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if serializer.is_human_readable() {
                serializer.serialize_str(&self.to_user_friendly_address())
            } else {
                FixedSizeByteArray::from(self.0).serialize(serializer)
            }
        }
    }

    impl<'de> Deserialize<'de> for Address {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let s: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
                Address::from_any_str(&s).map_err(Error::custom)
            } else {
                let data: [u8; Self::SIZE] =
                    FixedSizeByteArray::deserialize(deserializer)?.into_inner();
                Ok(Address(data))
            }
        }
    }
}
