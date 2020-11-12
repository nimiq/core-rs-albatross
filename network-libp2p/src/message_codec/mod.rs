mod reader;
mod writer;
mod header;

pub use self::reader::MessageReader;
pub use self::writer::MessageWriter;


#[cfg(test)]
mod tests {
    use futures::SinkExt;

    use beserial::{Serialize, Deserialize};

    use crate::message_codec::header::Header;
    use super::{MessageReader, MessageWriter};

    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    struct TestMessage {
        pub foo: u32,
        #[beserial(len_type(u8))]
        pub bar: String,
    }

    #[test]
    fn it_writes_and_reads_messages() {

    }
}