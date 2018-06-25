extern crate hex;
extern crate blake2_rfc;
extern crate argon2rs;
extern crate sha2;

use std::fmt;
use std::str;
use self::hex::{FromHex,FromHexError};
use self::blake2_rfc::blake2b::Blake2b;
use self::argon2rs::Argon2;
use self::sha2::{Sha256,Digest};

pub trait Hasher {
    type Output;

    fn finish(self) -> Self::Output;
    fn write(&mut self, bytes: &[u8]) -> &mut Self;
    fn digest(self, bytes: &[u8]) -> Self::Output;
}

pub trait Hash {
    fn hash<H>(&self, state: &mut H) where H: Hasher;
}

macro_rules! implement_hash {
	($name: ident, $len: expr) => {
		#[repr(C)]
		#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug)]
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

const BLAKE2B_LENGTH : usize = 32;
implement_hash!(Blake2bHash, BLAKE2B_LENGTH);
pub struct Blake2bHasher(Blake2b);

impl Blake2bHasher {
    pub fn new() -> Self {
        return Blake2bHasher(Blake2b::new(BLAKE2B_LENGTH));
    }
}

impl Default for Blake2bHasher {
    fn default() -> Self {
        return Blake2bHasher::new();
    }
}

impl Hasher for Blake2bHasher {
    type Output = Blake2bHash;

    fn finish(self) -> Blake2bHash {
        let result = self.0.finalize();
        return Blake2bHash::from(result.as_bytes());
    }

    fn write(&mut self, bytes: &[u8]) -> &mut Blake2bHasher {
        self.0.update(bytes);
        return self;
    }

    fn digest(mut self, bytes: &[u8]) -> Blake2bHash {
        self.write(bytes);
        return self.finish();
    }
}

const ARGON2D_LENGTH : usize = 32;
const NIMIQ_ARGON2_SALT: &'static str = "nimiqrocks!";
const DEFAULT_ARGON2_COST : u32 = 512;
implement_hash!(Argon2dHash, ARGON2D_LENGTH);
pub struct Argon2dHasher(Argon2, Vec<u8>);
pub type Argon2dParamErr = argon2rs::ParamErr;

impl Argon2dHasher {
    pub fn new(passes: u32, lanes: u32, kib: u32) -> Result<Self, Argon2dParamErr> {
        let mut h = Argon2::new(passes, lanes, kib, argon2rs::Variant::Argon2d)?;
        return Ok(Argon2dHasher(h, vec![]));
    }

    fn hash(&self, bytes: &[u8]) -> Argon2dHash {
        let mut out = [0u8; ARGON2D_LENGTH];
        self.0.hash(&mut out, bytes, NIMIQ_ARGON2_SALT.as_bytes(), &[], &[]);
        return Argon2dHash::from(out);
    }
}

impl Default for Argon2dHasher {
    fn default() -> Self {
        return Argon2dHasher::new(1, 1, DEFAULT_ARGON2_COST).unwrap();
    }
}

impl Hasher for Argon2dHasher {
    type Output = Argon2dHash;

    fn finish(self) -> Argon2dHash {
        return self.hash(self.1.as_slice());
    }

    fn write(&mut self, bytes: &[u8]) -> &mut Argon2dHasher {
        self.1.extend(bytes);
        return self;
    }

    fn digest(self, bytes: &[u8]) -> Argon2dHash {
        return self.hash(bytes);
    }
}

const SHA256_LENGTH : usize = 32;
implement_hash!(Sha256Hash, SHA256_LENGTH);
pub struct Sha256Hasher(Sha256);

impl Sha256Hasher {
    pub fn new() -> Self {
        return Sha256Hasher(Sha256::default());
    }
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        return Sha256Hasher::new();
    }
}

impl Hasher for Sha256Hasher {
    type Output = Sha256Hash;

    fn finish(self) -> Sha256Hash {
        let result = self.0.result();
        return Sha256Hash::from(result.as_slice());
    }

    fn write(&mut self, bytes: &[u8]) -> &mut Sha256Hasher {
        self.0.input(bytes);
        return self;
    }

    fn digest(mut self, bytes: &[u8]) -> Sha256Hash {
        self.write(bytes);
        return self.finish();
    }
}
