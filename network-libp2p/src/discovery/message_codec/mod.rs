//! This module contains a `Reader` and a `Writer` for the NIMIQ discovery
//! messages. This messages have a fixed header, defined in `Header` with a
//! magic number and some auxiliary information. The body of the message can
//! be arbitrary bytes which are later serialized/deserialized.
//!

mod header;
mod reader;
mod writer;

pub use self::{reader::MessageReader, writer::MessageWriter};

#[cfg(test)]
mod tests {
    use futures::{io::Cursor, SinkExt, StreamExt};
    use nimiq_test_log::test;
    use serde::{Deserialize, Serialize};

    use super::{MessageReader, MessageWriter};

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
}
