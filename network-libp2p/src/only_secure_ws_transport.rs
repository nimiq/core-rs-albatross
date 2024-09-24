use std::{
    pin::Pin,
    task::{Context, Poll},
};

use libp2p::{
    core::transport::{DialOpts, ListenerId, TransportError, TransportEvent},
    multiaddr::Multiaddr,
};

use crate::utils;

/// Drops all dial requests to non-secure Web Socket multiaddresses.
/// Inspired on the global-only transport from libp2p: https://github.com/libp2p/rust-libp2p/blob/master/core/src/transport/global_only.rs
#[derive(Debug, Clone, Default)]
pub struct Transport<T> {
    inner: T,
}

impl<T> Transport<T> {
    pub(crate) fn new(transport: T) -> Self {
        Transport { inner: transport }
    }
}

impl<T: libp2p::Transport + Unpin> libp2p::Transport for Transport<T> {
    type Output = <T as libp2p::Transport>::Output;
    type Error = <T as libp2p::Transport>::Error;
    type ListenerUpgrade = <T as libp2p::Transport>::ListenerUpgrade;
    type Dial = <T as libp2p::Transport>::Dial;

    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        self.inner.listen_on(id, addr)
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        self.inner.remove_listener(id)
    }

    fn dial(
        &mut self,
        addr: Multiaddr,
        opts: DialOpts,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        if utils::is_address_ws_secure(&addr) {
            return self.inner.dial(addr, opts);
        }
        debug!(%addr, "Not dialing non secure address");
        Err(TransportError::MultiaddrNotSupported(addr))
    }

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Pin::new(&mut self.inner).poll(cx)
    }
}
