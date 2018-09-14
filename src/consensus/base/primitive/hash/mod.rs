pub mod hmac;
pub mod pbkdf2;
pub mod sha512;

use std::str;
use hex::{FromHex};
use blake2_rfc::blake2b::Blake2b;
use libargon2_sys::argon2d_hash;
use sha2::{Sha256, Sha512, Digest};
use beserial::{Serialize, Deserialize};
use std::io;
use std::fmt::Debug;
use std::cmp::Ordering;
use std::fmt::Error;
use std::fmt::Formatter;
pub use self::sha512::*;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum HashAlgorithm {
    Blake2b = 1,
    Argon2d = 2,
    Sha256 = 3,
    Sha512 = 4
}

pub trait Hasher: Default + io::Write {
    type Output: HashOutput;

    fn finish(self) -> Self::Output;
    fn digest(mut self, bytes: &[u8]) -> Self::Output {
        self.write(bytes).unwrap();
        return self.finish();
    }

    fn hash<T: SerializeContent>(&mut self, h: &T) -> &mut Self {
        h.serialize_content(self).unwrap();
        return self;
    }

    fn chain<T: SerializeContent>(mut self, h: &T) -> Self {
        self.hash(h);
        return self;
    }
}

pub trait SerializeContent {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize>;
}

pub trait Hash: SerializeContent {
    fn hash<H: HashOutput>(&self) -> H  {
        let mut h = H::Builder::default();
        self.serialize_content(&mut h).unwrap();
        return h.finish();
    }
}

pub trait HashOutput: PartialEq + Eq + Clone + Serialize + Deserialize + Sized + SerializeContent + Debug {
    type Builder: Hasher<Output=Self>;

    fn as_bytes<'a>(&'a self) -> &'a [u8];
    fn len() -> usize;
}

impl<H> SerializeContent for H where H: HashOutput {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write(self.as_bytes())?;
        return Ok(Self::len());
    }
}

impl<H> Hash for H where H: HashOutput {}

const BLAKE2B_LENGTH : usize = 32;
create_typed_array!(Blake2bHash, u8, BLAKE2B_LENGTH);
add_hex_io_fns_typed_arr!(Blake2bHash, BLAKE2B_LENGTH);
pub struct Blake2bHasher(Blake2b);
impl HashOutput for Blake2bHash {
    type Builder = Blake2bHasher;

    fn as_bytes<'a>(&'a self) -> &[u8] {
        return &self.0;
    }
    fn len() -> usize { BLAKE2B_LENGTH }
}

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

impl io::Write for Blake2bHasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        return Ok(buf.len());
    }

    fn flush(&mut self) -> io::Result<()> {
        return Ok(());
    }
}

impl Hasher for Blake2bHasher {
    type Output = Blake2bHash;

    fn finish(self) -> Blake2bHash {
        let result = self.0.finalize();
        return Blake2bHash::from(result.as_bytes());
    }
}

const ARGON2D_LENGTH : usize = 32;
const NIMIQ_ARGON2_SALT: &'static str = "nimiqrocks!";
const DEFAULT_ARGON2_COST : u32 = 512;
create_typed_array!(Argon2dHash, u8, ARGON2D_LENGTH);
add_hex_io_fns_typed_arr!(Argon2dHash, ARGON2D_LENGTH);
pub struct Argon2dHasher {
    buf: Vec<u8>,
    passes: u32,
    lanes: u32,
    kib: u32
}
impl HashOutput for Argon2dHash {
    type Builder = Argon2dHasher;

    fn as_bytes<'a>(&'a self) -> &[u8] {
        return &self.0;
    }
    fn len() -> usize { ARGON2D_LENGTH }
}

impl Argon2dHasher {
    pub fn new(passes: u32, lanes: u32, kib: u32) -> Self {
        return Argon2dHasher { buf: Vec::new(), passes, lanes, kib };
    }

    fn hash_bytes(&self, bytes: &[u8], salt: &[u8]) -> Argon2dHash {
        let mut out = [0u8; ARGON2D_LENGTH];
        argon2d_hash(self.passes, self.kib, self.lanes,bytes, salt, &mut out, 0);
        return Argon2dHash::from(out);
    }
}

impl Default for Argon2dHasher {
    fn default() -> Self {
        return Argon2dHasher::new(1, 1, DEFAULT_ARGON2_COST);
    }
}

impl io::Write for Argon2dHasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend(buf);
        return Ok(buf.len());
    }

    fn flush(&mut self) -> Result<(), io::Error> {
        return Ok(());
    }
}

impl Hasher for Argon2dHasher {
    type Output = Argon2dHash;

    fn finish(self) -> Argon2dHash {
        return self.hash_bytes(self.buf.as_slice(), NIMIQ_ARGON2_SALT.as_bytes());
    }
}

const SHA256_LENGTH : usize = 32;
create_typed_array!(Sha256Hash, u8, SHA256_LENGTH);
add_hex_io_fns_typed_arr!(Sha256Hash, SHA256_LENGTH);
pub struct Sha256Hasher(Sha256);
impl HashOutput for Sha256Hash {
    type Builder = Sha256Hasher;

    fn as_bytes<'a>(&'a self) -> &[u8] {
        return &self.0;
    }
    fn len() -> usize { SHA256_LENGTH }
}

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

impl io::Write for Sha256Hasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.input(buf);
        return Ok(buf.len());
    }

    fn flush(&mut self) -> io::Result<()> {
        return Ok(());
    }
}

impl Hasher for Sha256Hasher {
    type Output = Sha256Hash;

    fn finish(self) -> Sha256Hash {
        let result = self.0.result();
        return Sha256Hash::from(result.as_slice());
    }
}

add_hash_trait_arr!([u8; 32]);
add_hash_trait_arr!([u8; 64]);
add_hash_trait_arr!([u8]);
add_hash_trait_arr!(Vec<u8>);
impl<'a> SerializeContent for &'a [u8] {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write(self)?;
        Ok(self.len())
    }
}

impl<'a> SerializeContent for &'a str {
    fn serialize_content<W: io::Write>(&self, state: &mut W) -> io::Result<usize> {
        state.write(self.as_bytes())?;
        Ok(self.len())
    }
}

impl<'a> Hash for &'a str {}
