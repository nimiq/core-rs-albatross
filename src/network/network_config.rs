use std::fs;
use std::time::SystemTime;

use beserial::{Deserialize, Serialize};

use keys::{KeyPair, PublicKey};
use crate::network::address::net_address::NetAddress;
use crate::network::address::peer_address::{PeerAddress, PeerAddressType};
use crate::network::address::PeerId;
use crate::network::Protocol;
use crate::utils::services::Services;
use utils::time::systemtime_to_timestamp;

use super::ProtocolFlags;

pub struct NetworkConfig {
    protocol_mask: ProtocolFlags,
    key_pair: Option<KeyPair>,
    peer_id: Option<PeerId>,
    services: Services,
    protocol_config: ProtocolConfig,
    user_agent: Option<String>
}

impl NetworkConfig {
    pub fn new_ws_network_config(host: String, port: u16, reverse_proxy_config: Option<ReverseProxyConfig>, user_agent: Option<String>) -> Self {
        Self {
            protocol_mask: ProtocolFlags::WS | ProtocolFlags::WSS,
            key_pair: None,
            peer_id: None,
            services: Services::full(),
            protocol_config: ProtocolConfig::Ws {
                host,
                port,
                reverse_proxy_config,
            },
            user_agent
        }
    }

    pub fn new_wss_network_config(host: String, port: u16, identity_file: String, user_agent: Option<String>) -> Self {
        Self {
            protocol_mask: ProtocolFlags::WS | ProtocolFlags::WSS,
            key_pair: None,
            peer_id: None,
            services: Services::full(),
            protocol_config: ProtocolConfig::Wss {
                host,
                port,
                identity_file,
            },
            user_agent
        }
    }

    pub fn new_dumb_network_config(user_agent: Option<String>) -> Self {
        Self {
            protocol_mask: ProtocolFlags::WS | ProtocolFlags::WSS, // TODO Browsers might not always support WS.
            key_pair: None,
            peer_id: None,
            services: Services::full(),
            protocol_config: ProtocolConfig::Dumb,
            user_agent
        }
    }

    pub fn init_persistent(&mut self) {
        if self.key_pair.is_some() {
            return;
        }

        let key_pair = PeerKeyStore::load_peer_key().unwrap_or_else(|| {
            let key_pair = KeyPair::generate();
            PeerKeyStore::save_peer_key(&key_pair);
            key_pair
        });

        self.peer_id = Some(PeerId::from(&key_pair.public));
        self.key_pair = Some(key_pair);
    }

    pub fn init_volatile(&mut self) {
        let key_pair = KeyPair::generate();
        self.peer_id = Some(PeerId::from(&key_pair.public));
        self.key_pair = Some(key_pair);
    }

    pub fn protocol(&self) -> Protocol {
        Protocol::from(&self.protocol_config)
    }

    pub fn protocol_mask(&self) -> ProtocolFlags {
        self.protocol_mask
    }

    pub fn key_pair(&self) -> &KeyPair {
        &self.key_pair.as_ref().expect("NetworkConfig is uninitialized")
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.key_pair.as_ref().expect("NetworkConfig is uninitialized").public
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id.as_ref().expect("NetworkConfig is uninitialized")
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn set_services(&mut self, services: Services) {
        self.services = services;
    }

    pub fn can_connect(&self, protocol: Protocol) -> bool {
        self.protocol_mask.contains(ProtocolFlags::from(protocol))
    }

    pub fn user_agent(&self) -> &Option<String> {
        &self.user_agent
    }

    pub fn protocol_config(&self) -> &ProtocolConfig {
        &self.protocol_config
    }

    pub fn peer_address(&self) -> PeerAddress {
        // TODO Check PeerAddress globally reachable.
        let mut addr = PeerAddress {
            ty: match self.protocol_config {
                ProtocolConfig::Rtc => PeerAddressType::Rtc,
                ProtocolConfig::Dumb => PeerAddressType::Dumb,
                ProtocolConfig::Ws {
                    ref host,
                    port,
                    ref reverse_proxy_config,
                    ..
                } => {
                    if let Some(reverse_proxy_config) = reverse_proxy_config.as_ref() {
                        PeerAddressType::Ws(host.clone(), reverse_proxy_config.port)
                    } else {
                        PeerAddressType::Ws(host.clone(), port)
                    }
                },
                ProtocolConfig::Wss {
                    ref host,
                    port,
                    ..
                } => PeerAddressType::Wss(host.clone(), port),
            },
            services: self.services.provided,
            timestamp: systemtime_to_timestamp(SystemTime::now()),
            net_address: NetAddress::Unspecified,
            public_key: self.key_pair.as_ref().expect("NetworkConfig is uninitialized").public.clone(),
            distance: 0,
            signature: None,
            peer_id: self.peer_id.as_ref().expect("NetworkConfig is uninitialized").clone(),
        };
        addr.signature = Some(self.key_pair.as_ref().expect("NetworkConfig is uninitialized").sign(&addr.get_signature_data()[..]));
        addr
    }
}

#[derive(Debug, Clone)]
pub struct ReverseProxyConfig {
    pub port: u16,
    pub address: String,
    pub header: String,
}

#[derive(Debug, Clone)]
pub enum ProtocolConfig {
    Dumb,
    Ws {
        host: String,
        port: u16,
        reverse_proxy_config: Option<ReverseProxyConfig>,
    },
    Wss {
        host: String,
        port: u16,
        identity_file: String,
    },
    Rtc,
}

impl From<&ProtocolConfig> for Protocol {
    fn from(config: &ProtocolConfig) -> Self {
        match config {
            ProtocolConfig::Dumb => Protocol::Dumb,
            ProtocolConfig::Rtc => Protocol::Rtc,
            ProtocolConfig::Ws { .. } => Protocol::Ws,
            ProtocolConfig::Wss { .. } => Protocol::Wss,
        }
    }
}

pub struct PeerKeyStore {}

impl PeerKeyStore {
    const KEY_FILE: &'static str = "key.db";

    pub fn load_peer_key() -> Option<KeyPair> {
        fs::read(PeerKeyStore::KEY_FILE).map(|data| {
            Deserialize::deserialize_from_vec(&data).expect("Invalid key file")
        }).ok()
    }

    pub fn save_peer_key(key_pair: &KeyPair) {
        fs::write(PeerKeyStore::KEY_FILE, key_pair.serialize_to_vec()).unwrap();
    }
}
