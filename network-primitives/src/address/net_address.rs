use std::cmp::min;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::net::AddrParseError;
use std::str::FromStr;

use failure::Fail;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};

#[derive(Debug, Ord, PartialOrd, PartialEq, Eq, Hash, Clone)]
pub enum NetAddress {
    IPv4(Ipv4Addr),
    IPv6(Ipv6Addr),
    Unspecified,
    Unknown,
}

impl NetAddress {
    pub fn get_type(&self) -> NetAddressType {
        match self {
            NetAddress::IPv4(_) => NetAddressType::IPv4,
            NetAddress::IPv6(_) => NetAddressType::IPv6,
            NetAddress::Unspecified => NetAddressType::Unspecified,
            NetAddress::Unknown => NetAddressType::Unknown
        }
    }

    pub fn subnet(&self, bit_count: u8) -> Self {
        match self {
            NetAddress::IPv4(ref ip) => {
                let masked = ip_to_subnet(&ip.octets(), bit_count);
                let mut masked_ip = [0u8; 4];
                masked_ip.copy_from_slice(&masked[..]);
                NetAddress::IPv4(masked_ip.into())
            },
            NetAddress::IPv6(ref ip) => {
                let masked = ip_to_subnet(&ip.octets(), bit_count);
                let mut masked_ip = [0u8; 16];
                masked_ip.copy_from_slice(&masked[..]);
                NetAddress::IPv6(masked_ip.into())
            },
            NetAddress::Unspecified => NetAddress::Unspecified,
            NetAddress::Unknown => NetAddress::Unknown
        }
    }

    pub fn is_pseudo(&self) -> bool {
        let ty = self.get_type();
        ty == NetAddressType::Unknown || ty == NetAddressType::Unspecified
    }

    pub fn is_reliable(&self) -> bool {
        // TODO add reliability flag
        !self.is_pseudo()
    }

    pub fn into_ip_address(self) -> Option<IpAddr> {
        match self {
            NetAddress::IPv4(addr) => Some(IpAddr::V4(addr)),
            NetAddress::IPv6(addr) => Some(IpAddr::V6(addr)),
            _ => None
        }
    }
}

fn ip_to_subnet(ip: &[u8], mut bit_count: u8) -> Vec<u8> {
    let mut mask: Vec<u8> = Vec::new();
    for &byte in ip {
        let n = min(bit_count, 8);
        mask.push(byte & ((256 - (1 << (8 - (n as u16)))) as u8));
        bit_count -= n;
    }
    mask
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
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let ty: NetAddressType = Deserialize::deserialize(reader)?;
        match ty {
            NetAddressType::IPv4 => {
                let mut ip = [0u8; 4];
                reader.read_exact(&mut ip)?;
                Ok(NetAddress::IPv4(Ipv4Addr::from(ip)))
            },
            NetAddressType::IPv6 => {
                let mut ip = [0u8; 16];
                reader.read_exact(&mut ip)?;
                Ok(NetAddress::IPv6(Ipv6Addr::from(ip)))
            },
            NetAddressType::Unspecified => Ok(NetAddress::Unspecified),
            NetAddressType::Unknown => Ok(NetAddress::Unknown)
        }
    }
}

impl Serialize for NetAddress {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        size += self.get_type().serialize(writer)?;
        size += match self {
            NetAddress::IPv4(ipv4) => writer.write(&ipv4.octets())?,
            NetAddress::IPv6(ipv6) => writer.write(&ipv6.octets())?,
            NetAddress::Unspecified => 0,
            NetAddress::Unknown => 0
        };
        return Ok(size);
    }

    fn serialized_size(&self) -> usize {
        let mut size = 0;
        size += self.get_type().serialized_size();
        size += match self {
            NetAddress::IPv4(_) => 4,
            NetAddress::IPv6(_) => 16,
            NetAddress::Unspecified => 0,
            NetAddress::Unknown => 0
        };
        return size;
    }
}

impl fmt::Display for NetAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NetAddress::IPv4(ip) => write!(f, "{}", ip),
            NetAddress::IPv6(ip) => write!(f, "{}", ip),
            NetAddress::Unspecified => write!(f, "<unspecified>"),
            NetAddress::Unknown => write!(f, "<unknown>"),
        }
    }
}

#[derive(Debug, Clone, Fail)]
#[fail(display = "{}", _0)]
pub struct NetAddressParseError(#[cause] AddrParseError);

impl FromStr for NetAddress {
    type Err = NetAddressParseError;

    fn from_str(s: &str) -> Result<Self, <Self as FromStr>::Err> {
        let addr: IpAddr = s.parse().map_err(NetAddressParseError)?;
        match addr {
            IpAddr::V4(addr) => Ok(NetAddress::IPv4(addr)),
            IpAddr::V6(addr) => Ok(NetAddress::IPv6(addr)),
        }
    }
}