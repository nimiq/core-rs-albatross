use futures::{
    future,
    io::{AsyncRead, AsyncWrite},
};
use libp2p::{core::UpgradeInfo, InboundUpgrade, OutboundUpgrade};

use crate::LIMIT_PROTOCOL;

pub struct LimitProtocol;

impl UpgradeInfo for LimitProtocol {
    type Info = &'static [u8];
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(LIMIT_PROTOCOL)
    }
}

impl<C> InboundUpgrade<C> for LimitProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = ();
    type Error = std::io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, info: Self::Info) -> Self::Future {
        log::debug!("LimitProtocol::upgrade_inbound: {:?}", info);
        future::ok(())
    }
}

impl<C> OutboundUpgrade<C> for LimitProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = ();
    type Error = std::io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, info: Self::Info) -> Self::Future {
        log::debug!("DiscoveryProtocol::upgrade_outbound: {:?}", info);
        future::ok(())
    }
}
