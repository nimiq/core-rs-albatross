mod reader;
mod writer;
mod header;

pub use self::reader::MessageReader;
pub use self::writer::MessageWriter;


#[cfg(test)]
mod tests {
    use beserial::{Serialize, Deserialize};
    use futures::{
        io::Cursor,
        SinkExt, StreamExt
    };

    use super::{MessageReader, MessageWriter};

    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    struct TestMessage {
        pub foo: u32,
        #[beserial(len_type(u8))]
        pub bar: String,
    }

    #[tokio::test]
    async fn it_writes_and_reads_messages() {
        let mut buf = vec![];

        let message = TestMessage { foo: 42, bar: "Hello World".to_owned() };

        MessageWriter::new(&mut buf).send(&message).await.unwrap();

        let received = MessageReader::new(Cursor::new(buf)).next().await.unwrap().unwrap();

        assert_eq!(message, received);
    }
}
