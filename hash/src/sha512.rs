use super::*;

// Since there are no trait implementations for [u8; 64], we have to implement everything on our own.
pub(super) const SHA512_LENGTH: usize = 64;

#[repr(C)]
pub struct Sha512Hash([u8; SHA512_LENGTH]);

impl<'a> From<&'a [u8]> for Sha512Hash {
    fn from(slice: &'a [u8]) -> Self {
        assert_eq!(
            slice.len(),
            SHA512_LENGTH,
            "Tried to create instance with slice of wrong length"
        );
        let mut a = [0 as u8; SHA512_LENGTH];
        a.clone_from_slice(&slice[0..SHA512_LENGTH]);
        Sha512Hash(a)
    }
}

impl ::beserial::Deserialize for Sha512Hash {
    fn deserialize<R: ::beserial::ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, ::beserial::SerializingError> {
        let mut a = [0 as u8; SHA512_LENGTH];
        reader.read_exact(&mut a[..])?;
        Ok(Sha512Hash(a))
    }
}

impl ::beserial::Serialize for Sha512Hash {
    fn serialize<W: ::beserial::WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, ::beserial::SerializingError> {
        writer.write_all(&self.0)?;
        Ok(SHA512_LENGTH)
    }

    fn serialized_size(&self) -> usize {
        SHA512_LENGTH
    }
}

impl From<[u8; SHA512_LENGTH]> for Sha512Hash {
    fn from(arr: [u8; SHA512_LENGTH]) -> Self {
        Sha512Hash(arr)
    }
}

impl From<Sha512Hash> for [u8; SHA512_LENGTH] {
    fn from(i: Sha512Hash) -> [u8; SHA512_LENGTH] {
        i.0
    }
}

impl Default for Sha512Hash {
    fn default() -> Self {
        Sha512Hash([
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
            u8::default(),
        ])
    }
}

impl Clone for Sha512Hash {
    fn clone(&self) -> Self {
        let mut hash = Sha512Hash::default();
        hash.0.copy_from_slice(&self.0[..]);
        hash
    }
}

impl PartialEq for Sha512Hash {
    fn eq(&self, other: &Self) -> bool {
        self.0[..] == other.0[..]
    }
}

impl Eq for Sha512Hash {}

impl PartialOrd for Sha512Hash {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0[..].partial_cmp(&other.0[..])
    }
}

impl Ord for Sha512Hash {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0[..].cmp(&other.0[..])
    }
}

impl Debug for Sha512Hash {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        f.write_str(&::hex::encode(&self.0[..]))
    }
}

impl ::std::hash::Hash for Sha512Hash {
    fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
        ::std::hash::Hash::hash(&self.0[..], state);
    }
}

impl Sha512Hash {
    pub const SIZE: usize = SHA512_LENGTH;
    #[inline]
    pub fn len() -> usize {
        SHA512_LENGTH
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl ::std::fmt::Display for Sha512Hash {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        f.write_str(&::hex::encode(&self.0[..]))
    }
}

impl ::std::str::FromStr for Sha512Hash {
    type Err = ::hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let vec = Vec::from_hex(s)?;
        if vec.len() == SHA512_LENGTH {
            Ok(Sha512Hash::from(&vec[..]))
        } else {
            Err(::hex::FromHexError::InvalidStringLength)
        }
    }
}

impl From<&'static str> for Sha512Hash {
    fn from(s: &'static str) -> Self {
        s.parse().unwrap()
    }
}

impl Sha512Hash {
    #[inline]
    pub fn block_size() -> usize {
        128
    }
}

pub struct Sha512Hasher(Sha512);

impl HashOutput for Sha512Hash {
    type Builder = Sha512Hasher;

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    fn len() -> usize {
        SHA512_LENGTH
    }
}

impl Sha512Hasher {
    pub fn new() -> Self {
        Sha512Hasher(Sha512::default())
    }
}

impl Default for Sha512Hasher {
    fn default() -> Self {
        Sha512Hasher::new()
    }
}

impl io::Write for Sha512Hasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.input(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Hasher for Sha512Hasher {
    type Output = Sha512Hash;

    fn finish(self) -> Sha512Hash {
        let result = self.0.result();
        Sha512Hash::from(result.as_slice())
    }
}
