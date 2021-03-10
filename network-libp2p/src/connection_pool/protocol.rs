use futures::{
    future,
    io::{AsyncRead, AsyncWrite},
};
use libp2p::{core::UpgradeInfo, InboundUpgrade, OutboundUpgrade};

use crate::CONNECTION_POOL_PROTOCOL;

pub struct ConnectionPoolProtocol;

impl UpgradeInfo for ConnectionPoolProtocol {
    type Info = &'static [u8];
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(CONNECTION_POOL_PROTOCOL)
    }
}

impl<C> InboundUpgrade<C> for ConnectionPoolProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = ();
    type Error = std::io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, _socket: C, info: Self::Info) -> Self::Future {
        log::debug!("ConnectionPoolProtocol::upgrade_inbound: {:?}", info);
        future::ok(())
    }
}

impl<C> OutboundUpgrade<C> for ConnectionPoolProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = ();
    type Error = std::io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, _socket: C, info: Self::Info) -> Self::Future {
        log::debug!("ConnectionPoolProtocol::upgrade_outbound: {:?}", info);
        future::ok(())
    }
}
