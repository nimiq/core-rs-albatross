use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    vec,
};

use nimiq_account::punished_slots::PunishedSlots;
use nimiq_collections::BitSet;
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    policy::Policy,
    slots_allocation::{PenalizedSlot, SlashedValidator},
};
use nimiq_test_log::test;

#[derive(Debug)]
struct SlashConfig {
    event_block1: u32,
    slots_range1: Range<u16>,
    event_block2: u32,
    slots_range2: Range<u16>,
    reporting_block: u32,
    slots_range_next_epoch: Range<u16>,
    expected_current1: Range<u16>,
    expected_previous1: Range<u16>,
    expected_current2: Range<u16>,
    expected_previous2: Range<u16>,
}

impl Default for SlashConfig {
    fn default() -> Self {
        Self {
            event_block1: 1 + Policy::genesis_block_number(),
            slots_range1: 0..5,
            event_block2: 2 + Policy::genesis_block_number(),
            slots_range2: 0..5,
            reporting_block: 3 + Policy::genesis_block_number(),
            slots_range_next_epoch: 1..6,
            expected_current1: 0..0,
            expected_previous1: 0..0,
            expected_current2: 0..0,
            expected_previous2: 0..0,
        }
    }
}

fn penalize_slash_and_revert_twice(config: SlashConfig) {
    // Only allow valid block combinations.
    assert!(
        config.event_block1 < config.reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        config.event_block2 < config.reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        Policy::batch_at(config.reporting_block) <= Policy::batch_at(config.event_block1) + 1,
        "Can only report event up until end of next batch"
    );
    assert!(
        Policy::batch_at(config.reporting_block) <= Policy::batch_at(config.event_block2) + 1,
        "Can only report event up until end of next batch"
    );

    // Start with an empty set.
    let mut punished_slots = PunishedSlots::default();

    // Always penalize the same slot for different events.
    let penalize_slot1 = PenalizedSlot {
        slot: config.slots_range1.start,
        validator_address: Address([1u8; 20]),
        event_block: config.event_block1,
    };

    // Always slash the same validator for different events.
    let slashed_validator2 = SlashedValidator {
        slots: config.slots_range2,
        validator_address: Address([1u8; 20]),
        event_block: config.event_block2,
    };

    // After a slash, we expect all slots of the validator to be in the set.
    let mut expected_current_batch1 = BitSet::default();
    for slot in config.expected_current1 {
        expected_current_batch1.insert(slot as usize);
    }
    let mut expected_previous_batch1 = BitSet::default();
    for slot in config.expected_previous1 {
        expected_previous_batch1.insert(slot as usize);
    }
    let mut expected_current_batch2 = BitSet::default();
    for slot in config.expected_current2 {
        expected_current_batch2.insert(slot as usize);
    }
    let mut expected_previous_batch2 = BitSet::default();
    for slot in config.expected_previous2 {
        expected_previous_batch2.insert(slot as usize);
    }

    // Store initial sets.
    let current_batch_initial = punished_slots.current_batch_punished_slots();
    let previous_batch_initial = punished_slots.previous_batch_punished_slots.clone();

    // ------
    // Penalty.
    // ------
    let (old_prev1, old_current1) =
        punished_slots.register_penalty(&penalize_slot1, config.reporting_block);

    let current_batch_intermediate = punished_slots.current_batch_punished_slots();
    let previous_batch_intermediate = punished_slots.previous_batch_punished_slots.clone();

    // Test that penalty is properly registered in batch of event + current batch.
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        expected_previous_batch1
    );
    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        expected_current_batch1
    );

    // --------
    // Slash.
    // --------
    let (old_prev2, old_current2) = punished_slots.register_slash(
        &slashed_validator2,
        config.reporting_block,
        Some(config.slots_range_next_epoch),
    );

    // Test that slash is properly registered in batch of event + current batch.
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        expected_previous_batch2
    );
    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        expected_current_batch2
    );

    // ---------------
    // Revert slash.
    // ---------------
    punished_slots.revert_register_slash(&slashed_validator2, old_prev2, old_current2);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_intermediate
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_intermediate
    );

    // -------------
    // Revert penalty.
    // -------------
    punished_slots.revert_register_penalty(&penalize_slot1, old_prev1, old_current1);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_initial
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_initial
    );
}

