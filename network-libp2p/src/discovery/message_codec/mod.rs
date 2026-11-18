//! This module contains a `Reader` and a `Writer` for the NIMIQ discovery
//! messages. This messages have a fixed header, defined in `Header` with a
//! magic number and some auxiliary information. The body of the message can
//! be arbitrary bytes which are later serialized/deserialized.
//!

mod header;
mod reader;
mod writer;

pub use self::{reader::MessageReader, writer::MessageWriter};

/// Maximum size of a message (excluding the header).
pub const MAX_MESSAGE_SIZE: usize = 1_000_000;

#[cfg(test)]
mod tests {
    use futures::{io::Cursor, SinkExt, StreamExt};
    use nimiq_serde::Serialize as _;
    use nimiq_test_log::test;
    use serde::{Deserialize, Serialize};

    use super::{MessageReader, MessageWriter, MAX_MESSAGE_SIZE};

    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    struct TestMessage {
        pub foo: u32,
        pub bar: String,
    }

    #[test(tokio::test)]
    async fn it_writes_and_reads_messages() {
        let mut buf = vec![];

        let message = TestMessage {
            foo: 42,
            bar: "Hello World".to_owned(),
        };

        MessageWriter::new(&mut buf).send(&message).await.unwrap();

        let received = MessageReader::new(Cursor::new(buf))
            .next()
            .await
            .unwrap()
            .unwrap();

        assert_eq!(message, received);
    }

    #[test(tokio::test)]
    async fn it_writes_and_reads_messages_of_maximum_size() {
        let mut buf = vec![];

        // `foo` takes 1 byte and the length prefix of `bar` 3 bytes
        let message = TestMessage {
            foo: 42,
            bar: "a".repeat(MAX_MESSAGE_SIZE - 4),
        };
        assert_eq!(message.serialized_size(), MAX_MESSAGE_SIZE);

        MessageWriter::new(&mut buf).send(&message).await.unwrap();

        let received = MessageReader::new(Cursor::new(buf))
            .next()
            .await
            .unwrap()
            .unwrap();

        assert_eq!(message, received);
    }
}
