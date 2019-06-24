use crate::PublicKey;
use crate::hash::{Blake2bHash, Blake2bHasher, Hasher, SerializeContent};
use std::convert::From;
use std::char;
use std::io;
use std::iter::Iterator;
use std::str::FromStr;
use hex::FromHex;

create_typed_array!(Address, u8, 20);
hash_typed_array!(Address);
add_hex_io_fns_typed_arr!(Address, Address::SIZE);

#[derive(Debug, Fail)]
pub enum AddressParseError {
    // User-friendly
    #[fail(display = "Wrong country code")]
    WrongCountryCode,
    #[fail(display = "Wrong length")]
    WrongLength,
    #[fail(display = "Invalid checksum")]
    InvalidChecksum,
    // from Hash
    #[fail(display = "Invalid hash")]
    InvalidHash,
    // trying both
    #[fail(display = "Unknown format")]
    UnknownFormat
}

impl Address {
    const CCODE: &'static str = "NQ";
    const NIMIQ_ALPHABET: &'static str = "0123456789ABCDEFGHJKLMNPQRSTUVXY";

    pub fn from_user_friendly_address(friendly_addr: &str) -> Result<Address, AddressParseError> {
        let friendly_addr_wospace = str::replace(friendly_addr, " ", "");

        if friendly_addr_wospace.len() != 36 {
            return Err(AddressParseError::WrongLength);
        }
        if friendly_addr_wospace[0..2].to_uppercase() != Address::CCODE {
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

        let b_vec = encoding.decode(friendly_addr_wospace[4..].as_bytes()).unwrap();
        let mut b = [0; 20];
        b.copy_from_slice(&b_vec[..b_vec.len()]);
        Ok(Address(b))
    }

    pub fn to_user_friendly_address(&self) -> String {
        let mut spec = data_encoding::Specification::new();
        spec.symbols.push_str(Address::NIMIQ_ALPHABET);
        let encoding = spec.encoding().unwrap();

        let base32 = encoding.encode(&self.0);
        let check_string = "00".to_string() + &(98 - Address::iban_check(&(base32.clone() + Address::CCODE + "00"))).to_string();
        let check = check_string.chars().skip(check_string.len() - 2).take(2).collect::<String>();
        let friendly_addr = Address::CCODE.to_string() + &check + &base32;
        let mut friendly_spaces = String::with_capacity(36+8);
        for i in 0..9 {
            friendly_spaces.push_str(&friendly_addr.chars().skip(4*i).take(4).collect::<String>());
            if i != 8 {
                friendly_spaces.push_str(" ");
            }
        }
        friendly_spaces
    }

    fn iban_check(s: &str) -> u32 {
        let mut num = String::with_capacity(s.len() * 2);
        for c in s.to_uppercase().chars() {
            let code = c as u32;
            if code >= 48 && code <=57 {
                num.push(char::from_u32(code).unwrap());
            } else {
                num.push_str(&(code - 55).to_string());
            }
        }
        let mut tmp: String = "".to_string();
        for i in 0..(f32::ceil(num.len() as f32 / 6.0) as usize) {
            let num_substr = num.chars().skip(i*6).take(6).collect::<String>();
            let num_tmp_sub = tmp.clone() + &num_substr;
            tmp = (num_tmp_sub.parse::<u32>().unwrap() % 97).to_string();
        }

        tmp.parse::<u32>().unwrap()
    }

    pub fn from_any_str(s: &str) -> Result<Address, AddressParseError> {
        Address::from_user_friendly_address(&String::from(s))
            .or_else(|_| Address::from_str(s))
            .map_err(|_|AddressParseError::UnknownFormat)
    }
}

impl From<Blake2bHash> for Address {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        Address::from(&hash_arr[0..Address::len()])
    }
}

impl<'a> From<&'a PublicKey> for Address {
    fn from(public_key: &'a PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        Address::from(hash)
    }
}