#[test]
fn can_penalize_slash_and_revert_twice() {
    let combinations = vec![
        // Test cases within the same epoch:
        // 1. both events in same batch
        SlashConfig {
            slots_range1: 2..3,
            expected_current1: 2..3,
            expected_current2: 0..5,
            ..Default::default()
        },
        // 2. both events in previous batch
        SlashConfig {
            slots_range1: 2..3,
            reporting_block: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            expected_current1: 2..3,
            expected_previous1: 2..3,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 3. 1st event in current, 2nd in previous
        SlashConfig {
            slots_range1: 2..3,
            event_block1: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_batch() + 2 + Policy::genesis_block_number(),
            expected_current1: 2..3,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 4. 1st event in previous, 2nd in current
        SlashConfig {
            slots_range1: 2..3,
            event_block2: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_batch() + 2 + Policy::genesis_block_number(),
            expected_current1: 2..3,
            expected_previous1: 2..3,
            expected_current2: 0..5,
            expected_previous2: 2..3,
            ..Default::default()
        },
        // Test cases at epoch boundary:
        // 1. both events in previous batch
        SlashConfig {
            slots_range1: 2..3,
            event_block1: Policy::blocks_per_epoch() - 2 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            expected_previous1: 2..3,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 2. 1st event in current, 2nd in previous
        SlashConfig {
            slots_range1: 2..3,
            event_block1: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 2 + Policy::genesis_block_number(),
            expected_current1: 2..3,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 3. 1st event in previous, 2nd in current
        SlashConfig {
            slots_range1: 2..3,
            slots_range2: 1..6,
            event_block1: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 2 + Policy::genesis_block_number(),
            expected_previous1: 2..3,
            expected_current2: 1..6,
            expected_previous2: 2..3,
            ..Default::default()
        },
    ];

    for config in combinations {
        log::info!(?config, "slash and revert twice");
        penalize_slash_and_revert_twice(config);
    }
}

fn slash_penalize_and_revert_twice(config: SlashConfig) {
    // Only allow valid block combinations.
    assert!(
        config.event_block1 < config.reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        config.event_block2 < config.reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        Policy::batch_at(config.reporting_block) <= Policy::batch_at(config.event_block1) + 1,
        "Can only report event up until end of next batch"
    );
    assert!(
        Policy::batch_at(config.reporting_block) <= Policy::batch_at(config.event_block2) + 1,
        "Can only report event up until end of next batch"
    );

    // Start with an empty set.
    let mut punished_slots = PunishedSlots::default();

    // Always slash the same validator for different events.
    let slashed_validator1 = SlashedValidator {
        slots: config.slots_range1,
        validator_address: Address([1u8; 20]),
        event_block: config.event_block1,
    };

    // Always penalize the same slot for different events.
    let penalized_slot2 = PenalizedSlot {
        slot: config.slots_range2.start,
        validator_address: Address([1u8; 20]),
        event_block: config.event_block2,
    };

    // After a slash, we expect all slots of the validator to be in the set.
    let mut expected_current_batch1 = BitSet::default();
    for slot in config.expected_current1 {
        expected_current_batch1.insert(slot as usize);
    }
    let mut expected_previous_batch1 = BitSet::default();
    for slot in config.expected_previous1 {
        expected_previous_batch1.insert(slot as usize);
    }
    let mut expected_current_batch2 = BitSet::default();
    for slot in config.expected_current2 {
        expected_current_batch2.insert(slot as usize);
    }
    let mut expected_previous_batch2 = BitSet::default();
    for slot in config.expected_previous2 {
        expected_previous_batch2.insert(slot as usize);
    }

    // Store initial sets.
    let current_batch_initial = punished_slots.current_batch_punished_slots();
    let previous_batch_initial = punished_slots.previous_batch_punished_slots.clone();

    // ------
    // Slash.
    // ------
    let (old_prev1, old_current1) = punished_slots.register_slash(
        &slashed_validator1,
        config.reporting_block,
        Some(config.slots_range_next_epoch),
    );

    let current_batch_intermediate = punished_slots.current_batch_punished_slots();
    let previous_batch_intermediate = punished_slots.previous_batch_punished_slots.clone();

    // Test that slash is properly registered in batch of event + current batch.
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        expected_previous_batch1
    );
    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        expected_current_batch1
    );

    // --------
    // Penalty.
    // --------
    let (old_prev2, old_current2) =
        punished_slots.register_penalty(&penalized_slot2, config.reporting_block);

    // Test that penalty is properly registered in batch of event + current batch.
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        expected_previous_batch2
    );
    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        expected_current_batch2
    );

    // ---------------
    // Revert penalty.
    // ---------------
    punished_slots.revert_register_penalty(&penalized_slot2, old_prev2, old_current2);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_intermediate
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_intermediate
    );

    // -------------
    // Revert slash.
    // -------------
    punished_slots.revert_register_slash(&slashed_validator1, old_prev1, old_current1);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_initial
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_initial
    );
}

