#[doc(hidden)]
pub extern crate nimiq_serde;

use std::{
    borrow::Cow,
    cmp::Ordering,
    fmt::{Debug, Error, Formatter},
    io,
    io::Write as _,
    str,
};

use blake2_rfc::{blake2b::Blake2b, blake2s::Blake2s};
use byteorder::WriteBytesExt;
use hex::FromHex;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_macros::{add_hex_io_fns_typed_arr, add_serialization_fns_typed_arr, create_typed_array};
use nimiq_mmr::hash::Merge;
use nimiq_serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

pub mod argon2kdf;
pub mod blake2s;
pub mod hmac;
pub mod pbkdf2;
pub mod sha512;

#[macro_export]
macro_rules! add_hash_trait_arr {
    ($t: ty) => {
        impl SerializeContent for $t {
            fn serialize_content<W: io::Write, H>(&self, state: &mut W) -> io::Result<()> {
                state.write_all(&self[..])?;
                Ok(())
            }
        }
    };
}

#[macro_export]
macro_rules! hash_typed_array {
    ($name: ident) => {
        impl ::nimiq_hash::SerializeContent for $name {
            fn serialize_content<W: io::Write, H>(&self, state: &mut W) -> io::Result<()> {
                state.write_all(&self.0[..])?;
                Ok(())
            }
        }
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
        h.serialize_content::<_, Self::Output>(self).unwrap();
        self
    }

    #[must_use]
    fn chain<T: SerializeContent>(mut self, h: &T) -> Self {
        self.hash(h);
        self
    }
}

pub trait SerializeContent {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()>;
}

pub trait Hash {
    fn hash<H: HashOutput>(&self) -> H;
}

impl<T> Hash for T
where
    T: SerializeContent,
{
    fn hash<H: HashOutput>(&self) -> H {
        let mut h = H::Builder::default();
        self.serialize_content::<_, H>(&mut h).unwrap();
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
    + Hash
{
    type Builder: Hasher<Output = Self>;

    fn as_bytes(&self) -> &[u8];
    fn len() -> usize;
}

const BLAKE2B_LENGTH: usize = 32;
create_typed_array!(Blake2bHash, u8, BLAKE2B_LENGTH);
add_hex_io_fns_typed_arr!(Blake2bHash, BLAKE2B_LENGTH);
add_serialization_fns_typed_arr!(Blake2bHash, BLAKE2B_LENGTH);

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

impl SerializeContent for Blake2bHash {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(self.as_bytes())?;
        Ok(())
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

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.update(buf);
        Ok(())
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

impl AsDatabaseBytes for Blake2bHash {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(self.as_bytes())
    }
}

impl FromDatabaseValue for Blake2bHash {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(bytes.into())
    }
}

impl Merge for Blake2bHash {
    /// Hashes just a prefix.
    fn empty(prefix: u64) -> Self {
        let mut hasher = Blake2bHasher::new();
        hasher.write_all(&prefix.to_be_bytes()).unwrap();
        hasher.finish()
    }

    /// Hashes a prefix and two Blake2b hashes together.
    fn merge(&self, other: &Self, prefix: u64) -> Self {
        let mut hasher = Blake2bHasher::new();
        hasher.write_all(&prefix.to_be_bytes()).unwrap();
        self.serialize_to_writer(&mut hasher).unwrap();
        other.serialize_to_writer(&mut hasher).unwrap();
        hasher.finish()
    }
}

// Blake2s

const BLAKE2S_LENGTH: usize = 32;
create_typed_array!(Blake2sHash, u8, BLAKE2S_LENGTH);
add_hex_io_fns_typed_arr!(Blake2sHash, BLAKE2S_LENGTH);
add_serialization_fns_typed_arr!(Blake2sHash, BLAKE2S_LENGTH);
pub struct Blake2sHasher(Blake2s);
impl HashOutput for Blake2sHash {
    type Builder = Blake2sHasher;

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    fn len() -> usize {
        BLAKE2S_LENGTH
    }
}

impl SerializeContent for Blake2sHash {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(self.as_bytes())?;
        Ok(())
    }
}

impl Blake2sHasher {
    pub fn new() -> Self {
        Blake2sHasher(Blake2s::new(BLAKE2S_LENGTH))
    }
}

impl Default for Blake2sHasher {
    fn default() -> Self {
        Blake2sHasher::new()
    }
}

impl io::Write for Blake2sHasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.update(buf);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Hasher for Blake2sHasher {
    type Output = Blake2sHash;

    fn finish(self) -> Blake2sHash {
        let result = self.0.finalize();
        Blake2sHash::from(result.as_bytes())
    }
}

// Argon2d

const ARGON2D_LENGTH: usize = 32;
const NIMIQ_ARGON2_SALT: &str = "nimiqrocks!";
const DEFAULT_ARGON2_COST: u32 = 512;
create_typed_array!(Argon2dHash, u8, ARGON2D_LENGTH);
add_hex_io_fns_typed_arr!(Argon2dHash, ARGON2D_LENGTH);
add_serialization_fns_typed_arr!(Argon2dHash, ARGON2D_LENGTH);
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

impl SerializeContent for Argon2dHash {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(self.as_bytes())?;
        Ok(())
    }
}

impl Argon2dHasher {
    pub fn new(passes: u32, lanes: u32, kib: u32) -> Self {
        let config = argon2::Config {
            time_cost: passes,
            lanes,
            mem_cost: kib,
            hash_length: 32,
            variant: argon2::Variant::Argon2d,
            ..Default::default()
        };
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

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.buf.extend(buf);
        Ok(())
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
add_serialization_fns_typed_arr!(Sha256Hash, SHA256_LENGTH);
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

impl SerializeContent for Sha256Hash {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(self.as_bytes())?;
        Ok(())
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
        self.0.update(buf);
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.update(buf);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Hasher for Sha256Hasher {
    type Output = Sha256Hash;

    fn finish(self) -> Sha256Hash {
        let result = self.0.finalize();
        Sha256Hash::from(result.as_slice())
    }
}

add_hash_trait_arr!([u8; 32]);
add_hash_trait_arr!([u8; 64]);
add_hash_trait_arr!([u8]);
add_hash_trait_arr!(Vec<u8>);

impl SerializeContent for str {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(self.as_bytes())?;
        Ok(())
    }
}

impl SerializeContent for String {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(self.as_bytes())?;
        Ok(())
    }
}

impl<'a, T: SerializeContent + ?Sized> SerializeContent for &'a T {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        (**self).serialize_content::<W, H>(writer)
    }
}

impl<T: SerializeContent> SerializeContent for Option<T> {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        if let Some(inner) = self {
            writer.write_u8(1)?;
            inner.serialize_content::<W, H>(writer)?;
            Ok(())
        } else {
            writer.write_u8(0)?;
            Ok(())
        }
    }
}

impl<T: SerializeContent, U: SerializeContent> SerializeContent for (T, U) {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        self.0.serialize_content::<W, H>(writer)?;
        self.1.serialize_content::<W, H>(writer)?;
        Ok(())
    }
}
