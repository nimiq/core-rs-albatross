use beserial::{Serialize, Deserialize};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Protocol {
    Dumb = 0,
    Wss = 1,
    Rtc = 2,
    Ws = 4
}

impl From<ProtocolFlags> for Vec<Protocol> {
    fn from(flags: ProtocolFlags) -> Self {
        let mut v = Vec::new();
        if flags.contains(ProtocolFlags::DUMB) {
            v.push(Protocol::Dumb);
        }
        if flags.contains(ProtocolFlags::WSS) {
            v.push(Protocol::Wss);
        }
        if flags.contains(ProtocolFlags::RTC) {
            v.push(Protocol::Rtc);
        }
        if flags.contains(ProtocolFlags::WS) {
            v.push(Protocol::Ws);
        }
        v
    }
}

bitflags! {
    #[derive(Default, Serialize, Deserialize)]
    pub struct ProtocolFlags: u8 {
        const DUMB  = 0b00000000;
        const WSS  = 0b00000001;
        const RTC = 0b00000010;
        const WS  = 0b00000100;
    }
}

impl From<Protocol> for ProtocolFlags {
    fn from(protocol: Protocol) -> Self {
        match protocol {
            Protocol::Dumb => ProtocolFlags::DUMB,
            Protocol::Rtc => ProtocolFlags::RTC,
            Protocol::Wss => ProtocolFlags::WSS,
            Protocol::Ws => ProtocolFlags::WS,
        }
    }
}

impl From<Vec<Protocol>> for ProtocolFlags {
    fn from(protocols: Vec<Protocol>) -> Self {
        let mut flags = ProtocolFlags::default();
        for protocol in protocols {
            flags |= ProtocolFlags::from(protocol);
        }
        flags
    }
}

impl Protocol {
    pub fn default_port(&self) -> Option<u16> {
        match self {
            Protocol::Ws | Protocol::Wss => Some(8443),
            _ => None
        }
    }
}
