pub mod argon2kdf;
pub mod hmac;
pub mod pbkdf2;
pub mod sha512;

use beserial::{Deserialize, Serialize};
use blake2_rfc::blake2b::Blake2b;
use hex::FromHex;
use nimiq_macros::{add_hex_io_fns_typed_arr, create_typed_array};
use sha2::{Digest, Sha256, Sha512};

use std::cmp::Ordering;
use std::fmt::{Debug, Error, Formatter};
use std::io;
use std::str;

pub use self::sha512::*;

#[macro_export]
macro_rules! add_hash_trait_arr {
    ($t: ty) => {
        impl SerializeContent for $t {
            fn serialize_content<W: io::Write>(&self, state: &mut W) -> io::Result<usize> {
                state.write_all(&self[..])?;
                return Ok(self.len());
            }
        }

        impl Hash for $t {}
    };
}

#[macro_export]
macro_rules! hash_typed_array {
    ($name: ident) => {
        impl ::nimiq_hash::SerializeContent for $name {
            fn serialize_content<W: io::Write>(&self, state: &mut W) -> io::Result<usize> {
                state.write_all(&self.0[..])?;
                return Ok(Self::SIZE);
            }
        }

        impl ::nimiq_hash::Hash for $name {}
    };
}

pub trait Hasher: Default + io::Write {
    type Output: HashOutput;

    fn finish(self) -> Self::Output;
    fn digest(mut self, bytes: &[u8]) -> Self::Output {
        self.write_all(bytes).unwrap();
        self.finish()
    }

    fn hash<T: SerializeContent>(&mut self, h: &T) -> &mut Self {
        h.serialize_content(self).unwrap();
        self
    }

    fn chain<T: SerializeContent>(mut self, h: &T) -> Self {
        self.hash(h);
        self
    }
}

pub trait SerializeContent {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize>;
}

pub trait Hash: SerializeContent {
    fn hash<H: HashOutput>(&self) -> H {
        let mut h = H::Builder::default();
        self.serialize_content(&mut h).unwrap();
        h.finish()
    }
}

pub trait HashOutput:
    PartialEq
    + Eq
    + Clone
    + Serialize
    + Deserialize
    + Sized
    + SerializeContent
    + Debug
    + std::hash::Hash
{
    type Builder: Hasher<Output = Self>;

    fn as_bytes(&self) -> &[u8];
    fn len() -> usize;
}

impl<H> SerializeContent for H
where
    H: HashOutput,
{
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(self.as_bytes())?;
        Ok(Self::len())
    }
}

// Blake2b

const BLAKE2B_LENGTH: usize = 32;
create_typed_array!(Blake2bHash, u8, BLAKE2B_LENGTH);
add_hex_io_fns_typed_arr!(Blake2bHash, BLAKE2B_LENGTH);
pub struct Blake2bHasher(Blake2b);
impl HashOutput for Blake2bHash {
    type Builder = Blake2bHasher;

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    fn len() -> usize {
        BLAKE2B_LENGTH
    }
}

impl Blake2bHasher {
    pub fn new() -> Self {
        Blake2bHasher(Blake2b::new(BLAKE2B_LENGTH))
    }
}

impl Default for Blake2bHasher {
    fn default() -> Self {
        Blake2bHasher::new()
    }
}

impl io::Write for Blake2bHasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Hasher for Blake2bHasher {
    type Output = Blake2bHash;

    fn finish(self) -> Blake2bHash {
        let result = self.0.finalize();
        Blake2bHash::from(result.as_bytes())
    }
}

// Argon2d

const ARGON2D_LENGTH: usize = 32;
const NIMIQ_ARGON2_SALT: &str = "nimiqrocks!";
const DEFAULT_ARGON2_COST: u32 = 512;
create_typed_array!(Argon2dHash, u8, ARGON2D_LENGTH);
add_hex_io_fns_typed_arr!(Argon2dHash, ARGON2D_LENGTH);
pub struct Argon2dHasher {
    buf: Vec<u8>,
    config: argon2::Config<'static>,
}
impl HashOutput for Argon2dHash {
    type Builder = Argon2dHasher;

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    fn len() -> usize {
        ARGON2D_LENGTH
    }
}

impl Argon2dHasher {
    pub fn new(passes: u32, lanes: u32, kib: u32) -> Self {
        let mut config = argon2::Config::default();
        config.time_cost = passes;
        config.lanes = lanes;
        config.mem_cost = kib;
        config.hash_length = 32;
        config.variant = argon2::Variant::Argon2d;
        Argon2dHasher {
            buf: Vec::new(),
            config,
        }
    }

    fn hash_bytes(&self, bytes: &[u8], salt: &[u8]) -> Argon2dHash {
        let hash = argon2::hash_raw(bytes, salt, &self.config).expect("Argon2 hashing failed");
        let mut out = [0u8; ARGON2D_LENGTH];
        out.copy_from_slice(&hash);
        Argon2dHash::from(out)
    }
}

impl Default for Argon2dHasher {
    fn default() -> Self {
        Argon2dHasher::new(1, 1, DEFAULT_ARGON2_COST)
    }
}

impl io::Write for Argon2dHasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), io::Error> {
        Ok(())
    }
}

impl Hasher for Argon2dHasher {
    type Output = Argon2dHash;

    fn finish(self) -> Argon2dHash {
        self.hash_bytes(self.buf.as_slice(), NIMIQ_ARGON2_SALT.as_bytes())
    }
}

// SHA256

const SHA256_LENGTH: usize = 32;
create_typed_array!(Sha256Hash, u8, SHA256_LENGTH);
add_hex_io_fns_typed_arr!(Sha256Hash, SHA256_LENGTH);
pub struct Sha256Hasher(Sha256);
impl HashOutput for Sha256Hash {
    type Builder = Sha256Hasher;

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    fn len() -> usize {
        SHA256_LENGTH
    }
}

impl Sha256Hasher {
    pub fn new() -> Self {
        Sha256Hasher(Sha256::default())
    }
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        Sha256Hasher::new()
    }
}

impl io::Write for Sha256Hasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.input(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Hasher for Sha256Hasher {
    type Output = Sha256Hash;

    fn finish(self) -> Sha256Hash {
        let result = self.0.result();
        Sha256Hash::from(result.as_slice())
    }
}

add_hash_trait_arr!([u8; 32]);
add_hash_trait_arr!([u8; 64]);
add_hash_trait_arr!([u8]);
add_hash_trait_arr!(Vec<u8>);
impl<'a> SerializeContent for &'a [u8] {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(self)?;
        Ok(self.len())
    }
}
impl<'a> Hash for &'a [u8] {}

impl<'a> SerializeContent for &'a str {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let b = self.as_bytes();
        writer.write_all(b)?;
        Ok(b.len())
    }
}
impl<'a> Hash for &'a str {}

impl SerializeContent for String {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        let b = self.as_bytes();
        writer.write_all(b)?;
        Ok(b.len())
    }
}
impl Hash for String {}
