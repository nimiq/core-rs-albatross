use std::collections::{HashMap, HashSet};

use libp2p::Multiaddr;

/// The NAT status an address can have
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) enum NatStatus {
    /// The address is publicly reachable
    Public,
    /// The address is not publicly reachable
    Private,
    /// The reachability of the address is unknown
    #[default]
    Unknown,
}

/// The NAT state of the local peer
#[derive(Default)]
pub(crate) struct NatState {
    /// The addresses that are confirmed thus publicly reachable
    confirmed_addresses: HashSet<Multiaddr>,
    /// The list of discovered local addresses
    address_status: HashMap<Multiaddr, NatStatus>,
    /// The NAT status of the local peer
    status: NatStatus,
}

impl NatState {
    /// Adds an address to track its NAT status
    pub fn add_address(&mut self, address: Multiaddr) {
        self.address_status.insert(address, NatStatus::Unknown);
    }

    /// Remove address and no longer track its NAT status
    pub fn remove_address(&mut self, address: &Multiaddr) {
        self.address_status.remove(address);
        self.confirmed_addresses.remove(address);
        self.update_state();
    }

    /// Set the NAT status of address
    pub fn set_address_nat(&mut self, address: Multiaddr, nat_status: NatStatus) {
        let address_status = self
            .address_status
            .entry(address.clone())
            .or_insert(nat_status);

        if *address_status == NatStatus::Public {
            self.confirmed_addresses.insert(address);
        } else {
            self.confirmed_addresses.remove(&address);
        }
        self.update_state();
    }

    /// Mark the address as confirmed thus publicly reachable
    pub fn add_confirmed_address(&mut self, address: Multiaddr) {
        let address_status = self.address_status.entry(address.clone()).or_default();
        *address_status = NatStatus::Public;

        self.confirmed_addresses.insert(address);
        self.update_state();
    }

    /// External address expired thus no longer publicly reachable
    pub fn remove_confirmed_address(&mut self, address: &Multiaddr) {
        let address_status = self.address_status.entry(address.clone()).or_default();
        *address_status = NatStatus::Private;

        self.confirmed_addresses.remove(address);
        self.update_state();
    }

    /// Determine the general NAT state of the local peer
    fn update_state(&mut self) {
        let old_nat_status = self.status;

        if !self.confirmed_addresses.is_empty() {
            self.status = NatStatus::Public;
        } else if self
            .address_status
            .iter()
            .all(|(_, status)| *status == NatStatus::Private)
        {
            self.status = NatStatus::Private;
        } else {
            self.status = NatStatus::Unknown;
        }

        if old_nat_status == self.status {
            return;
        }

        if self.status == NatStatus::Private {
            log::warn!("Couldn't detect a public reachable address. Validator network operations won't be possible");
            log::warn!("You may need to find a relay to enable validator network operations");
        } else if self.status == NatStatus::Public {
            log::info!(
                ?old_nat_status,
                new_nat_status = ?self.status,
                "NAT status changed and detected public reachable address. Validator network operations will be possible"
            );
        }
    }
}
