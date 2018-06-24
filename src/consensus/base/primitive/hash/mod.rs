extern crate hex;

use std::fmt;
use std::str;
use self::hex::{FromHex,FromHexError};

pub trait Hasher {
    type Output;

    fn finish(&self) -> Self::Output;
    fn write(&mut self, bytes: &[u8]);
}

pub trait Hash {
    fn hash<H>(&self, state: &mut H) where H: Hasher;
}

macro_rules! implement_hash {
	($name: ident, $len: expr) => {
		#[repr(C)]
		#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord)]
		pub struct $name([u8; $len]);

		impl $name {
		    pub fn len() -> usize {
				return $len;
			}
		}

		impl fmt::Display for $name {
			fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
				return f.write_str(&hex::encode(&self.0));
			}
		}

		impl From<[u8; $len]> for $name {
			fn from(h: [u8; $len]) -> Self {
				return $name(h);
			}
		}

		impl From<$name> for [u8; $len] {
			fn from(h: $name) -> Self {
				return h.0;
			}
		}

		impl<'a> From<&'a [u8]> for $name {
			fn from(slice: &[u8]) -> Self {
				let mut inner = [0u8; $len];
				inner[..].clone_from_slice(&slice[0..$len]);
				return $name(inner);
			}
		}

		impl str::FromStr for $name {
			type Err = FromHexError;

			fn from_str(s: &str) -> Result<Self, Self::Err> {
				let vec = Vec::from_hex(s)?;
				if vec.len() == $len {
                    return Ok($name::from(&vec[..]));
				} else {
				    return Err(FromHexError::InvalidStringLength);
				}
			}
		}

		impl From<&'static str> for $name {
			fn from(s: &'static str) -> Self {
				return s.parse().unwrap();
			}
		}
	}
}

implement_hash!(Blake2bHash, 32);
implement_hash!(Argon2dHash, 32);
implement_hash!(Sha256Hash, 32);

struct Blake2bHasher;

impl Hasher for Blake2bHasher {
    type Output = Blake2bHash;

    fn finish(&self) -> Blake2bHash {
        unimplemented!()
    }

    fn write(&mut self, bytes: &[u8]) {
        unimplemented!()
    }
}

struct Argon2dHasher;

impl Hasher for Argon2dHasher {
    type Output = Argon2dHash;

    fn finish(&self) -> Argon2dHash {
        unimplemented!()
    }

    fn write(&mut self, bytes: &[u8]) {
        unimplemented!()
    }
}

struct Sha256Hasher;

impl Hasher for Sha256Hasher {
    type Output = Sha256Hash;

    fn finish(&self) -> Sha256Hash {
        unimplemented!()
    }

    fn write(&mut self, bytes: &[u8]) {
        unimplemented!()
    }
}
