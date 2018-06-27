use std::str;
use hex::{FromHex,FromHexError};
use blake2_rfc::blake2b::Blake2b;
use libargon2_sys::argon2d_hash;
use sha2::{Sha256,Digest};
use beserial::{Serialize, Deserialize};

pub trait Hasher: Default {
    type Output: HashOutput + Hash<Self>;

    fn finish(self) -> Self::Output;
    fn write(&mut self, bytes: &[u8]) -> &mut Self;
    fn digest(self, bytes: &[u8]) -> Self::Output;

    fn hash(&mut self, h: &Hash<Self>) -> &mut Self {
        h.hash(self);
        return self;
    }

    fn chain(mut self, h: &Hash<Self>) -> Self {
        self.hash(h);
        return self;
    }
}

pub trait Hash<H: Hasher> {
    fn hash(&self, state: &mut H);
    fn hash_and_finish(&self) -> H::Output {
        let mut h = H::default();
        self.hash(&mut h);
        return h.finish();
    }
}

pub trait HashOutput: PartialEq + Eq {
    type Builder: Hasher;

    fn as_bytes<'a>(&'a self) -> &'a [u8];
}

impl<H> Hash<H::Builder> for H where H: HashOutput {
    fn hash(&self, state: &mut H::Builder) {
        state.write(self.as_bytes());
    }
}

const BLAKE2B_LENGTH : usize = 32;
create_typed_array!(Blake2bHash, u8, BLAKE2B_LENGTH);
add_hex_io_fns_typed_arr!(Blake2bHash, BLAKE2B_LENGTH);
pub struct Blake2bHasher(Blake2b);
impl HashOutput for Blake2bHash {
    type Builder = Blake2bHasher;

    fn as_bytes<'a>(&'a self) -> &[u8] {
        return &self.0;
    }
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
}

impl Argon2dHasher {
    pub fn new(passes: u32, lanes: u32, kib: u32) -> Self {
        return Argon2dHasher { buf: vec![], passes, lanes, kib };
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

impl Hasher for Argon2dHasher {
    type Output = Argon2dHash;

    fn finish(self) -> Argon2dHash {
        return self.hash_bytes(self.buf.as_slice(), NIMIQ_ARGON2_SALT.as_bytes());
    }

    fn write(&mut self, bytes: &[u8]) -> &mut Argon2dHasher {
        self.buf.extend(bytes);
        return self;
    }

    fn digest(self, bytes: &[u8]) -> Argon2dHash {
        return self.hash_bytes(bytes, NIMIQ_ARGON2_SALT.as_bytes());
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

add_hash_trait_arr!([u8; 32]);
add_hash_trait_arr!([u8; 64]);
add_hash_trait_arr!([u8]);
impl<'a, H> Hash<H> for &'a str where H: Hasher {
    fn hash(&self, state: &mut H) {
        state.write(self.as_bytes());
    }
}
