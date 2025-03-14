use std::cmp;

use nimiq_keys::Address;

/// Enum that represents the overall health of a validator
/// - Green means the Validator is working as expected.
/// - Yellow means there has been more than 'VALIDATOR_YELLOW_HEALTH_INACTIVATIONS' consecutive inactivations
///   in the current epoch.
/// - Red means there has been more than 'VALIDATOR_RED_HEALTH_INACTIVATIONS' consecutive inactivations
///   in the current epoch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ValidatorHealthState {
    Green,
    Yellow,
    Red,
}

/// Struct that represents the overall Validator Health
pub struct ValidatorHealth {
    /// The current validator health
    health: ValidatorHealthState,
    /// Validator address
    address: Address,
    /// Only used for testing purposes controls whether blocks are published by the validator
    publish: bool,
    /// Number of consecutive inactivations that have occurred in the current epoch
    inactivations: u32,
    /// Next block number indicating when the re-activate transaction should be sent
    reactivate_block_number: u32,
    /// Flag that indicates if we are currently inactive
    inactive: bool,
}

impl ValidatorHealth {
    /// The number of consecutive inactivations from which a validator is considered with yellow health
    const VALIDATOR_YELLOW_HEALTH_INACTIVATIONS: u32 = 2;
    /// The number of consecutive inactivations from which a validator is considered with red health
    const VALIDATOR_RED_HEALTH_INACTIVATIONS: u32 = 4;
    // The maximum number of blocks the reactivate transaction can be delayed
    const MAX_REACTIVATE_DELAY: u32 = 10_000;

    /// Creates a new instance of the validator health structure
    pub fn new(validator_address: &Address) -> ValidatorHealth {
        ValidatorHealth {
            health: ValidatorHealthState::Green,
            publish: true,
            inactivations: 0,
            reactivate_block_number: 0,
            inactive: false,
            address: validator_address.clone(),
        }
    }

    /// Computes the associated delay based on the current number of inactivations
    fn get_reactivate_delay(&self) -> u32 {
        cmp::min(self.inactivations.pow(2), Self::MAX_REACTIVATE_DELAY)
    }

    /// Returns the current health state of the validator
    pub fn health(&self) -> ValidatorHealthState {
        self.health
    }

    /// This flag should only be set in testing environments.
    pub fn _set_publish_flag(&mut self, publish: bool) {
        self.publish = publish
    }

    /// Decides if blocks should be published by the validator (only used for testing)
    pub fn publish_block(&self) -> bool {
        self.publish
    }

    /// Recomputes the current validator health state
    pub fn refresh_validator_health_status(&mut self) {
        log::trace!(
            address=%self.address,
            self.inactivations,
            "Current validator Inactivations counter"
        );

        match self.health {
            ValidatorHealthState::Green => {}
            ValidatorHealthState::Yellow => {
                if self.inactivations < Self::VALIDATOR_YELLOW_HEALTH_INACTIVATIONS {
                    log::info!(self.inactivations, "Changed validator health to green");
                    self.health = ValidatorHealthState::Green;
                }
            }
            ValidatorHealthState::Red => {
                if self.inactivations < Self::VALIDATOR_RED_HEALTH_INACTIVATIONS {
                    log::info!(self.inactivations, "Changed validator health to yellow");
                    self.health = ValidatorHealthState::Yellow;
                }
            }
        }
    }

    /// Decreases the number of consecutive deactivations
    pub fn block_produced(&mut self) {
        self.inactivations = self.inactivations.saturating_sub(1);
    }

    /// Increases the number of consecutive deactivations
    pub fn inactivate(&mut self, block_number: u32) {
        if self.inactive {
            // If we are currently inactive, then we dont have anything to do.
            return;
        }

        self.inactivations = self.inactivations.saturating_add(1);
        self.reactivate_block_number = block_number + self.get_reactivate_delay();
        self.inactive = true;

        match self.health {
            ValidatorHealthState::Green => {
                if self.inactivations >= Self::VALIDATOR_YELLOW_HEALTH_INACTIVATIONS {
                    log::warn!(self.inactivations, "Changed validator health to yellow");
                    self.health = ValidatorHealthState::Yellow;
                }
            }
            ValidatorHealthState::Yellow => {
                if self.inactivations >= Self::VALIDATOR_RED_HEALTH_INACTIVATIONS {
                    log::warn!(self.inactivations, "Changed validator health to red");
                    self.health = ValidatorHealthState::Red;
                }
            }
            ValidatorHealthState::Red => {
                log::warn!("Validator health is still red")
            }
        }

        log::debug!(
            address=%self.address,
            self.inactivations,
            self.reactivate_block_number,
            block_number,
            "New inactivation, current status",
        );
    }

    /// Resets the internal counters and validator health state for a new epoch
    pub fn reset_epoch(&mut self) {
        // Reset the inactivations counter
        self.inactivations = 0;
        // Reset the validator health every epoch
        self.health = ValidatorHealthState::Green;
    }

    /// Resets the pending reactivation flag
    pub fn reactivate(&mut self) {
        self.inactive = false;
    }

    /// Returns the block number for the next re-activate transaction
    pub fn get_reactivate_block_number(&self) -> u32 {
        self.reactivate_block_number
    }
}
