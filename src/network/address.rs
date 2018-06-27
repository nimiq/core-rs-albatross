use beserial::{Serialize, SerializeWithLength, Deserialize, DeserializeWithLength, ReadBytesExt, WriteBytesExt};
use std::io;

create_typed_array!(PeerId, u8, 16);

pub enum PeerAddress {
    WssPeerAddress,
    RtcPeerAddress,
    WsPeerAddress,
    DumbPeerAddress
}

impl Serialize for PeerAddress {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, io::Error> {
        unimplemented!()
    }

    fn serialized_size(&self) -> usize {
        unimplemented!()
    }
}

impl Deserialize for PeerAddress {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, io::Error> {
        unimplemented!()
    }
}

pub struct WssPeerAddress {
    services: u32,
    timestamp: u64,
    net_address: NetAddress
}

create_typed_array!(IPv4Address, u8, 4);
create_typed_array!(IPv6Address, u8, 16);

pub enum NetAddress {
    IPv4(IPv4Address),
    IPv6(IPv6Address),
    Unspecified,
    Unknown
}

impl Deserialize for NetAddress {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> io::Result<Self> {
        let ty: u8 = Deserialize::deserialize(reader)?;
        match ty {
            0 => {
                let addr: IPv4Address = Deserialize::deserialize(reader)?;
                return Ok(NetAddress::IPv4(addr));
            },
            1 => {
                let addr: IPv6Address = Deserialize::deserialize(reader)?;
                return Ok(NetAddress::IPv6(addr));
            },
            2 => { return Ok(NetAddress::Unspecified); },
            3 => { return Ok(NetAddress::Unknown); },
            _ => { return Err(io::Error::new(io::ErrorKind::InvalidData, "Non-existing NetAddress type")) }
        };
    }
}

impl Serialize for NetAddress {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, io::Error> {
        unimplemented!()
    }

    fn serialized_size(&self) -> usize {
        unimplemented!()
    }
}
