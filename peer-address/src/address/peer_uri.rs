use std::fmt;
use std::str::FromStr;

use hex::FromHex;
use thiserror::Error;
use url::Url;

use nimiq_keys::PublicKey;

use crate::address::{NetAddress, PeerAddress, PeerAddressType, PeerId};
use crate::protocol::Protocol;
use crate::services::ServiceFlags;

#[derive(Debug, Error)]
pub enum PeerUriError {
    #[error("Invalid URI: {0}")]
    InvalidUri(#[from] url::ParseError),
    #[error("Protocol unknown")]
    UnknownProtocol,
    #[error("Peer ID is missing")]
    MissingPeerId,
    #[error("Hostname is missing")]
    MissingHostname,
    #[error("Unexpected username in URI")]
    UnexpectedUsername,
    #[error("Unexpected password in URI")]
    UnexpectedPassword,
    #[error("Unexpected query in URI")]
    UnexpectedQuery,
    #[error("Unexpected fragment in URI")]
    UnexpectedFragment,
    #[error("Unexpected port number")]
    UnexpectedPort,
    #[error("Unexpected path segment")]
    UnexpectedPath,
    #[error("Too many path segments")]
    TooManyPathSegments,
    #[error("Invalid peer ID")]
    InvalidPeerId,
    #[error("Invalid public key {0}")]
    InvalidPublicKey(#[from] nimiq_keys::ParseError),
    #[error("Seed node is missing the public key")]
    SeedNodeMissingPublicKey,
    #[error("The only allowed protocols for seed nodes are Wss and Ws")]
    SeedNodeWithInvalidProtocol,
}

impl FromStr for Protocol {
    type Err = PeerUriError;

    fn from_str(s: &str) -> Result<Protocol, PeerUriError> {
        match s {
            "dumb" => Ok(Protocol::Dumb),
            "ws" => Ok(Protocol::Ws),
            "wss" => Ok(Protocol::Wss),
            "rtc" => Ok(Protocol::Rtc),
            _ => Err(PeerUriError::UnknownProtocol),
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Protocol::Dumb => "dumb",
                Protocol::Ws => "ws",
                Protocol::Wss => "wss",
                Protocol::Rtc => "rtc",
            }
        )
    }
}

#[derive(Debug, Clone)]
pub struct PeerUri {
    protocol: Protocol,
    hostname: Option<String>,
    port: Option<u16>,
    peer_id: Option<String>,
    public_key: Option<String>,
}

impl FromStr for PeerUri {
    type Err = PeerUriError;

    fn from_str(s: &str) -> Result<Self, PeerUriError> {
        let url = Url::parse(s)?;
        Self::from_url(url)
    }
}

impl fmt::Display for PeerUri {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.protocol {
            Protocol::Dumb | Protocol::Rtc => {
                write!(
                    f,
                    "{}://{}",
                    self.protocol,
                    self.peer_id().expect("No peer ID for dumb/rtc URI")
                )?;
            }
            Protocol::Ws | Protocol::Wss => {
                write!(f, "{}://{}", self.protocol, self.hostname.as_ref().unwrap())?;
                self.port.map(|p| write!(f, ":{}", p)).transpose()?;
                self.peer_id()
                    .or_else(|| self.public_key())
                    .map(|p| write!(f, "/{}", p))
                    .transpose()?;
            }
        }
        Ok(())
    }
}

impl PeerUri {
    pub fn new_wss(
        hostname: String,
        port: Option<u16>,
        peer_id: Option<String>,
        public_key: Option<String>,
    ) -> PeerUri {
        PeerUri {
            protocol: Protocol::Wss,
            hostname: Some(hostname),
            port,
            peer_id,
            public_key,
        }
    }

