use std::{borrow::Cow, fmt, fmt::Write as _, io, iter::Iterator, str::FromStr, sync::LazyLock};

use hex::FromHex;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes};
use nimiq_hash::{hash_typed_array, Blake2bHash, Blake2bHasher, Hasher};
use nimiq_macros::create_typed_array;
use thiserror::Error;

use crate::{key_pair::KeyPair, ES256PublicKey, Ed25519PublicKey, PublicKey};

const ADDRESS_LEN: usize = 20;
create_typed_array!(Address, u8, ADDRESS_LEN);
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

static ENCODING: LazyLock<data_encoding::Encoding> = LazyLock::new(|| {
    data_encoding::Specification {
        symbols: Address::NIMIQ_ALPHABET.into(),
        ..Default::default()
    }
    .encoding()
    .unwrap()
});

impl Address {
    const CCODE: &'static str = "NQ";
    const NIMIQ_ALPHABET: &'static str = "0123456789ABCDEFGHJKLMNPQRSTUVXY";

    /// The lexicographically first address.
    pub const START_ADDRESS: Address = Address([0x00; ADDRESS_LEN]);
    /// The lexicographically last address.
    pub const END_ADDRESS: Address = Address([0xff; ADDRESS_LEN]);

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_user_friendly_address(friendly_addr: &str) -> Result<Address, AddressParseError> {
        let friendly_addr = friendly_addr.replace(" ", "");

        if friendly_addr.chars().count() != 36 {
            return Err(AddressParseError::WrongLength);
        }
        if !friendly_addr.starts_with(Address::CCODE) {
            return Err(AddressParseError::WrongCountryCode);
        }
        if Address::iban_checksum(&friendly_addr)? != 1 {
            return Err(AddressParseError::InvalidChecksum);
        }
        // Doesn't make sense to avoid the panic here, it would be an indicator of a bug.
        #[allow(clippy::sliced_string_as_bytes)]
        let b_vec = ENCODING
            .decode(friendly_addr[4..].as_bytes())
            .map_err(|_| AddressParseError::UnknownFormat)?;
        let mut b = [0; ADDRESS_LEN];
        b.copy_from_slice(&b_vec);
        Ok(Address(b))
    }

    pub fn to_user_friendly_address(&self) -> String {
        let base32 = ENCODING.encode(&self.0);
        let checksum =
            98 - Address::iban_checksum(&format!("{}00{}", Address::CCODE, base32)).unwrap();
        let mut friendly = format!("{}{:02}{}", Address::CCODE, checksum, base32);
        for i in (1..9).rev() {
            friendly.insert(4 * i, ' ');
        }
        friendly
    }

    fn iban_checksum(iban: &str) -> Result<u32, AddressParseError> {
        fn iban_to_num(num: &mut String, iban_part: &str) -> Result<(), AddressParseError> {
            for c in iban_part.chars() {
                match c {
                    '0'..='9' => num.push(c),
                    'a'..='z' => write!(num, "{}", c as u32 - 'a' as u32 + 10).unwrap(),
                    'A'..='Z' => write!(num, "{}", c as u32 - 'A' as u32 + 10).unwrap(),
                    _ => return Err(AddressParseError::UnknownFormat),
                }
            }
            Ok(())
        }
        let char4_idx = iban
            .char_indices()
            .nth(4)
            .ok_or(AddressParseError::WrongLength)?
            .0;
        let mut num = String::with_capacity(iban.len() * 2);
        iban_to_num(&mut num, &iban[char4_idx..])?;
        iban_to_num(&mut num, &iban[..char4_idx])?;
        let mut checksum = 0;
        for digit_ascii in num.bytes() {
            let digit = u32::from(digit_ascii - b'0');
            checksum = (checksum * 10 + digit) % 97;
        }
        Ok(checksum)
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
    fn as_key_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(self.as_bytes())
    }

    const FIXED_SIZE: Option<usize> = Some(ADDRESS_LEN);
}

impl FromDatabaseBytes for Address {
    fn from_key_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized,
    {
        bytes.into()
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
                // No need to fuzz, it just delegates to `FixedSizeByteArray`.
                let data: [u8; Self::SIZE] =
                    FixedSizeByteArray::deserialize(deserializer)?.into_inner();
                Ok(Address(data))
            }
        }
    }
}
