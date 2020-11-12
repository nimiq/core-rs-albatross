use std::{
    sync::Arc,
    io, iter,
};

use futures::{
    channel::mpsc,
    future, AsyncRead, AsyncWrite, AsyncWriteExt, FutureExt,
};
use libp2p::{
    core::UpgradeInfo,
    InboundUpgrade, OutboundUpgrade,
};

use beserial::SerializingError;
use nimiq_network_interface::message;

use crate::MESSAGE_PROTOCOL;
use super::{
    dispatch::{MessageSender, MessageReceiver},
    peer::Peer,
};


#[derive(Default)]
pub struct MessageProtocol {
    peer: Option<Arc<Peer>>,
}

impl MessageProtocol {
    fn peer(&self) -> Arc<Peer> {
        Arc::clone(self.peer.as_ref().unwrap())
    }
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
    type Output = MessageReceiver;
    type Error = SerializingError;
    type Future = future::Ready<Result<MessageReceiver, SerializingError>>;

    fn upgrade_inbound(self, socket: C, _info: Self::Info) -> Self::Future {
        log::debug!("MessageProtocol::upgrade_inbound");
        future::ok(MessageReceiver::new(socket))
    }
}

impl<C> OutboundUpgrade<C> for MessageProtocol
    where
        C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = MessageSender<C>;
    type Error = SerializingError;
    type Future = future::Ready<Result<MessageSender<C>, SerializingError>>;

    fn upgrade_outbound(self, mut socket: C, _info: Self::Info) -> Self::Future {
        log::debug!("MessageProtocol::upgrade_outbound");
        future::ok(MessageSender::new(socket))
    }
}
