#![allow(unused_imports, unused_variables, dead_code)]

#[macro_use]
extern crate log;
extern crate nimiq_block as block;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_consensus as consensus;
extern crate nimiq_mempool as mempool;
extern crate nimiq_network_interface as network;

use std::fs::File;
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::stream::Stream;
use futures::{future::Future, IntoFuture};
use hyper::server::conn::Http;
use native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use tokio::net::TcpListener;
use tokio_tls::TlsAcceptor as TokioTlsAcceptor;

use consensus::Consensus;
use network::network::Network;

use crate::error::Error;
pub use crate::metrics::chain::{AbstractChainMetrics, AlbatrossChainMetrics};
use crate::metrics::mempool::MempoolMetrics;
use crate::metrics::network::NetworkMetrics;

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

pub mod error;
pub mod metrics;
pub mod server;

pub type MetricsServerFuture = Box<dyn Future<Item = (), Error = ()> + Send + Sync>;

pub struct MetricsServer {
    future: MetricsServerFuture,
}

impl MetricsServer {
    pub fn new<CM, N>(
        ip: IpAddr,
        port: u16,
        username: Option<String>,
        password: Option<String>,
        pkcs12_key_file: &str,
        pkcs12_passphrase: &str,
        consensus: Arc<Consensus<N>>,
    ) -> Result<MetricsServer, Error>
    where
        CM: AbstractChainMetrics + server::Metrics + 'static,
        N: Network + 'static,
    {
        let mut file = File::open(pkcs12_key_file)?;
        let mut pkcs12 = vec![];
        file.read_to_end(&mut pkcs12)?;
        let pkcs12 = Identity::from_pkcs12(&pkcs12, pkcs12_passphrase)?;

        let tls_cx = NativeTlsAcceptor::builder(pkcs12).build()?;
        let tls_cx = TokioTlsAcceptor::from(tls_cx);

        let srv = TcpListener::bind(&SocketAddr::new(ip, port))?;

        // let future = Box::new(
        //     Http::new()
        //         .serve_incoming(
        //             srv.incoming()
        //                 .and_then(move |socket| tls_cx.accept(socket).map_err(|e| io::Error::new(io::ErrorKind::Other, e))),
        //             move || {
        //                 server::MetricsServer::new(
        //                     vec![
        //                         Arc::new(CM::new(consensus.blockchain.clone())),
        //                         Arc::new(MempoolMetrics::new(consensus.mempool.clone())),
        //                         Arc::new(NetworkMetrics::new(consensus.network.clone())),
        //                     ],
        //                     attributes! { "peer" => consensus.network.network_config.peer_address() },
        //                     username.clone(),
        //                     password.clone(),
        //                 )
        //             },
        //         )
        //         .then(|res| match res {
        //             Ok(conn) => Ok(Some(conn)),
        //             Err(e) => {
        //                 error!("Metrics server failed: {}", e);
        //                 Ok(None)
        //             }
        //         })
        //         .for_each(|conn_opt| {
        //             if let Some(conn) = conn_opt {
        //                 hyper::rt::spawn(
        //                     conn.and_then(|c| c.map_err(|e| panic!("Metrics server unrecoverable error {}", e)))
        //                         .map_err(|e| error!("Metrics server connection error: {}", e)),
        //                 );
        //             }
        //
        //             Ok(())
        //         }),
        // );

        unimplemented!()
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
