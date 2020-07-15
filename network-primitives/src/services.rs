use beserial::{Deserialize, Serialize};

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct ServiceFlags: u32 {
        const NONE  = 0b0000_0000;
        const NANO  = 0b0000_0001;
        const LIGHT = 0b0000_0010;
        const FULL  = 0b0000_0100;
        // Node supports validator protocol
        const VALIDATOR  = 0b0100_0000_0000;
    }
}

impl ServiceFlags {
    pub fn is_full_node(self) -> bool {
        self.contains(ServiceFlags::FULL)
    }

    pub fn is_light_node(self) -> bool {
        self.contains(ServiceFlags::LIGHT)
    }

    pub fn is_nano_node(self) -> bool {
        self.contains(ServiceFlags::NANO)
    }

    pub fn is_validator(self) -> bool {
        self.contains(ServiceFlags::VALIDATOR)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Services {
    pub provided: ServiceFlags,
    pub accepted: ServiceFlags,
}

impl Services {
    pub fn new(provided: ServiceFlags, accepted: ServiceFlags) -> Self {
        Services { provided, accepted }
    }

    pub fn full() -> Self {
        Services {
            provided: ServiceFlags::FULL,
            accepted: ServiceFlags::FULL,
        }
    }
}
