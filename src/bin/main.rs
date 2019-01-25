extern crate futures;
#[macro_use]
extern crate lazy_static;
extern crate nimiq;
extern crate parking_lot;
extern crate tokio;
#[macro_use]
extern crate log;
extern crate pretty_env_logger;

use std::sync::Arc;

use futures::Async;
use futures::future::Future;


use nimiq::consensus::networks::NetworkId;
use nimiq::network::network_config::NetworkConfig;
use database::Environment;
use database::lmdb::{LmdbEnvironment, open};
use nimiq::network::network::Network;
use nimiq::consensus::consensus::Consensus;

lazy_static! {
    static ref env: Environment = LmdbEnvironment::new("./db/", 1024 * 1024 * 50, 10, open::Flags::empty()).unwrap(); //VolatileEnvironment::new(10).unwrap();
}

pub fn main() {
    pretty_env_logger::try_init().unwrap_or(());

    let network_id = NetworkId::Main;

    let mut network_config = NetworkConfig::new_ws_network_config(
        "test.vcap.me".to_string(),
        13337,
        None,
        None
    );
    network_config.init_volatile();

    info!("Nimiq Core starting: network={:?}, peer_address={}", network_id, network_config.peer_address());

    let consensus = Consensus::new(&env, network_id, network_config);

    info!("Blockchain state: height={}, head={}", consensus.blockchain.height(), consensus.blockchain.head_hash());

    tokio::run(Runner {
        network: consensus.network.clone(),
        initialized: false,
    });
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