#[test]
fn can_slash_penalize_and_revert_twice() {
    let combinations = vec![
        // Test cases within the same epoch:
        // 1. both events in same batch
        SlashConfig {
            slots_range2: 2..3,
            expected_current1: 0..5,
            expected_current2: 0..5,
            ..Default::default()
        },
        // 2. both events in previous batch
        SlashConfig {
            slots_range2: 2..3,
            reporting_block: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            expected_current1: 0..5,
            expected_previous1: 0..5,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 3. 1st event in current, 2nd in previous
        SlashConfig {
            slots_range2: 2..3,
            event_block1: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_batch() + 2 + Policy::genesis_block_number(),
            expected_current1: 0..5,
            expected_current2: 0..5,
            expected_previous2: 2..3,
            ..Default::default()
        },
        // 4. 1st event in previous, 2nd in current
        SlashConfig {
            slots_range2: 2..3,
            event_block2: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_batch() + 2 + Policy::genesis_block_number(),
            expected_current1: 0..5,
            expected_previous1: 0..5,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // Test cases at epoch boundary:
        // 1. both events in previous batch
        SlashConfig {
            slots_range2: 2..3,
            event_block1: Policy::blocks_per_epoch() - 2 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_previous1: 0..5,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 2. 1st event in current, 2nd in previous
        SlashConfig {
            slots_range1: 1..6,
            slots_range2: 2..3,
            event_block1: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 2 + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_current2: 1..6,
            expected_previous2: 2..3,
            ..Default::default()
        },
        // 3. 1st event in previous, 2nd in current
        SlashConfig {
            slots_range2: 2..3,
            event_block1: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 2 + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_previous1: 0..5,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
    ];

    for config in combinations {
        log::info!(?config, "slash and revert twice");
        slash_penalize_and_revert_twice(config);
    }
}

fn slash_and_revert_twice(config: SlashConfig) {
    // Only allow valid block combinations.
    assert!(
        config.event_block1 < config.reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        config.event_block2 < config.reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        config.reporting_block <= Policy::last_block_of_reporting_window(config.event_block1),
        "Can only report event up until reporting window"
    );
    assert!(
        config.reporting_block <= Policy::last_block_of_reporting_window(config.event_block2),
        "Can only report event up until reporting window"
    );

    // Start with an empty set.
    let mut punished_slots = PunishedSlots::default();

    // Always slash the same validator for different events.
    let slashed_validator1 = SlashedValidator {
        slots: config.slots_range1,
        validator_address: Address([1u8; 20]),
        event_block: config.event_block1,
    };

    let slashed_validator2 = SlashedValidator {
        slots: config.slots_range2,
        validator_address: Address([1u8; 20]),
        event_block: config.event_block2,
    };
    let validator_slots_next_epoch = Some(config.slots_range_next_epoch.clone());

    // After a slash, we expect all slots of the validator to be in the set.
    let mut expected_current_batch1 = BitSet::default();
    for slot in config.expected_current1 {
        expected_current_batch1.insert(slot as usize);
    }
    let mut expected_previous_batch1 = BitSet::default();
    for slot in config.expected_previous1 {
        expected_previous_batch1.insert(slot as usize);
    }
    let mut expected_current_batch2 = BitSet::default();
    for slot in config.expected_current2 {
        expected_current_batch2.insert(slot as usize);
    }
    let mut expected_previous_batch2 = BitSet::default();
    for slot in config.expected_previous2 {
        expected_previous_batch2.insert(slot as usize);
    }

    // Store initial sets.
    let current_batch_initial = punished_slots.current_batch_punished_slots();
    let previous_batch_initial = punished_slots.previous_batch_punished_slots.clone();

    // ------------
    // First slash.
    // ------------
    let (old_prev1, old_current1) = punished_slots.register_slash(
        &slashed_validator1,
        config.reporting_block,
        validator_slots_next_epoch.clone(),
    );

    let current_batch_intermediate = punished_slots.current_batch_punished_slots();
    let previous_batch_intermediate = punished_slots.previous_batch_punished_slots.clone();

    // Test that slash is properly registered in batch of event + current batch.
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        expected_previous_batch1
    );
    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        expected_current_batch1
    );

    // -------------
    // Second slash.
    // -------------
    let (old_prev2, old_current2) = punished_slots.register_slash(
        &slashed_validator2,
        config.reporting_block,
        validator_slots_next_epoch,
    );

    // Test that slash is properly registered in batch of event + current batch.
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        expected_previous_batch2
    );
    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        expected_current_batch2
    );

    // --------------------
    // Revert second slash.
    // --------------------
    punished_slots.revert_register_slash(&slashed_validator2, old_prev2, old_current2);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_intermediate
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_intermediate
    );

    // -------------------
    // Revert first slash.
    // -------------------
    punished_slots.revert_register_slash(&slashed_validator1, old_prev1, old_current1);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_initial
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_initial
    );
}

