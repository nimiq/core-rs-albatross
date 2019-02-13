use std::sync::Arc;

use failure::Fail;
use futures::{Async, Future, Poll};

use network::error::Error as NetworkError;
use network::network::Network;

/// Prototype for a Error returned by these futures
/// Errors can occur, when e.g. the bind port is already used
#[derive(Debug, Fail)]
pub enum ClientError {
    #[fail(display = "{}", _0)]
    NetworkError(#[cause] NetworkError),
}

/// A trait representing a Client that may be uninitialized, initialized or connected
trait Client {
    fn initialized(&self) -> bool;
    fn connected(&self) -> bool;
    fn network(&self) -> Arc<Network>;
}



/// Prototype for initialize method. This could be a method of a ClientBuilder
pub fn initialize(network: Arc<Network>) -> ClientInitializeFuture {
    ClientInitializeFuture { network, initialized: false }
}


/// Future that eventually returns a InitializedClient
pub struct ClientInitializeFuture {
    network: Arc<Network>,
    initialized: bool
}

impl Future for ClientInitializeFuture {
    type Item = InitializedClient;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<InitializedClient, ClientError> {
        // NOTE: This is practically Future::fuse, but this way the types are cleaner
        if !self.initialized {
            self.network.initialize().map_err(|e| ClientError::NetworkError(e))?;
            Ok(Async::Ready(InitializedClient { network: Arc::clone(&self.network) }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}


/// The initialized client
pub struct InitializedClient {
    network: Arc<Network>
}

impl InitializedClient {
    pub fn connect(&self) -> ClientConnectFuture {
        ClientConnectFuture { network: Arc::clone(&self.network), initialized: false }
    }
}

impl Client for InitializedClient {
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        false
    }

    fn network(&self) -> Arc<Network> {
        Arc::clone(&self.network)
    }
}


/// Future that eventually returns a ConnectedClient
pub struct ClientConnectFuture {
    network: Arc<Network>,
    initialized: bool
}

impl Future for ClientConnectFuture {
    type Item = ConnectedClient;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<ConnectedClient, ClientError> {
        if !self.initialized {
            self.network.connect().map_err(|e| ClientError::NetworkError(e))?;
            Ok(Async::Ready(ConnectedClient { network: Arc::clone(&self.network) }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}


/// The connected client
pub struct ConnectedClient {
    network: Arc<Network>
}

impl ConnectedClient {
    pub fn network(&self) -> Arc<Network> {
        Arc::clone(&self.network)
    }
}

impl Client for ConnectedClient {
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        true
    }

    fn network(&self) -> Arc<Network> {
        Arc::clone(&self.network)
    }
}
