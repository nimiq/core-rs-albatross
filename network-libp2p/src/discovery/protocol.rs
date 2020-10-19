use std::collections::HashSet;

use libp2p::{swarm::ProtocolsHandler, core::UpgradeInfo, InboundUpgrade, OutboundUpgrade};
use futures::{
    io::{AsyncRead, AsyncWrite},
    future,
};
use beserial::{SerializingError, Serialize, Deserialize};
use nimiq_peer_address::services::ServiceFlags;

use crate::message::{MessageReader, MessageWriter};
use super::peer_address_book::{PeerContact, Services, Protocols};


#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum DiscoveryMessage {
    #[beserial(discriminant = 1)]
    PeerAddresses {
        #[beserial(len_type(u16))]
        peers: Vec<PeerContact>,
    },

    #[beserial(discriminant = 2)]
    GetPeerAddresses {
        limit: Option<u16>,
        services: Services,
        protocols: Protocols,
    }
}


pub struct DiscoveryProtocol;

impl UpgradeInfo for DiscoveryProtocol {
    type Info = &'static [u8];
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(b"/nimiq/discovery/0.0.1")
    }
}

impl<C> InboundUpgrade<C> for DiscoveryProtocol
    where
        C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = MessageReader<C, DiscoveryMessage>;
    type Error = SerializingError;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, info: Self::Info) -> Self::Future {
        future::ok(MessageReader::new(socket))
    }
}

impl<C> OutboundUpgrade<C> for DiscoveryProtocol
    where
        C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = MessageWriter<C, DiscoveryMessage>;
    type Error = SerializingError;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, info: Self::Info) -> Self::Future {
        future::ok(MessageWriter::new(socket))
    }
}
