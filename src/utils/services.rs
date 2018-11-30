use beserial::{Deserialize, Serialize};

bitflags! {
    #[derive(Default, Serialize, Deserialize)]
    pub struct ServiceFlags: u32 {
        const NONE  = 0b00000000;
        const NANO  = 0b00000001;
        const LIGHT = 0b00000010;
        const FULL  = 0b00000100;
    }
}

impl ServiceFlags {
    pub fn is_full_node(&self) -> bool {
        self.contains(ServiceFlags::FULL)
    }

    pub fn is_light_node(&self) -> bool {
        self.contains(ServiceFlags::LIGHT)
    }

    pub fn is_nano_node(&self) -> bool {
        self.contains(ServiceFlags::NANO)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
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

    pub fn default() -> Self {
        Services {
            provided: ServiceFlags::FULL,
            accepted: ServiceFlags::FULL,
        }
    }
}
