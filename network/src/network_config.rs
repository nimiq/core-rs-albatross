use std::time::SystemTime;

use keys::{KeyPair, PublicKey};
use network_primitives::address::net_address::NetAddress;
use network_primitives::address::peer_address::{PeerAddress, PeerAddressType};
use network_primitives::address::PeerId;
use network_primitives::address::seed_list::SeedList;
use network_primitives::protocol::{Protocol, ProtocolFlags};
use network_primitives::services::Services;
use utils::time::systemtime_to_timestamp;
use utils::key_store::{Error as KeyStoreError, KeyStore};
use network_primitives::address::{PeerUri};

use crate::error::Error;


// One or multiple seed nodes. Either a peer URI or a http(s) URL to a seed list
#[derive(Clone, Debug)]
pub enum Seed {
    Peer(PeerUri),
    List(SeedList)
}


#[derive(Clone, Debug)]
pub struct NetworkConfig {
    protocol_mask: ProtocolFlags,
    key_pair: Option<KeyPair>,
    peer_id: Option<PeerId>,
    services: Services,
    protocol_config: ProtocolConfig,
    user_agent: Option<String>,
    additional_seeds: Vec<Seed>,
}

impl NetworkConfig {
    pub fn new_ws_network_config(host: String, port: u16, reverse_proxy_config: Option<ReverseProxyConfig>) -> Self {
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
            user_agent: None,
            additional_seeds: Vec::new()
        }
    }

    pub fn new_wss_network_config(host: String, port: u16, identity_file: String, identity_password: String) -> Self {
        Self {
            protocol_mask: ProtocolFlags::WS | ProtocolFlags::WSS,
            key_pair: None,
            peer_id: None,
            services: Services::full(),
            protocol_config: ProtocolConfig::Wss {
                host,
                port,
                identity_file,
                identity_password,
            },
            user_agent: None,
            additional_seeds: Vec::new()
        }
    }

    pub fn new_dumb_network_config() -> Self {
        Self {
            protocol_mask: ProtocolFlags::WS | ProtocolFlags::WSS, // TODO Browsers might not always support WS.
            key_pair: None,
            peer_id: None,
            services: Services::full(),
            protocol_config: ProtocolConfig::Dumb,
            user_agent: None,
            additional_seeds: Vec::new()
        }
    }

    pub fn init_persistent(&mut self, peer_key_store: &KeyStore) -> Result<(), Error> {
        if self.key_pair.is_some() {
            return Ok(());
        }

        let key_pair = match peer_key_store.load_key() {
            Err(KeyStoreError::IoError(_)) => {
                let key_pair = KeyPair::generate();
                peer_key_store.save_key(&key_pair)?;
                Ok(key_pair)
            },
            res => res,
        }?;

        self.peer_id = Some(PeerId::from(&key_pair.public));
        self.key_pair = Some(key_pair);
        Ok(())
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

    pub fn set_user_agent(&mut self, user_agent: String) {
        self.user_agent = Some(user_agent)
    }

    pub fn additional_seeds(&self) -> &Vec<Seed> {
        &self.additional_seeds
    }

    pub fn set_additional_seeds(&mut self, seeds: Vec<Seed>) {
        self.additional_seeds = seeds
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
                        if reverse_proxy_config.with_tls_termination {
                            PeerAddressType::Wss(host.clone(), reverse_proxy_config.port)
                        } else {
                            PeerAddressType::Ws(host.clone(), reverse_proxy_config.port)
                        }
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
            public_key: self.key_pair.as_ref().expect("NetworkConfig is uninitialized").public,
            distance: 0,
            signature: None,
            peer_id: self.peer_id.as_ref().expect("NetworkConfig is uninitialized").clone(),
        };
        if addr.protocol() == Protocol::Wss || addr.protocol() == Protocol::Ws {
            // TODO Disabled for debugging
            //assert!(addr.is_globally_reachable(false), "PeerAddress not globally reachable.");
        }
        addr.signature = Some(self.key_pair.as_ref().expect("NetworkConfig is uninitialized").sign(&addr.get_signature_data()[..]));
        addr
    }

    pub fn is_initialized(&self) -> bool {
        self.key_pair.is_some() && self.peer_id.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct ReverseProxyConfig {
    pub port: u16,
    pub address: NetAddress,
    pub header: String,
    pub with_tls_termination: bool,
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
        identity_password: String,
    },
    Rtc,
}

impl From<&ProtocolConfig> for Protocol {
    fn from(config: &ProtocolConfig) -> Self {
        match config {
            ProtocolConfig::Dumb => Protocol::Dumb,
            ProtocolConfig::Rtc => Protocol::Rtc,
            ProtocolConfig::Ws { reverse_proxy_config, .. } => {
                match reverse_proxy_config {
                    Some(ReverseProxyConfig { with_tls_termination: true, .. }) => Protocol::Wss,
                    Some(ReverseProxyConfig { with_tls_termination: false, .. }) => Protocol::Ws,
                    _ => Protocol::Ws,
                }
            },
            ProtocolConfig::Wss { .. } => Protocol::Wss,
        }
    }
}