    pub fn from_url(url: Url) -> Result<Self, PeerUriError> {
        if !url.username().is_empty() {
            return Err(PeerUriError::UnexpectedUsername);
        }
        if url.password().is_some() {
            return Err(PeerUriError::UnexpectedPassword);
        }
        if url.query().is_some() {
            return Err(PeerUriError::UnexpectedQuery);
        }
        if url.fragment().is_some() {
            return Err(PeerUriError::UnexpectedFragment);
        }

        let protocol = Protocol::from_str(url.scheme())?;

        // Takes path segments and either returns Some(segment) if there was a single segment
        // or None if there was no path segments at all. If there are multiple segments, returns
        // with an error.
        //
        // For Dumb and Rtc this must be None (checked later). For Ws and Wss this is the peer_id.
        let path_segment = url
            .path_segments()
            .and_then(|segments| {
                let segments = segments.collect::<Vec<&str>>();
                match segments.len() {
                    0 => None,
                    1 => {
                        if segments[0].is_empty() {
                            None
                        } else {
                            Some(Ok(String::from(segments[0])))
                        }
                    }
                    _ => Some(Err(PeerUriError::TooManyPathSegments)),
                }
            })
            .transpose()?;

        // Take appropriate parts of URI to construct the PeerUri
        match protocol {
            Protocol::Dumb | Protocol::Rtc => {
                let peer_id = String::from(url.host_str().ok_or(PeerUriError::MissingPeerId)?);
                if url.port().is_some() {
                    return Err(PeerUriError::UnexpectedPort);
                }
                if path_segment.is_some() {
                    return Err(PeerUriError::UnexpectedPath);
                }
                Ok(PeerUri {
                    protocol,
                    hostname: None,
                    port: None,
                    peer_id: Some(peer_id), // For dumb or Rtc this is always the peer ID
                    public_key: None,
                })
            }
            Protocol::Ws | Protocol::Wss => {
                let host = String::from(url.host_str().ok_or(PeerUriError::MissingHostname)?);
                let (peer_id, public_key) = match path_segment {
                    Some(ref peer_id) if peer_id.len() == 2 * PeerId::SIZE => (path_segment, None),
                    Some(ref public_key) if public_key.len() == 2 * PublicKey::SIZE => {
                        (None, path_segment)
                    }
                    None => (None, None),
                    _ => return Err(PeerUriError::InvalidPeerId),
                };
                Ok(PeerUri {
                    protocol,
                    hostname: Some(host),
                    port: url.port(),
                    peer_id,
                    public_key,
                })
            }
        }
    }

    pub fn protocol(&self) -> Protocol {
        self.protocol
    }
    pub fn hostname(&self) -> Option<&String> {
        self.hostname.as_ref()
    }
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn peer_id(&self) -> Option<&String> {
        self.peer_id.as_ref()
    }
    pub fn public_key(&self) -> Option<&String> {
        self.public_key.as_ref()
    }

    pub fn as_seed_peer_address(&self) -> Result<PeerAddress, PeerUriError> {
        // TODO: May be we want to allow seed nodes without public key?
        if self.public_key().is_none() {
            return Err(PeerUriError::SeedNodeMissingPublicKey);
        }

        let public_key =
            match PublicKey::from_hex(self.public_key().expect("Checked in previous step")) {
                Ok(public_key) => public_key,
                Err(e) => return Err(PeerUriError::from(e)),
            };

        match self.protocol() {
            Protocol::Wss => Ok(PeerAddress {
                ty: PeerAddressType::Wss(
                    self.hostname().expect("Mandatory for Wss").to_string(),
                    self.port().unwrap_or(443),
                ),
                services: ServiceFlags::FULL,
                timestamp: 0,
                net_address: NetAddress::Unspecified,
                public_key,
                distance: 0,
                signature: None,
                peer_id: PeerId::from(&public_key),
            }),
            Protocol::Ws => Ok(PeerAddress {
                ty: PeerAddressType::Ws(
                    self.hostname().expect("Mandatory for Ws").to_string(),
                    self.port().unwrap_or(80),
                ),
                services: ServiceFlags::FULL,
                timestamp: 0,
                net_address: NetAddress::Unspecified,
                public_key,
                distance: 0,
                signature: None,
                peer_id: PeerId::from(&public_key),
            }),
            _ => Err(PeerUriError::SeedNodeWithInvalidProtocol),
        }
    }
}

impl From<PeerAddress> for PeerUri {
    fn from(peer_address: PeerAddress) -> PeerUri {
        let protocol = peer_address.ty.protocol();
        let peer_id = Some(peer_address.peer_id.to_hex());

        match peer_address.ty {
            PeerAddressType::Dumb | PeerAddressType::Rtc => PeerUri {
                protocol,
                peer_id,
                hostname: None,
                port: None,
                public_key: None,
            },
            PeerAddressType::Ws(host, port) | PeerAddressType::Wss(host, port) => PeerUri {
                protocol,
                peer_id,
                hostname: Some(host),
                port: Some(port),
                public_key: None,
            },
        }
    }
}
