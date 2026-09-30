use std::{
    io,
    panic::AssertUnwindSafe,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, FutureExt};
use libp2p::{
    core::transport::{DialOpts, ListenerId, TransportError, TransportEvent},
    dns,
    multiaddr::Multiaddr,
};

/// Turns a panic while dialing through the DNS transport into a dial error.
///
/// libp2p-dns 0.45 panics if a successful lookup has no matching record in the answer section,
/// which hickory 0.26 allows (e.g. when the address of a CNAME target is only in the additional
/// section). The panic ends the connection task before it reports back to the swarm, so the
/// pending connection would never be released and would count against the pending connection
/// limit forever.
#[derive(Debug, Clone, Default)]
pub struct Transport<T> {
    inner: T,
}

impl<T> Transport<T> {
    pub(crate) fn new(transport: T) -> Self {
        Transport { inner: transport }
    }
}

impl<T> libp2p::Transport for Transport<T>
where
    T: libp2p::Transport<Error = dns::Error<io::Error>> + Unpin,
    T::Dial: Send + 'static,
    T::Output: 'static,
{
    type Output = <T as libp2p::Transport>::Output;
    type Error = <T as libp2p::Transport>::Error;
    type ListenerUpgrade = <T as libp2p::Transport>::ListenerUpgrade;
    type Dial = BoxFuture<'static, Result<Self::Output, Self::Error>>;

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
        let dial = self.inner.dial(addr.clone(), opts)?;
        Ok(AssertUnwindSafe(dial)
            .catch_unwind()
            .map(move |result| {
                result.unwrap_or_else(|_| {
                    warn!(%addr, "Dialing panicked");
                    Err(dns::Error::Transport(io::Error::other("dialing panicked")))
                })
            })
            .boxed())
    }

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Pin::new(&mut self.inner).poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        net::{SocketAddr, UdpSocket},
        pin::Pin,
        task::{Context, Poll},
        thread,
        time::Duration,
    };

    use futures::future::{self, BoxFuture, FutureExt};
    use hickory_resolver::config::NameServerConfig;
    use libp2p::{
        core::{
            transport::{DialOpts, ListenerId, PortUse, TransportError, TransportEvent},
            Endpoint,
        },
        dns, tcp, Multiaddr, Transport as _,
    };
    use nimiq_test_log::test;

    fn dial_opts() -> DialOpts {
        DialOpts {
            role: Endpoint::Dialer,
            port_use: PortUse::Reuse,
        }
    }

    /// Starts a DNS server on localhost that answers every query with an empty ANSWER section and
    /// an A record for 127.0.0.1 in the ADDITIONAL section.
    fn start_dns_server_without_answers() -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        thread::spawn(move || {
            let mut buf = [0u8; 512];
            while let Ok((len, peer)) = socket.recv_from(&mut buf) {
                let query = &buf[..len];
                // The question follows the 12-byte header: the name's labels up to the root
                // label, then the type and the class.
                let mut end = 12;
                while end < len && query[end] != 0 {
                    end += 1 + query[end] as usize;
                }
                end += 1 + 4;
                if end > len {
                    continue;
                }

                let mut response = query[..2].to_vec(); // ID
                response.extend_from_slice(&[0x81, 0x80]); // Response, RD, RA, NOERROR
                response.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1]); // 1 question, 1 additional
                response.extend_from_slice(&query[12..end]);
                // The queried name (pointer to the question), A, IN, TTL 60, 127.0.0.1
                response
                    .extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 127, 0, 0, 1]);
                let _ = socket.send_to(&response, peer);
            }
        });
        address
    }

    /// Transport whose dials panic, like libp2p-dns does on some DNS responses.
    struct PanickingTransport;

    impl libp2p::Transport for PanickingTransport {
        type Output = ();
        type Error = dns::Error<io::Error>;
        type ListenerUpgrade = future::Pending<Result<(), Self::Error>>;
        type Dial = BoxFuture<'static, Result<(), Self::Error>>;

        fn listen_on(
            &mut self,
            _: ListenerId,
            addr: Multiaddr,
        ) -> Result<(), TransportError<Self::Error>> {
            Err(TransportError::MultiaddrNotSupported(addr))
        }

        fn remove_listener(&mut self, _: ListenerId) -> bool {
            false
        }

        fn dial(
            &mut self,
            _: Multiaddr,
            _: DialOpts,
        ) -> Result<Self::Dial, TransportError<Self::Error>> {
            Ok(
                async { panic!("If there are no results, `Err(NoRecordsFound)` is expected.") }
                    .boxed(),
            )
        }

        fn poll(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
            Poll::Pending
        }
    }

    #[test(tokio::test)]
    async fn dial_panic_becomes_dial_error() {
        let mut transport = super::Transport::new(PanickingTransport);
        let dial = transport
            .dial("/dns4/peer.local/tcp/443/wss".parse().unwrap(), dial_opts())
            .unwrap();

        assert!(matches!(dial.await, Err(dns::Error::Transport(_))));
    }

    /// libp2p-dns 0.45.0 panics on a lookup without a matching answer. When this test fails
    /// because the dial returns a `ResolveError` instead, libp2p-dns no longer panics and the
    /// wrapper can go.
    #[test(tokio::test)]
    async fn lookup_without_answers_becomes_dial_error() {
        let server = start_dns_server_without_answers();
        let mut name_server = NameServerConfig::udp(server.ip());
        name_server.connections[0].port = server.port();
        let config = dns::ResolverConfig::from_name_servers(vec![name_server]);
        let mut opts = dns::ResolverOpts::default();
        opts.attempts = 1;
        opts.timeout = Duration::from_secs(5);

        let mut transport = super::Transport::new(dns::tokio::Transport::custom(
            tcp::tokio::Transport::new(tcp::Config::default()),
            config,
            opts,
        ));

        for address in ["/dns4/peer.test./tcp/1", "/dns/peer.test./tcp/1"] {
            let dial = transport
                .dial(address.parse().unwrap(), dial_opts())
                .unwrap();
            match dial.await {
                Err(dns::Error::Transport(error)) => {
                    assert_eq!(error.to_string(), "dialing panicked", "{address}")
                }
                Err(error) => panic!("{address}: expected the dial to panic, got {error}"),
                Ok(_) => panic!("{address}: expected the dial to panic, but it connected"),
            }
        }
    }
}
