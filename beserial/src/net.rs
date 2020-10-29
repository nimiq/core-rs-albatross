//! This module generates `Serialize` and `Deserialize` implementations for the `std::net`
//! IP address structs and enums.

use crate::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

macro_rules! impl_serialize_ipaddr {
    ($name: ident, $bytes: expr) => {
        impl Serialize for $name {
            fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
                Ok(writer.write(&self.octets())?)
            }

            fn serialized_size(&self) -> usize {
                $bytes
            }
        }

        impl Deserialize for $name {
            fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
                let mut octets: [u8; $bytes] = [0u8; $bytes];
                reader.read_exact(&mut octets)?;
                Ok(Self::from(octets))
            }
        }
    };
}

macro_rules! impl_serialize_sockaddr {
    ($name: ident, $constructor: expr) => {
        impl Serialize for $name {
            fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
                let mut size = 0;
                size += Serialize::serialize(self.ip(), writer)?;
                size += Serialize::serialize(&self.port(), writer)?;
                Ok(size)
            }

            fn serialized_size(&self) -> usize {
                // always 2 bytes for the port
                Serialize::serialized_size(self.ip()) + 2
            }
        }

        impl Deserialize for $name {
            fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
                let ip = Deserialize::deserialize(reader)?;
                let port = Deserialize::deserialize(reader)?;

                Ok($constructor(ip, port))
            }
        }
    };
}

impl_serialize_ipaddr!(Ipv4Addr, 4);
impl_serialize_ipaddr!(Ipv6Addr, 16);

impl Serialize for IpAddr {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 1; // one byte for version
        match self {
            IpAddr::V4(addr) => {
                writer.write_u8(4)?;
                size += Serialize::serialize(addr, writer)?;
            }
            IpAddr::V6(addr) => {
                writer.write_u8(6)?;
                size += Serialize::serialize(addr, writer)?;
            }
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let size = match self {
            IpAddr::V4(addr) => Serialize::serialized_size(addr),
            IpAddr::V6(addr) => Serialize::serialized_size(addr),
        };
        size + 1 // one byte for version
    }
}

impl Deserialize for IpAddr {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        match reader.read_u8()? {
            4 => {
                let ip: Ipv4Addr = Deserialize::deserialize(reader)?;
                Ok(IpAddr::from(ip))
            }
            6 => {
                let ip: Ipv6Addr = Deserialize::deserialize(reader)?;
                Ok(IpAddr::from(ip))
            }
            _ => Err(SerializingError::InvalidEncoding),
        }
    }
}

impl Serialize for SocketAddr {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += Serialize::serialize(&self.ip(), writer)?;
        size += Serialize::serialize(&self.port(), writer)?;
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let size = match self {
            SocketAddr::V4(_) => 4,
            SocketAddr::V6(_) => 16,
        };
        size + 2 // always 2 bytes for the port
    }
}

impl Deserialize for SocketAddr {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ip = Deserialize::deserialize(reader)?;
        let port = Deserialize::deserialize(reader)?;
        Ok(SocketAddr::new(ip, port))
    }
}

impl_serialize_sockaddr!(SocketAddrV4, |ip, port| Self::new(ip, port));
impl_serialize_sockaddr!(SocketAddrV6, |ip, port| Self::new(ip, port, 0, 0));
