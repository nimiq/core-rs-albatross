pub mod address;
pub mod message;

use beserial::{Serialize, SerializeWithLength, Deserialize, DeserializeWithLength, ReadBytesExt, WriteBytesExt};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Protocol {
    Dumb = 0,
    Wss = 1,
    Rtc = 2,
    Ws = 4
}
