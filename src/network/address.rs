use beserial::{Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength, WriteBytesExt};
use consensus::base::primitive::crypto::{PublicKey, Signature};
use consensus::base::primitive::hash::{Blake2bHash, Blake2bHasher, Hasher};
use network::Protocol;
use std::io;
use std::fmt;
use std::vec::Vec;

create_typed_array!(PeerId, u8, 16);

impl From<Blake2bHash> for PeerId {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        return PeerId::from(&hash_arr[0..PeerId::len()]);
    }
}

impl<'a> From<&'a PublicKey> for PeerId {
    fn from(public_key: &'a PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        return PeerId::from(hash);
    }
}

pub enum PeerAddressType {
    Dumb,
    Ws(String, u16),
    Wss(String, u16),
    Rtc,
}

impl PeerAddressType {
    pub fn get_protocol(&self) -> Protocol {
        match self {
            PeerAddressType::Dumb => Protocol::Dumb,
            PeerAddressType::Ws(_, _) => Protocol::Ws,
            PeerAddressType::Wss(_, _) => Protocol::Wss,
            PeerAddressType::Rtc => Protocol::Rtc
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct PeerAddress {
    ty: PeerAddressType,
    services: u32,
    timestamp: u64,
    net_address: NetAddress,
    public_key: Option<PublicKey>,
    distance: u8,
    signature: Signature,
}

impl PeerAddress {
    pub fn verify_signature(&self) -> bool {
        self.public_key.unwrap().verify(&self.signature, self.get_signature_data().as_slice())
    }

    pub fn as_uri(&self) -> String {
        let peer_id: String = match self.public_key {
            Some(public_key) => String::from(::hex::encode(&PeerId::from(&public_key).0)),
            None => String::from("")
        };
        match self.ty {
            PeerAddressType::Dumb => format!("dumb:///{}", peer_id),
            PeerAddressType::Ws(ref host, ref port) => format!("ws:///{}:{}/{}", host, port, peer_id),
            PeerAddressType::Wss(ref host, ref port) => format!("wss:///{}:{}/{}", host, port, peer_id),
            PeerAddressType::Rtc => format!("rtc:///{}", peer_id)
        }
    }

    pub fn get_signature_data(&self) -> Vec<u8> {
        let mut res: Vec<u8> = (self.ty.get_protocol() as u8).serialize_to_vec();
        res.append(&mut self.services.serialize_to_vec());
        res.append(&mut self.timestamp.serialize_to_vec());

        match &self.ty {
            PeerAddressType::Ws(host, port) | PeerAddressType::Wss(host, port) => {
                res.append(&mut host.serialize_to_vec::<u16>());
                res.append(&mut port.serialize_to_vec());
            }
            _ => {}
        };

        return res;
    }
}

impl Deserialize for PeerAddressType {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, io::Error> {
        let protocol: Protocol = Deserialize::deserialize(reader)?;
        match protocol {
            Protocol::Dumb => Ok(PeerAddressType::Dumb),
            Protocol::Ws => Ok(PeerAddressType::Ws(DeserializeWithLength::deserialize::<u16, R>(reader)?, Deserialize::deserialize(reader)?)),
            Protocol::Wss => Ok(PeerAddressType::Wss(DeserializeWithLength::deserialize::<u16, R>(reader)?, Deserialize::deserialize(reader)?)),
            Protocol::Rtc => Ok(PeerAddressType::Rtc)
        }
    }
}

impl Serialize for PeerAddressType {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, io::Error> {
        Ok(match self {
            PeerAddressType::Dumb => Protocol::Dumb.serialize(writer)?,
            PeerAddressType::Ws(host, port) => Protocol::Ws.serialize(writer)? + host.serialize::<u16, W>(writer)? + port.serialize(writer)?,
            PeerAddressType::Wss(host, port) => Protocol::Wss.serialize(writer)? + host.serialize::<u16, W>(writer)? + port.serialize(writer)?,
            PeerAddressType::Rtc => Protocol::Rtc.serialize(writer)?
        })
    }

    fn serialized_size(&self) -> usize {
        Protocol::Dumb.serialized_size() + match self {
            PeerAddressType::Ws(host, port) => host.serialized_size::<u16>() + port.serialized_size(),
            PeerAddressType::Wss(host, port) => host.serialized_size::<u16>() + port.serialized_size(),
            _ => 0
        }
    }
}

pub struct WssPeerAddress {
    services: u32,
    timestamp: u64,
    net_address: NetAddress,
}

create_typed_array!(IPv4Address, u8, 4);
create_typed_array!(IPv6Address, u8, 16);

pub enum NetAddress {
    IPv4(IPv4Address),
    IPv6(IPv6Address),
    Unspecified,
    Unknown,
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
        unimplemented!()
    }

    fn serialized_size(&self) -> usize {
        unimplemented!()
    }
}
