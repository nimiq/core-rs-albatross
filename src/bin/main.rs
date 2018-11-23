extern crate futures;
#[macro_use]
extern crate lazy_static;
extern crate nimiq;
extern crate parking_lot;
extern crate tokio;

use std::sync::Arc;

use futures::Async;
use futures::future::Future;
use parking_lot::RwLock;

use nimiq::consensus::base::blockchain::Blockchain;
use nimiq::consensus::networks::NetworkId;
use nimiq::network::address::peer_address_book::PeerAddressBook;
use nimiq::network::connection::connection_pool::ConnectionPool;
use nimiq::network::network_config::NetworkConfig;
use nimiq::network::NetworkTime;
use nimiq::utils::db::Environment;
use nimiq::utils::db::volatile::VolatileEnvironment;
use nimiq::network::network::Network;

lazy_static! {
    static ref env: Environment = VolatileEnvironment::new(10).unwrap();
}

pub fn main() {
    let network = NetworkId::Main;

    let mut network_config = NetworkConfig::new_ws_network_config(
        "test.vcap.me".to_string(),
        13337,
        None,
    );
    network_config.init_volatile();
    let network_config = Arc::new(network_config);

    let network_time = Arc::new(NetworkTime::new(0));
    let blockchain: Arc<Blockchain<'static>> = Arc::new(Blockchain::new(&env, network_time.clone(), network));

    tokio::run(Runner {
        network_time,
        network_config,
        blockchain,
    });
}

pub struct Runner {
    network_time: Arc<NetworkTime>,
    network_config: Arc<NetworkConfig>,
    blockchain: Arc<Blockchain<'static>>,
}

impl Future for Runner {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Result<Async<<Self as Future>::Item>, <Self as Future>::Error> {
        let network = Network::new(self.blockchain.clone(), self.network_config.clone(), self.network_time.clone());
        network.connect();
        Ok(Async::Ready(()))
    }
}
