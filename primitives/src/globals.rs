use once_cell::sync::OnceCell;

use crate::networks::NetworkId;

pub static NETWORK_ID: OnceCell<NetworkId> = OnceCell::new();
