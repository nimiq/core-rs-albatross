#[macro_use]
extern crate log;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_blockchain_albatross as blockchain_albatross;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_consensus as consensus;
extern crate nimiq_mempool as mempool;
extern crate nimiq_network as network;
extern crate nimiq_block as block;
extern crate nimiq_block_albatross as block_albatross;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::{future::Future, IntoFuture};
use hyper::Server;

use consensus::{Consensus, ConsensusProtocol};

use crate::error::Error;
use crate::metrics::mempool::MempoolMetrics;
use crate::metrics::network::NetworkMetrics;
pub use crate::metrics::chain::{AbstractChainMetrics, NimiqChainMetrics, AlbatrossChainMetrics};

macro_rules! attributes {
    // Empty attributes.
    {} => ({
        use $crate::server::attributes::VecAttributes;

        VecAttributes::new()
    });

    // Non-empty attributes, no trailing comma.
    //
    // In this implementation, key/value pairs separated by commas.
    { $( $key:expr => $value:expr ),* } => {
        attributes!( $(
            $key => $value,
        )* )
    };

    // Non-empty attributes, trailing comma.
    //
    // In this implementation, the comma is part of the value.
    { $( $key:expr => $value:expr, )* } => ({
        use $crate::server::attributes::VecAttributes;

        let mut attributes = VecAttributes::new();

        $(
            attributes.add($key, $value);
        )*

        attributes
    })
}

pub mod server;
pub mod metrics;
pub mod error;


pub type MetricsServerFuture = Box<dyn Future<Item=(), Error=()> + Send + Sync>;

pub struct MetricsServer {
    future: MetricsServerFuture
}

impl MetricsServer {
    pub fn new<P, CM>(ip: IpAddr, port: u16, username: Option<String>, password: Option<String>, consensus: Arc<Consensus<P>>) -> Result<MetricsServer, Error>
        where P: ConsensusProtocol + 'static,
              CM: AbstractChainMetrics<P> + server::Metrics + 'static
    {
        let future = Box::new(Server::try_bind(&SocketAddr::new(ip, port))?
            .serve(move || {
                server::MetricsServer::new(
                    vec![
                        Arc::new(CM::new(consensus.blockchain.clone())),
                        Arc::new(MempoolMetrics::new(consensus.mempool.clone())),
                        Arc::new(NetworkMetrics::new(consensus.network.clone()))
                    ],
                    attributes! { "peer" => consensus.network.network_config.peer_address() },
                    username.clone(),
                    password.clone())
            })
            .map_err(|e| error!("Metrics server failed: {}", e)));

        Ok(MetricsServer {
            future,
        })
    }
}

impl IntoFuture for MetricsServer {
    type Future = MetricsServerFuture;
    type Item = ();
    type Error = ();

    fn into_future(self) -> Self::Future {
        self.future
    }
}
