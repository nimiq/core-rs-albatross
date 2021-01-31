use std::{marker::PhantomData, pin::Pin};

use bytes::{Buf, BufMut, BytesMut};
use futures::{
    ready,
    task::{Context, Poll},
    AsyncWrite, Sink,
};
use pin_project::pin_project;

use beserial::{Serialize, SerializingError};

use super::header::Header;

fn write_from_buf<'w, W>(inner: &mut W, buffer: &mut BytesMut, cx: &mut Context) -> Poll<Result<(), SerializingError>>
where
    W: AsyncWrite + Unpin,
{
    if buffer.remaining() > 0 {
        match Pin::new(inner).poll_write(cx, buffer.bytes()) {
            Poll::Ready(Ok(0)) => {
                log::warn!("MessageWriter: write_from_buf: Unexpected EOF.");
                Poll::Ready(Err(SerializingError::from(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))))
            }

            Poll::Ready(Ok(n)) => {
                //log::trace!("MessageWriter: write_from_buf: {} bytes written", n);
                buffer.advance(n);
                if buffer.remaining() > 0 {
                    Poll::Pending
                } else {
                    buffer.clear();
                    Poll::Ready(Ok(()))
                }
            }

            Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),

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
    type Error = SerializingError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let self_projected = self.project();

        //log::trace!("MessageWriter: poll_ready");

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
            log::warn!("MessageWriter: Trying to send while buffer is not empty.");
            return Err(SerializingError::from(std::io::Error::from(std::io::ErrorKind::WouldBlock)));
        }

        //log::trace!("MessageWriter: Sending {:?}", item);
        //log::trace!("MessageWriter: serialized_size = {}", item.serialized_size());

        // Reserve space for the header and message.
        let n = Serialize::serialized_size(item);
        self_projected.buffer.reserve(n + Header::SIZE);

        let header = Header::new(n as u32);

        let mut w = self_projected.buffer.writer();

        // Write header
        Serialize::serialize(&header, &mut w)?;

        // Serialize the message into the buffer.
        Serialize::serialize(item, &mut w)?;

        //log::trace!("MessageWriter: buffer = {:?}", self_projected.buffer);

        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let self_projected = self.project();

        //log::trace!("MessageWriter: poll_flush called.");

        // Try to finish writing from buffer to the inner `AsyncWrite`
        match write_from_buf(self_projected.inner, self_projected.buffer, cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {
                // Finished writing the message. Flush the underlying `AsyncWrite`.
                Poll::Ready(ready!(Pin::new(self_projected.inner).poll_flush(cx)).map_err(|e| e.into()))
            }
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let self_projected = self.project();

        //log::trace!("MessageWriter: poll_close called.");

        // Try to finish writing from buffer to the inner `AsyncWrite`
        match write_from_buf(self_projected.inner, self_projected.buffer, cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {
                // Finished writing the message. Close the underlying `AsyncWrite`.
                Poll::Ready(ready!(Pin::new(self_projected.inner).poll_close(cx)).map_err(|e| e.into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::SinkExt;

    use beserial::{Deserialize, Serialize};

    use super::MessageWriter;
    use crate::message_codec::header::Header;

    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    struct TestMessage {
        pub foo: u32,
        #[beserial(len_type(u8))]
        pub bar: String,
    }

    #[tokio::test]
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