#[test]
fn can_slash_and_revert_twice() {
    let combinations = vec![
        // Test cases within the same epoch:
        // 1. both events in same batch
        SlashConfig {
            expected_current1: 0..5,
            expected_current2: 0..5,
            ..Default::default()
        },
        // 2. both events in previous batch
        SlashConfig {
            reporting_block: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            expected_current1: 0..5,
            expected_previous1: 0..5,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 3. 1st event in current, 2nd in previous
        SlashConfig {
            event_block1: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_batch() + 2 + Policy::genesis_block_number(),
            expected_current1: 0..5,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 4. 1st event in previous, 2nd in current
        SlashConfig {
            event_block2: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_batch() + 2 + Policy::genesis_block_number(),
            expected_current1: 0..5,
            expected_previous1: 0..5,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // Test cases at epoch boundary:
        // 1. both events in previous batch
        SlashConfig {
            event_block1: Policy::blocks_per_epoch() - 2 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_previous1: 0..5,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 2. 1st event in current, 2nd in previous
        SlashConfig {
            event_block1: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            slots_range1: 1..6,
            event_block2: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 2 + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 3. 1st event in previous, 2nd in current
        SlashConfig {
            event_block1: Policy::blocks_per_epoch() - 1 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            slots_range2: 1..6,
            reporting_block: Policy::blocks_per_epoch() + 2 + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_previous1: 0..5,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 4. 1st event in same epoch as reporting block and one batch earlier than `previous_batch`,
        // `current_batch` in the same epoch one batch after
        SlashConfig {
            event_block1: 1 + Policy::genesis_block_number(),
            event_block2: 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_batch() * 2 + 1 + Policy::genesis_block_number(),
            expected_current1: 0..5,
            expected_previous1: 0..5,
            expected_current2: 0..5,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 5. 1st event in previous epoch (same epoch as `previous_batch`) and earlier than `previous_batch`,
        // `current_batch` in a new epoch
        SlashConfig {
            event_block1: 1 + Policy::genesis_block_number(),
            event_block2: 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch() + 1 + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_previous1: 0..5,
            expected_current2: 1..6,
            expected_previous2: 0..5,
            ..Default::default()
        },
        // 6. 1st event in the epoch before `previous_batch`,
        // `previous_batch` and `current_batch` are both in a new epoch
        SlashConfig {
            event_block1: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            event_block2: Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
            reporting_block: Policy::blocks_per_epoch()
                + Policy::blocks_per_batch()
                + 1
                + Policy::genesis_block_number(),
            expected_current1: 1..6,
            expected_previous1: 1..6,
            expected_current2: 1..6,
            expected_previous2: 1..6,
            ..Default::default()
        },
    ];

    for config in combinations {
        log::info!(?config, "slash and revert twice");
        slash_and_revert_twice(config);
    }
}

fn penalize_and_revert_twice(event_block1: u32, event_block2: u32, reporting_block: u32) {
    // Only allow valid block combinations.
    assert!(
        event_block1 < reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        event_block2 < reporting_block,
        "Can only report event after it happened"
    );
    assert!(
        Policy::batch_at(reporting_block) <= Policy::batch_at(event_block1) + 1,
        "Can only report event up until end of next batch"
    );
    assert!(
        Policy::batch_at(reporting_block) <= Policy::batch_at(event_block2) + 1,
        "Can only report event up until end of next batch"
    );

    // Start with an empty set.
    let mut punished_slots = PunishedSlots::default();

    // Always penalize the same slot for different events.
    let penalized_slot1 = PenalizedSlot {
        slot: 1,
        validator_address: Address([1u8; 20]),
        event_block: event_block1,
    };

    let penalized_slot2 = PenalizedSlot {
        slot: 1,
        validator_address: Address([1u8; 20]),
        event_block: event_block2,
    };

    // After a penalty, we expect the corresponding slot of the validator to be in the set.
    let mut expected_set_filled = BitSet::default();
    expected_set_filled.insert(penalized_slot1.slot as usize);

    // Store initial sets.
    let current_batch_initial = punished_slots.current_batch_punished_slots();
    let previous_batch_initial = punished_slots.previous_batch_punished_slots.clone();

    // --------------
    // First penalty.
    // --------------
    let (old_prev1, old_current1) =
        punished_slots.register_penalty(&penalized_slot1, reporting_block);

    let current_batch_intermediate = punished_slots.current_batch_punished_slots();
    let previous_batch_intermediate = punished_slots.previous_batch_punished_slots.clone();

    // Test that penalty is properly registered in batch of event + current batch.
    if Policy::batch_at(event_block1) < Policy::batch_at(reporting_block) {
        assert_eq!(
            punished_slots.previous_batch_punished_slots,
            expected_set_filled
        );
    } else {
        assert_eq!(
            punished_slots.previous_batch_punished_slots,
            previous_batch_initial
        );
    }
    if Policy::epoch_at(event_block1) == Policy::epoch_at(reporting_block) {
        assert_eq!(
            punished_slots.current_batch_punished_slots(),
            expected_set_filled
        );
    } else {
        assert_eq!(
            punished_slots.current_batch_punished_slots(),
            current_batch_initial
        );
    }

    // ---------------
    // Second penalty.
    // ---------------
    let (old_prev2, old_current2) =
        punished_slots.register_penalty(&penalized_slot2, reporting_block);

    // Test that slash is properly registered in batch of event + current batch.
    if Policy::batch_at(event_block2) < Policy::batch_at(reporting_block) {
        assert_eq!(
            punished_slots.previous_batch_punished_slots,
            expected_set_filled
        );
    } else {
        assert_eq!(
            punished_slots.previous_batch_punished_slots,
            previous_batch_intermediate
        );
    }
    if Policy::epoch_at(event_block2) == Policy::epoch_at(reporting_block) {
        assert_eq!(
            punished_slots.current_batch_punished_slots(),
            expected_set_filled
        );
    } else {
        assert_eq!(
            punished_slots.current_batch_punished_slots(),
            current_batch_intermediate
        );
    }

    // ----------------------
    // Revert second penalty.
    // ----------------------
    punished_slots.revert_register_penalty(&penalized_slot2, old_prev2, old_current2);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_intermediate
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_intermediate
    );

    // ---------------------
    // Revert first penalty.
    // ---------------------
    punished_slots.revert_register_penalty(&penalized_slot1, old_prev1, old_current1);

    assert_eq!(
        punished_slots.current_batch_punished_slots(),
        current_batch_initial
    );
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        previous_batch_initial
    );
}

#[test]
fn can_penalize_and_revert_twice() {
    let combinations = vec![
        // Test cases within the same epoch:
        // 1. both events in same batch
        (1, 2, 3),
        // 2. both events in previous batch
        (1, 2, Policy::blocks_per_batch() + 1),
        // 3. 1st event in current, 2nd in previous
        (
            Policy::blocks_per_batch() + 1,
            2,
            Policy::blocks_per_batch() + 2,
        ),
        // 4. 1st event in previous, 2nd in current
        (
            2,
            Policy::blocks_per_batch() + 1,
            Policy::blocks_per_batch() + 2,
        ),
        // Test cases at epoch boundary:
        // 1. both events in previous batch
        (
            Policy::blocks_per_epoch() - 2,
            Policy::blocks_per_epoch() - 1,
            Policy::blocks_per_epoch() + 1,
        ),
        // 2. 1st event in current, 2nd in previous
        (
            Policy::blocks_per_epoch() + 1,
            Policy::blocks_per_epoch() - 1,
            Policy::blocks_per_epoch() + 2,
        ),
        // 3. 1st event in previous, 2nd in current
        (
            Policy::blocks_per_epoch() - 1,
            Policy::blocks_per_epoch() + 1,
            Policy::blocks_per_epoch() + 2,
        ),
    ];

    for (event_block1, event_block2, reporting_block) in combinations {
        log::info!(
            event_block1,
            event_block2,
            reporting_block,
            "slash and revert twice"
        );
        penalize_and_revert_twice(event_block1, event_block2, reporting_block);
    }
}

#[test]
fn can_finalize_batch() {
    let validator_address1 = Address([1u8; 20]);
    let validator_address2 = Address([2u8; 20]);
    let validator_address3 = Address([3u8; 20]);

    // Punish validator 1 slots {2, 4} in the current batch.
    let mut current_batch_punished_slots = BTreeMap::new();
    let mut validator_slots = BTreeSet::new();
    validator_slots.insert(2);
    validator_slots.insert(4);
    current_batch_punished_slots.insert(validator_address1.clone(), validator_slots);

    // Punish validator 2 slots {2, 4} in the current batch.
    let mut validator_slots = BTreeSet::new();
    validator_slots.insert(1);
    validator_slots.insert(3);
    current_batch_punished_slots.insert(validator_address2.clone(), validator_slots);

    // Punish validator 3 slots {2, 4} in the current batch.
    let mut validator_slots = BTreeSet::new();
    validator_slots.insert(5);
    validator_slots.insert(6);
    let validator3_slots = validator_slots.clone();
    current_batch_punished_slots.insert(validator_address3.clone(), validator_slots);

    // Punish slots {1, 2, 10} in the previous batch.
    let mut previous_batch_punished_slots = BitSet::new();
    previous_batch_punished_slots.insert(1);
    previous_batch_punished_slots.insert(2);
    previous_batch_punished_slots.insert(10);

    let mut punished_slots =
        PunishedSlots::new(current_batch_punished_slots, previous_batch_punished_slots);

    // Store the currently punished slots as they will be subject to change after the finalization.
    let old_current_punished_slots = punished_slots.current_batch_punished_slots();

    // The map of all active validators at batch finalization.
    let mut current_active_validators = BTreeMap::new();
    current_active_validators.insert(validator_address1.clone(), Coin::from_u64_unchecked(1));
    current_active_validators.insert(validator_address2.clone(), Coin::from_u64_unchecked(1));

    // Finalize the batch and check sets.
    punished_slots.finalize_batch(
        Policy::blocks_per_batch() + Policy::genesis_block_number(),
        &current_active_validators,
    );

    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        old_current_punished_slots
    );

    // Only validator 3 is remaining inactive upon batch finalization.
    // Thus it should be the only one remaining in the current punished set.
    assert_eq!(
        punished_slots
            .current_batch_punished_slots_map()
            .get(&validator_address3),
        Some(&validator3_slots)
    );
    assert_eq!(
        punished_slots
            .current_batch_punished_slots_map()
            .keys()
            .count(),
        1
    );
}

#[test]
fn can_finalize_epoch() {
    // Punish slots {2, 4} in the current batch/epoch and {1, 2} in the previous batch/epoch.
    let mut current_batch_punished_slots = BTreeMap::new();
    let mut validator_slots = BTreeSet::new();
    validator_slots.insert(2);
    validator_slots.insert(4);
    current_batch_punished_slots.insert(Address([1u8; 20]), validator_slots);

    let mut previous_batch_punished_slots = BitSet::new();
    previous_batch_punished_slots.insert(1);
    previous_batch_punished_slots.insert(2);

    let mut punished_slots =
        PunishedSlots::new(current_batch_punished_slots, previous_batch_punished_slots);

    // Store the currently punished slots as they will become the previous punished slots after the finalization.
    let current_punished_slots = punished_slots.current_batch_punished_slots();

    // Finalize the epoch and check sets.
    punished_slots.finalize_batch(Policy::blocks_per_epoch(), &BTreeMap::default());

    assert!(punished_slots.current_batch_punished_slots().is_empty());
    assert_eq!(
        punished_slots.previous_batch_punished_slots,
        current_punished_slots
    );
}
