use std::{
    marker::{PhantomData, Unpin, PhantomPinned},
    pin::Pin,
};

use futures::{
    task::{Context, Poll},
    AsyncRead, Stream, ready,
};
use bytes::{BytesMut, BufMut, Buf};
use pin_project::pin_project;

use beserial::{SerializingError, Deserialize};



/// Try to read, such that at most `n` bytes are in the buffer. This will return `Poll::Pending` until the buffer
/// has `n` bytes in it. This returns `Poll::Ready(Ok(false))` in case of EOF.
fn read_to_buf<R>(reader: Pin<&mut R>, buffer: &mut BytesMut, n: usize, cx: &mut Context<'_>) -> Poll<Result<bool, std::io::Error>>
    where
        R: AsyncRead,
{
    // Current length of buffer
    let n0 = buffer.len();

    if n > n0 {
        buffer.resize(n, 0);

        log::trace!("MessageReader: read_to_buf: n={}, n0={}", n, n0);

        match AsyncRead::poll_read(reader, cx, &mut buffer[n0 .. n]) {
            // EOF
            Poll::Ready(Ok(0)) => {
                log::trace!("MessageReader: read_to_buf: poll_read returned 0");

                buffer.resize(n0, 0);
                Poll::Ready(Ok(false))
            },

            // Data was read
            Poll::Ready(Ok(n_read)) => {
                log::trace!("MessageReader: read_to_buf: Received {} bytes, buffer={:?}", n_read, buffer);

                // New length of buffer
                let n_new = n0 + n_read;

                if n_new < n {
                    // We didn't read all the bytes, so let's resize the buffer accordingly
                    buffer.resize(n_new, 0);

                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                else {
                    Poll::Ready(Ok(true))
                }
            },

            // An error occured
            Poll::Ready(Err(e)) => {
                panic!("MessageReader: read_to_buf: poll_read failed: {}", e);
                Poll::Ready(Err(e))
            },

            // Reader is not ready
            Poll::Pending => {
                log::trace!("MessageReader: read_to_buf: poll_read pending");

                buffer.resize(n0, 0);
                Poll::Pending
            },
        }
    }
    else {
        Poll::Ready(Ok(true))
    }
}


/// TODO: Generalize over a type `H: Header`, which is `Deserialize` and has a getter for the length of the message.
#[derive(Clone, Copy, Debug)]
enum ReaderState {
    Head,
    Data(usize),
}


#[pin_project]
pub struct MessageReader<R, M>
    where
        R: AsyncRead,
        M: Deserialize,
{
    #[pin]
    inner: R,

    state: ReaderState,

    buffer: BytesMut,

    _message_type: PhantomData<M>,
}

impl<R, M> MessageReader<R, M>
    where
        R: AsyncRead,
        M: Deserialize,
{
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            state: ReaderState::Head,
            buffer: BytesMut::with_capacity(1024), // TODO: initial size?
            _message_type: PhantomData,
        }
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R, M> Stream for MessageReader<R, M>
    where
        R: AsyncRead,
        M: Deserialize + std::fmt::Debug,
{
    type Item = Result<M, SerializingError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let self_projected = self.project();

        log::trace!("MessageReader::poll_next: buffer={:?}", self_projected.buffer);

        match *self_projected.state {
            ReaderState::Head => {
                // Read header. This returns `Poll::Pending` until all header bytes have been read.
                match read_to_buf(self_projected.inner, self_projected.buffer, 2, cx) {
                    // Wait for more data.
                    Poll::Pending => return Poll::Pending,

                    // An error occured.
                    Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e.into()))),

                    // EOF while reading the header.
                    Poll::Ready(Ok(false)) => {
                        if self_projected.buffer.is_empty() {
                            // No partial message, so this is the end of the stream
                            return Poll::Ready(None)
                        }
                        else {
                            panic!("Unexpected EOF");
                            return Poll::Ready(Some(Err(SerializingError::from(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)))))
                        }
                    },

                    // Finished reading the header, and we didn't reach EOF.
                    Poll::Ready(Ok(true)) => {},
                }

                // Decode the header: 16 bit length big-endian
                // This will also advance the read position after the header.
                let n = self_projected.buffer.get_u16();

                // Change reader state to read the data next.
                *self_projected.state = ReaderState::Data(n as usize);

                // Future is still not ready
                cx.waker().wake_by_ref();
                Poll::Pending
            },
            ReaderState::Data(n) => {
                // Read data. This returns `Poll::Pending` until all data bytes have been read.
                // The argument to `read_to_buf` is `n + 2`, because it takes the expected number of bytes read in
                // total, which includes the header.
                match read_to_buf(self_projected.inner, self_projected.buffer, n, cx) {
                    // Wait for more data.
                    Poll::Pending => return Poll::Pending,

                    // An error occured.
                    Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e.into()))),

                    // EOF while reading the data.
                    Poll::Ready(Ok(false)) => return Poll::Ready(Some(Err(SerializingError::from(std::io::Error::from(std::io::ErrorKind::UnexpectedEof, ))))),

                    // Finished reading the message
                    Poll::Ready(Ok(true)) => (),
                }

                // Decode the message, the read position of the buffer is already at the start of the message.
                let message = match Deserialize::deserialize(&mut self_projected.buffer.reader()) {
                    Ok(message) => message,
                    Err(e) => {
                        panic!("MessageReader: Deserialization failed: {}", e);
                        return Poll::Ready(Some(Err(e)))
                    },
                };

                // Reset the reader state to read a header next.
                *self_projected.state = ReaderState::Head;

                // Reset the buffer
                self_projected.buffer.clear();

                log::trace!("MessageReader: Received message: {:?}", message);

                return Poll::Ready(Some(Ok(message)));
            },
        }
    }
}


#[cfg(test)]
mod tests {
    use futures::{
        io::Cursor,
        StreamExt,
    };

    use beserial::{Serialize, Deserialize};
    use bytes::{BytesMut, BufMut};

    use super::MessageReader;


    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    struct TestMessage {
        pub foo: u32,
        #[beserial(len_type(u8))]
        pub bar: String,
    }

    fn put_message<M: Serialize>(buf: &mut BytesMut, message: &M) {
        let n = message.serialized_size();
        buf.reserve(n);
        buf.put_u16(n as u16);
        message.serialize( &mut buf.writer()).unwrap();
    }

    #[tokio::test]
    pub async fn it_can_read_a_message() {
        let test_message = TestMessage {
            foo: 42,
            bar: "Hello World".to_owned(),
        };

        let mut data = BytesMut::new();
        put_message(&mut data, &test_message);
        let mut reader = MessageReader::new(Cursor::new(&data));

        assert_eq!(reader.next().await, Some(Ok(test_message)));
        assert_eq!(reader.next().await, None);
    }

    #[tokio::test]
    pub async fn it_can_read_multiple_messages() {
        let m1 = TestMessage {
            foo: 42,
            bar: "Hello World".to_owned(),
        };
        let m2 = TestMessage {
            foo: 420,
            bar: "foobar".to_owned(),
        };

        let mut data = BytesMut::new();
        put_message(&mut data, &m1);
        put_message(&mut data, &m2);

        let mut reader = MessageReader::new(Cursor::new(&data));

        assert_eq!(reader.next().await, Some(Ok(m1)));
        assert_eq!(reader.next().await, Some(Ok(m2)));
        assert_eq!(reader.next().await, None);
    }
}
