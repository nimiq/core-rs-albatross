use beserial::{Deserialize, Serialize};

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct ServiceFlags: u32 {
        const NONE  = 0b0000_0000;
        const NANO  = 0b0000_0001;
        const LIGHT = 0b0000_0010;
        const FULL  = 0b0000_0100;

        // Albatross specific services

        // the node supports multicast for participation in block production
        // i.e. FindNode, ViewChange, Pbft*
        const MULTICAST  = 0b0001_0000;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Services {
    pub provided: ServiceFlags,
    pub accepted: ServiceFlags,
}

impl Services {
    pub fn new(provided: ServiceFlags, accepted: ServiceFlags) -> Self {
        Services {
            provided,
            accepted
        }
    }

    pub fn full() -> Self {
        Services {
            provided: ServiceFlags::FULL,
            accepted: ServiceFlags::FULL,
        }
    }
}
