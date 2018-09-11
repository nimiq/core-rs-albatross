use beserial::{Serialize, Deserialize, ReadBytesExt, WriteBytesExt};
use std::io;

create_typed_array!(IPv4Address, u8, 4);
create_typed_array!(IPv6Address, u8, 16);

pub enum NetAddress {
    IPv4(IPv4Address),
    IPv6(IPv6Address),
    Unspecified,
    Unknown,
}

impl NetAddress {
    pub fn get_type(&self) -> NetAddressType {
        return match self {
            NetAddress::IPv4(_) => NetAddressType::IPv4,
            NetAddress::IPv6(_) => NetAddressType::IPv6,
            NetAddress::Unspecified => NetAddressType::Unspecified,
            NetAddress::Unknown => NetAddressType::Unknown
        };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum NetAddressType {
    IPv4 = 0,
    IPv6 = 1,
    Unspecified = 2,
    Unknown = 3,
}

impl Deserialize for NetAddress {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let ty: NetAddressType = Deserialize::deserialize(reader)?;
        match ty {
            NetAddressType::IPv4 => Ok(NetAddress::IPv4(Deserialize::deserialize(reader)?)),
            NetAddressType::IPv6 => Ok(NetAddress::IPv6(Deserialize::deserialize(reader)?)),
            NetAddressType::Unspecified => Ok(NetAddress::Unspecified),
            NetAddressType::Unknown => Ok(NetAddress::Unknown)
        }
    }
}

impl Serialize for NetAddress {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, io::Error> {
        let mut size = 0;
        size += self.get_type().serialize(writer)?;
        size += match self {
            NetAddress::IPv4(ipv4) => ipv4.serialize(writer)?,
            NetAddress::IPv6(ipv6) => ipv6.serialize(writer)?,
            NetAddress::Unspecified => 0,
            NetAddress::Unknown => 0
        };
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.get_type().serialized_size();
        size += match self {
            NetAddress::IPv4(ipv4) => ipv4.serialized_size(),
            NetAddress::IPv6(ipv6) => ipv6.serialized_size(),
            NetAddress::Unspecified => 0,
            NetAddress::Unknown => 0
        };
        return size;
    }
}
