use std::{
    marker::PhantomData,
    pin::Pin,
};

use futures::{
    task::{Context, Poll},
    AsyncWrite, Sink, ready,
};
use pin_project::pin_project;

use beserial::{Serialize, SerializingError};
use bytes::{BytesMut, Buf, BufMut};


fn write_from_buf<'w, W>(inner: &mut W, buffer: &mut BytesMut, cx: &mut Context) -> Poll<Result<(), SerializingError>>
    where
        W: AsyncWrite + Unpin
{
    if buffer.remaining() > 0 {
        match Pin::new(inner).poll_write(cx, buffer.bytes()) {
            Poll::Ready(Ok(0)) => Poll::Ready(Err(SerializingError::from(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)))),
            Poll::Ready(Ok(n)) => {
                buffer.advance(n);
                if buffer.remaining() > 0 {
                    Poll::Pending
                }
                else {
                    buffer.clear();
                    Poll::Ready(Ok(()))
                }
            },
            Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
            Poll::Pending => Poll::Pending,
        }
    }
    else {
        Poll::Ready(Ok(()))
    }
}


#[pin_project]
pub struct MessageWriter<W, M>
    where
        W: AsyncWrite,
        M: Serialize,
{
    inner: W,
    buffer: BytesMut,
    _message_type: PhantomData<M>,
}

impl<W, M> MessageWriter<W, M>
    where
        W: AsyncWrite,
        M: Serialize,
{
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
        M: Serialize,
{
    type Error = SerializingError;

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

        // Reserve space for the header and message.
        // TODO: Allow for a generic header struct.
        let n = Serialize::serialized_size(item);
        self_projected.buffer.reserve(n + 2);

        // Write header (i.e. length of message)
        self_projected.buffer.put_u16(n as u16);

        // Serialize the message into the buffer.
        Serialize::serialize(item, &mut self_projected.buffer.writer())?;

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
                Poll::Ready(ready!(Pin::new(self_projected.inner).poll_flush(cx))
                    .map_err(|e| e.into()))
            },
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
                Poll::Ready(ready!(Pin::new(self_projected.inner).poll_close(cx))
                    .map_err(|e| e.into()))
            },
        }
    }
}


#[cfg(test)]
mod tests {
    use futures::SinkExt;

    use beserial::{Serialize, Deserialize};

    use super::MessageWriter;


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

        let n = test_message.serialized_size();
        message_writer.send(&test_message).await.unwrap();

        let data = message_writer.into_inner();

        assert_eq!(data[0], 0);
        assert_eq!(data[1], n as u8);

        assert_eq!(&test_message.serialize_to_vec()[..], &data[2..])
    }
}