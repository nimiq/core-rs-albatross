//! This modules implements `Serialize` and `Deserialize` for some types from [`libp2p`](https://docs.rs/libp2p).

use std::convert::TryFrom;

use libp2p::Multiaddr;
use byteorder::{WriteBytesExt, ReadBytesExt};

use crate::{Serialize, Deserialize, SerializingError, uvar};


impl Serialize for Multiaddr {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let raw: &[u8] = self.as_ref();
        let mut size = 0;

        size += Serialize::serialize(&uvar::from(raw.len() as u64), writer)?;
        writer.write_all(raw)?;
        size += raw.len();

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let raw: &[u8] = self.as_ref();
        let mut size = 0;

        size += Serialize::serialized_size(&uvar::from(raw.len() as u64));
        size += raw.len();

        size
    }
}

impl Deserialize for Multiaddr {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let n: uvar = Deserialize::deserialize(reader)?;
        let n = u64::from(n) as usize;
        let mut buf = Vec::with_capacity(n);
        buf.resize(n, 0);
        reader.read_exact(&mut buf)?;
        Multiaddr::try_from(buf)
            .map_err(|e| SerializingError::InvalidValue)
    }
}
