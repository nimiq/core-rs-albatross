use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, BufMut, BytesMut};
use futures::{ready, AsyncWrite, Sink};
use nimiq_serde::{Serialize, SerializedSize as _};
use pin_project::pin_project;

use super::header::Header;

fn write_from_buf<W>(
    inner: &mut W,
    buffer: &mut BytesMut,
    cx: &mut Context,
) -> Poll<Result<(), std::io::Error>>
where
    W: AsyncWrite + Unpin,
{
    if buffer.remaining() > 0 {
        match Pin::new(inner).poll_write(cx, buffer.chunk()) {
            Poll::Ready(Ok(0)) => {
                warn!("MessageWriter: write_from_buf: Unexpected EOF.");
                Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)))
            }

            Poll::Ready(Ok(n)) => {
                buffer.advance(n);
                if buffer.remaining() > 0 {
                    Poll::Pending
                } else {
                    buffer.clear();
                    Poll::Ready(Ok(()))
                }
            }

            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),

            Poll::Pending => Poll::Pending,
        }
    } else {
        Poll::Ready(Ok(()))
    }
}

#[pin_project]
pub struct MessageWriter<W, M> {
    inner: W,
    buffer: BytesMut,
    _message_type: PhantomData<M>,
}

impl<W, M> MessageWriter<W, M> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            buffer: BytesMut::new(),
            _message_type: PhantomData,
        }
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W, M> Sink<&M> for MessageWriter<W, M>
where
    W: AsyncWrite + Unpin,
    M: Serialize + std::fmt::Debug,
{
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let self_projected = self.project();

        // Try to write from buffer to the inner `AsyncWrite`
        match write_from_buf(self_projected.inner, self_projected.buffer, cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
        }
    }

    fn start_send(self: Pin<&mut Self>, item: &M) -> Result<(), Self::Error> {
        let self_projected = self.project();

        if !self_projected.buffer.is_empty() {
            warn!("MessageWriter: Trying to send while buffer is not empty");
            return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
        }

        // Reserve space for the header and message.
        let n = Serialize::serialized_size(item);
        self_projected.buffer.reserve(n + Header::SIZE);

        let header = Header::new(n as u32);

        let mut w = self_projected.buffer.writer();

        // Write header
        Serialize::serialize_to_writer(&header, &mut w)?;

        // Serialize the message into the buffer.
        Serialize::serialize_to_writer(item, &mut w)?;

        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let self_projected = self.project();

        // Try to finish writing from buffer to the inner `AsyncWrite`
        match write_from_buf(self_projected.inner, self_projected.buffer, cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {
                // Finished writing the message. Flush the underlying `AsyncWrite`.
                Poll::Ready(ready!(Pin::new(self_projected.inner).poll_flush(cx)))
            }
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let self_projected = self.project();

        // Try to finish writing from buffer to the inner `AsyncWrite`
        match write_from_buf(self_projected.inner, self_projected.buffer, cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {
                // Finished writing the message. Close the underlying `AsyncWrite`.
                Poll::Ready(ready!(Pin::new(self_projected.inner).poll_close(cx)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::SinkExt;
    use nimiq_serde::{Deserialize, Serialize, SerializedSize};
    use nimiq_test_log::test;

    use super::{Header, MessageWriter};

    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    struct TestMessage {
        pub foo: u32,
        pub bar: String,
    }

    #[test(tokio::test)]
    pub async fn it_can_write_a_message() {
        let test_message = TestMessage {
            foo: 42,
            bar: "Hello World".to_owned(),
        };

        let mut message_writer = MessageWriter::new(vec![]);

        message_writer.send(&test_message).await.unwrap();

        let data = message_writer.into_inner();

        assert_eq!(&test_message.serialize_to_vec(), &data[Header::SIZE..])
    }
}
