extern crate futures;
#[macro_use]
extern crate lazy_static;
extern crate nimiq;
extern crate parking_lot;
extern crate tokio;
extern crate pretty_env_logger;

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
    pretty_env_logger::try_init().unwrap_or(());

    let network_id = NetworkId::Main;

    let mut network_config = NetworkConfig::new_ws_network_config(
        "test.vcap.me".to_string(),
        13337,
        None,
    );
    network_config.init_volatile();

    let network_time = Arc::new(NetworkTime::new());
    let blockchain: Arc<Blockchain<'static>> = Arc::new(Blockchain::new(&env, network_id, network_time.clone()));
    let network = Network::new(blockchain.clone(), network_config, network_time.clone());

    tokio::run(Runner {
        network: network.clone(),
        ran: false,
    });

    println!("Something from network: {:?}", network.peer_count());
}

pub struct Runner {
    network: Arc<Network>,
    ran: bool,
}

impl Future for Runner {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Result<Async<<Self as Future>::Item>, <Self as Future>::Error> {
        if !self.ran {
            self.network.initialize();
            self.network.connect();
            self.ran = true;
        }

        Ok(Async::Ready(()))
    }
}
