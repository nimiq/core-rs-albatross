extern crate toml;
#[macro_use]
extern crate serde_derive;
extern crate pretty_env_logger;
#[macro_use]
extern crate log;
extern crate futures;
extern crate database;
extern crate tokio;
#[macro_use]
extern crate lazy_static;

extern crate nimiq;

#[cfg(debug_assertions)]
extern crate dotenv;

mod settings;

use std::path::Path;
use std::error::Error;
use std::sync::Arc;

use futures::Async;
use futures::future::Future;

use nimiq::network::network_config::NetworkConfig;
use nimiq::consensus::networks::NetworkId;
use nimiq::consensus::consensus::Consensus;
use nimiq::network::network::Network;
use nimiq::network::network_config::ReverseProxyConfig;
use database::Environment;
use database::lmdb::{LmdbEnvironment, open};
use database::volatile::VolatileEnvironment;

use crate::settings::Settings;
use crate::settings as s;



lazy_static! {
    static ref env: Environment = LmdbEnvironment::new("./db/", 1024 * 1024 * 50, 10, open::Flags::empty()).unwrap();
}


// There is no really good way for a condition on the compilation envirnoment (dev, staging, prod)
#[cfg(debug_assertions)]
fn dev_init() {
    dotenv::dotenv().expect("Couldn't load dotenv");
}

fn main() -> Result<(), Box<dyn Error>> {
    dev_init();

    pretty_env_logger::init();

    let argv: Vec<String> = std::env::args().collect();
    let path = argv.get(1).map(|p| p.as_str()).unwrap_or("config.toml");

    info!("Reading configuration: {}", path);
    let settings = Settings::from_file(path)?;

    debug!("Settings: {:#?}", settings);

    // TODO: If a wallet is specified, we should use it, shouldn't we?
    assert!(settings.wallet.is_none(), "Wallet settings are not implemented yet");

    // Start database
    // TODO: env must have lifetime 'static
    /*
    let env: Environment = (if settings.volatile {
        VolatileEnvironment::new(settings.database.max_dbs)?
    }
    else {
        LmdbEnvironment::new(&settings.database.path, settings.database.size, settings.database.max_dbs, open::Flags::empty())?
    });
    */

    // Start network
    let mut network_config = match settings.network.protocol {
        s::Protocol::Dumb => NetworkConfig::new_dumb_network_config(),

        s::Protocol::Ws => NetworkConfig::new_ws_network_config(
            settings.network.host,
            settings.network.port.unwrap_or(s::DEFAULT_NETWORK_PORT),
            settings.reverse_proxy.map(|r| ReverseProxyConfig {
                port: r.port.unwrap_or(s::DEFAULT_REVERSE_PROXY_PORT),
                address: r.address,
                header: r.header
            })),

        // TODO: What is a `identify_file`? Shouldn't this be a key and cert file?
        s::Protocol::Wss => NetworkConfig::new_wss_network_config(
            settings.network.host,
            settings.network.port.unwrap_or(s::DEFAULT_NETWORK_PORT),
            unimplemented!()),

        s::Protocol::Rtc => unimplemented!()
    };
    network_config.init_persistent();
    let network_id = match settings.consensus.network {
        s::Network::Main => NetworkId::Main,
        s::Network::Test => NetworkId::Test,
        s::Network::Dev => NetworkId::Dev,
        s::Network::Dummy => NetworkId::Dummy,
        s::Network::Bounty => NetworkId::Bounty
    };
    info!("Nimiq Core starting: network={:?}, peer_address={}", network_id, network_config.peer_address());

    // Start consensus
    let consensus = Consensus::new(&env, network_id, network_config);
    info!("Blockchain state: height={}, head={}", consensus.blockchain.height(), consensus.blockchain.head_hash());

    tokio::run(Runner {
        network: consensus.network.clone(),
        initialized: false,
    });

    Ok(())
}


pub struct Runner {
    network: Arc<Network>,
    initialized: bool,
}

impl Future for Runner {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Result<Async<<Self as Future>::Item>, <Self as Future>::Error> {
        if !self.initialized {
            self.network.initialize();
            self.network.connect();
            self.initialized = true;
        }
        Ok(Async::Ready(()))
    }
}
