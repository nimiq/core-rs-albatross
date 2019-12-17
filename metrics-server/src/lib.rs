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

use std::io::Read;
use std::fs::File;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use hyper::server::conn::Http;
use native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use tokio::net::TcpListener;
use tokio_tls::TlsAcceptor as TokioTlsAcceptor;

use consensus::{Consensus, ConsensusProtocol};

pub use crate::error::Error;
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

// These need to be imported after we define the macro
pub mod server;
pub mod metrics;
pub mod error;



pub async fn metrics_server<P, CM>(
    ip: IpAddr,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    pkcs12_key_file: &str,
    pkcs12_passphrase: &str,
    consensus: Arc<Consensus<P>>
) -> Result<(), Error>
    where P: ConsensusProtocol + 'static,
          CM: AbstractChainMetrics<P> + server::Metrics + 'static
{
    let mut file = File::open(pkcs12_key_file)?;
    let mut pkcs12 = vec![];
    file.read_to_end(&mut pkcs12)?;
    let pkcs12 = Identity::from_pkcs12(&pkcs12, pkcs12_passphrase)?;

    let tls_cx = NativeTlsAcceptor::builder(pkcs12).build()?;
    let tls_cx = TokioTlsAcceptor::from(tls_cx);

    let mut listener = TcpListener::bind(&SocketAddr::new(ip, port)).await?;

    loop {
        match listener.accept().await {
            Ok((connection, address)) => {
                info!("Connection from: {}", address);
                let tls_connection = tls_cx.accept(connection).await?;
                let metrics = server::MetricsServer::new(
                    vec![
                        Arc::new(CM::new(consensus.blockchain.clone())),
                        Arc::new(MempoolMetrics::new(consensus.mempool.clone())),
                        Arc::new(NetworkMetrics::new(consensus.network.clone()))
                    ],
                    attributes! { "peer" => consensus.network.network_config.peer_address() },
                    username.clone(),
                    password.clone()
                );
                Http::new().serve_connection(tls_connection, metrics).await?;
            },
            Err(e) => {
                error!("Connection failed: {}", e);
            }
        }
    }
}
