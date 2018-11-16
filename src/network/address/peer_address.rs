use beserial::{Serialize, SerializingError, SerializeWithLength, Deserialize, DeserializeWithLength, ReadBytesExt, WriteBytesExt};
use crate::consensus::base::primitive::crypto::{PublicKey, Signature};
use crate::network;
use crate::network::Protocol;
use crate::network::address::{NetAddress, PeerId};
use crate::utils::services::ServiceFlags;
use crate::utils::systemtime_to_timestamp;
use std::vec::Vec;
use std::hash::Hash;
use std::hash::Hasher;
use std::fmt;
use std::time::SystemTime;
use std::time::Duration;
use std::time::UNIX_EPOCH;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum PeerAddressType {
    Dumb,
    Ws(String, u16),
    Wss(String, u16),
    Rtc,
}

impl PeerAddressType {
    pub fn protocol(&self) -> Protocol {
        match self {
            PeerAddressType::Dumb => Protocol::Dumb,
            PeerAddressType::Ws(_, _) => Protocol::Ws,
            PeerAddressType::Wss(_, _) => Protocol::Wss,
            PeerAddressType::Rtc => Protocol::Rtc
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerAddress {
    pub ty: PeerAddressType,
    pub services: ServiceFlags,
    pub timestamp: u64,
    pub net_address: NetAddress,
    pub public_key: PublicKey,
    pub distance: u8,
    pub signature: Option<Signature>,
    pub peer_id: PeerId,
}

impl Serialize for PeerAddress {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.ty.protocol().serialize(writer)?;
        size += self.services.serialize(writer)?;
        size += self.timestamp.serialize(writer)?;
        size += self.net_address.serialize(writer)?;
        size += self.public_key.serialize(writer)?;
        size += self.distance.serialize(writer)?;
        if let Some(signature) = &self.signature {
            size += signature.serialize(writer)?;
        } else {
            return Err(beserial::SerializingError::StaticStr("Signature required for serializing PeerAddress"));
        }
        size += match &self.ty {
            PeerAddressType::Dumb => 0,
            PeerAddressType::Ws(host, port) => host.serialize::<u16, W>(writer)? + port.serialize(writer)?,
            PeerAddressType::Wss(host, port) => host.serialize::<u16, W>(writer)? + port.serialize(writer)?,
            PeerAddressType::Rtc => 0
        };
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.ty.protocol().serialized_size();
        size += self.services.serialized_size();
        size += self.timestamp.serialized_size();
        size += self.net_address.serialized_size();
        size += self.public_key.serialized_size();
        size += self.distance.serialized_size();
        size += self.signature.serialized_size() - 1; // No 0/1 for the Option
        size += match &self.ty {
            PeerAddressType::Dumb => 0,
            PeerAddressType::Ws(host, port) => host.serialized_size::<u16>() + port.serialized_size(),
            PeerAddressType::Wss(host, port) => host.serialized_size::<u16>() + port.serialized_size(),
            PeerAddressType::Rtc => 0
        };
        return size;
    }
}

impl Deserialize for PeerAddress {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let protocol: Protocol = Deserialize::deserialize(reader)?;
        let services: ServiceFlags = Deserialize::deserialize(reader)?;
        let timestamp: u64 = Deserialize::deserialize(reader)?;
        let net_address: NetAddress = Deserialize::deserialize(reader)?;
        let public_key: PublicKey = Deserialize::deserialize(reader)?;
        let distance: u8 = Deserialize::deserialize(reader)?;
        let signature: Signature = Deserialize::deserialize(reader)?;
        let type_special: PeerAddressType = match protocol {
            Protocol::Dumb => PeerAddressType::Dumb,
            Protocol::Ws => PeerAddressType::Ws(DeserializeWithLength::deserialize::<u16, R>(reader)?, Deserialize::deserialize(reader)?),
            Protocol::Wss => PeerAddressType::Wss(DeserializeWithLength::deserialize::<u16, R>(reader)?, Deserialize::deserialize(reader)?),
            Protocol::Rtc => PeerAddressType::Rtc
        };
        let peer_id = PeerId::from(&public_key);
        return Ok(PeerAddress{ ty: type_special, services, timestamp, net_address, public_key, distance, signature: Some(signature), peer_id});
    }
}

impl PeerAddress {
    pub fn verify_signature(&self) -> bool {
        if let Some(signature) = &self.signature {
            return self.public_key.verify(signature, self.get_signature_data().as_slice());
        }
        return false;
    }

    pub fn as_uri(&self) -> String {
        let peer_id: String = String::from(::hex::encode(&self.peer_id.0));
        match self.ty {
            PeerAddressType::Dumb => format!("dumb:///{}", peer_id),
            PeerAddressType::Ws(ref host, ref port) => format!("ws:///{}:{}/{}", host, port, peer_id),
            PeerAddressType::Wss(ref host, ref port) => format!("wss:///{}:{}/{}", host, port, peer_id),
            PeerAddressType::Rtc => format!("rtc:///{}", peer_id)
        }
    }

    pub fn get_signature_data(&self) -> Vec<u8> {
        let mut res: Vec<u8> = (self.ty.protocol() as u8).serialize_to_vec();
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

    pub fn is_seed(&self) -> bool {
        return self.timestamp == 0;
    }

    pub fn exceeds_age(&self) -> bool {
        if self.is_seed() {
            return false;
        }

        if let Ok(duration_since_unix) = SystemTime::now().duration_since(UNIX_EPOCH) {
            let age = duration_since_unix - Duration::from_millis(self.timestamp);
            match self.protocol() {
                Protocol::Ws =>  return age > network::address::peer_address_book::MAX_AGE_WEBSOCKET,
                Protocol::Wss =>  return age > network::address::peer_address_book::MAX_AGE_WEBSOCKET,
                Protocol::Rtc =>  return age > network::address::peer_address_book::MAX_AGE_WEBRTC,
                Protocol::Dumb =>  return age > network::address::peer_address_book::MAX_AGE_DUMB,
            }
        }
        return false;
    }

    pub fn protocol(&self) -> Protocol { self.ty.protocol() }

    pub fn peer_id(&self) -> &PeerId { &self.peer_id }
}

impl PartialEq for PeerAddress {
    fn eq(&self, other: &PeerAddress) -> bool {
        // We consider peer addresses to be equal if the public key or peer id is not known on one of them:
        // Peers from the network always contain a peer id and public key, peers without peer id or public key
        // are always set by the user.
        return self.protocol() == other.protocol()
            && self.public_key == other.public_key
            && self.peer_id == other.peer_id
            /* services is ignored */
            /* timestamp is ignored */
            /* netAddress is ignored */
            /* distance is ignored */;
    }
}

impl Eq for PeerAddress {}

impl Hash for PeerAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let peer_id: String = String::from(::hex::encode(&self.peer_id.0));
        let peer_id_uri = match self.ty {
            PeerAddressType::Dumb => format!("dumb:///{}", peer_id),
            PeerAddressType::Ws(_, _) => format!("ws:///{}", peer_id),
            PeerAddressType::Wss(_, _) => format!("wss:///{}", peer_id),
            PeerAddressType::Rtc => format!("rtc:///{}", peer_id)
        };
        peer_id_uri.hash(state);
    }
}

impl fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.as_uri())
    }
}

impl Deserialize for PeerAddressType {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
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
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
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
