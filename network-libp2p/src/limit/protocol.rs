use libp2p::{
    core::UpgradeInfo,
    InboundUpgrade, OutboundUpgrade, Multiaddr,
};
use futures::{
    io::{AsyncRead, AsyncWrite},
    future,
};

use beserial::{SerializingError, Serialize, Deserialize};
use nimiq_macros::{create_typed_array, add_hex_io_fns_typed_arr};
use nimiq_hash::Blake2bHash;

use crate::{
    message_codec::{MessageReader, MessageWriter},
    tagged_signing::{TaggedSignature, TaggedSignable},
    LIMIT_PROTOCOL,
};


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
