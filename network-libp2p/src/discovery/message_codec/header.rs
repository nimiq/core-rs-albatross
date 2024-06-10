use nimiq_serde::{fixint, Deserialize, Serialize, SerializedSize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    #[serde(with = "fixint::be")]
    pub magic: u32,
    #[serde(with = "fixint::be")]
    pub size: u32,
    #[serde(with = "fixint::be")]
    pub checksum: u32,
}

impl SerializedSize for Header {
    const SIZE: usize = 12;
}

impl Header {
    pub const MAGIC: u32 = 0x4204_2042;

    pub fn new(size: u32) -> Self {
        Self {
            magic: Self::MAGIC,
            size,
            checksum: 0,
        }
    }
}

impl Default for Header {
    fn default() -> Self {
        Header::new(0)
    }
}
