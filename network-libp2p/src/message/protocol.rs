use std::{iter, sync::Arc};

use futures::{future, AsyncRead, AsyncWrite};
use libp2p::{core::UpgradeInfo, InboundUpgrade, OutboundUpgrade};

use beserial::SerializingError;

use super::{dispatch::MessageDispatch, peer::Peer};
use crate::MESSAGE_PROTOCOL;

#[derive(Debug, Default)]
pub struct MessageProtocol {
    peer: Option<Arc<Peer>>,
}

impl MessageProtocol {
    const BUFFER_SIZE: usize = 16;
}

impl UpgradeInfo for MessageProtocol {
    type Info = &'static [u8];
    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once(MESSAGE_PROTOCOL)
    }
}

impl<C> InboundUpgrade<C> for MessageProtocol
where
    C: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
{
    type Output = MessageDispatch<C>;
    type Error = SerializingError;
    type Future = future::Ready<Result<MessageDispatch<C>, SerializingError>>;

    fn upgrade_inbound(self, socket: C, _info: Self::Info) -> Self::Future {
        future::ok(MessageDispatch::new(socket, Self::BUFFER_SIZE))
    }
}

impl<C> OutboundUpgrade<C> for MessageProtocol
where
    C: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
{
    type Output = MessageDispatch<C>;
    type Error = SerializingError;
    type Future = future::Ready<Result<MessageDispatch<C>, SerializingError>>;

    fn upgrade_outbound(self, socket: C, _info: Self::Info) -> Self::Future {
        future::ok(MessageDispatch::new(socket, Self::BUFFER_SIZE))
    }
}
