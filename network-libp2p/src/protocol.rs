use std::{io, iter};

use futures::future::BoxFuture;
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt, FutureExt};
use libp2p::core::UpgradeInfo;
use libp2p::{InboundUpgrade, OutboundUpgrade};

use beserial::SerializingError;
use network_interface::message;

pub struct NimiqProtocol {
    msg: Option<Vec<u8>>,
}

impl NimiqProtocol {
    pub(crate) fn inbound() -> Self {
        Self { msg: None }
    }

    pub(crate) fn outbound(msg: Vec<u8>) -> Self {
        NimiqProtocol { msg: Some(msg) }
    }
}

impl UpgradeInfo for NimiqProtocol {
    type Info = &'static [u8];
    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once(b"/test/1.0.0")
    }
}

impl<C> InboundUpgrade<C> for NimiqProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = Vec<u8>;
    type Error = SerializingError;
    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, _info: Self::Info) -> Self::Future {
        // TODO close socket?
        //socket.close().await?;
        message::read_message(socket).boxed()
    }
}

impl<C> OutboundUpgrade<C> for NimiqProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = ();
    type Error = io::Error;
    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, mut socket: C, _info: Self::Info) -> Self::Future {
        async move {
            if let Some(msg) = self.msg {
                socket.write_all(&msg[..]).await?;
            }
            socket.close().await?;
            Ok(())
        }
        .boxed()
    }
}
