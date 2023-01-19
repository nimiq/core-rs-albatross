use beserial::Serialize;
use nimiq_account::{Inherent, InherentType};
use nimiq_block::{MacroBlock, MacroBody, MacroHeader};
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::{Blake2bHasher, Hasher};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy::Policy;
use nimiq_primitives::slots::SlashedSlot;
use nimiq_test_log::test;
use nimiq_utils::time::OffsetTime;
use nimiq_vrf::VrfSeed;
use std::sync::Arc;

#[test]
fn it_can_create_batch_finalization_inherents() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    );

    let staking_contract_address = blockchain.staking_contract_address();

    let hash = Blake2bHasher::default().digest(&[]);
    let macro_header = MacroHeader {
        version: 1,
        block_number: 42,
        round: 0,
        timestamp: blockchain.state().election_head.header.timestamp + 1,
        parent_hash: hash.clone(),
        parent_election_hash: hash.clone(),
        seed: VrfSeed::default(),
        extra_data: vec![],
        state_root: hash.clone(),
        body_root: hash.clone(),
        history_root: hash,
    };

    let staking_contract = blockchain.get_staking_contract();
    let active_validators = staking_contract.active_validators.clone();

    let body = MacroBody {
        validators: None,
        pk_tree_root: None,
        lost_reward_set: staking_contract.previous_lost_rewards(),
        disabled_set: staking_contract.previous_disabled_slots(),
    };

    let macro_block = MacroBlock {
        header: macro_header.clone(),
        body: Some(body),
        justification: None,
    };

    // Simple case. Expect 1x FinalizeBatch, 1x Reward to validator
    let inherents = blockchain.finalize_previous_batch(blockchain.state(), &macro_block);
    assert_eq!(inherents.len(), 2);

    let (validator_address, _) = active_validators.iter().next().unwrap();

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    for inherent in &inherents {
        match inherent.ty {
            InherentType::Reward => {
                assert_eq!(inherent.value, Coin::from_u64_unchecked(875));
                got_reward = true;
            }
            InherentType::FinalizeBatch => {
                assert_eq!(inherent.value, Coin::ZERO);
                assert_eq!(inherent.target, staking_contract_address.clone());
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch);

    // Slash one slot. Expect 1x FinalizeBatch, 1x Reward to validator, 1x Reward burn
    let slot = SlashedSlot {
        slot: 0,
        validator_address: validator_address.clone(),
        event_block: 0,
    };
    let slash_inherent = Inherent {
        ty: InherentType::Slash,
        target: staking_contract_address.clone(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };

    let mut txn = blockchain.write_transaction();
    // adds slot 0 to previous_lost_rewards -> slot won't get reward on next finalize_previous_batch
    assert!(blockchain
        .state()
        .accounts
        .commit(
            &mut txn,
            &[],
            &[slash_inherent],
            Policy::blocks_per_batch() + 1,
            0,
        )
        .is_ok());
    txn.commit();

    let staking_contract = blockchain.get_staking_contract();
    let body = MacroBody {
        validators: None,
        pk_tree_root: None,
        lost_reward_set: staking_contract.previous_lost_rewards(),
        disabled_set: staking_contract.previous_disabled_slots(),
    };
    let macro_block = MacroBlock {
        header: macro_header,
        body: Some(body),
        justification: None,
    };

    let inherents = blockchain.finalize_previous_batch(blockchain.state(), &macro_block);
    assert_eq!(inherents.len(), 3);
    let one_slot_reward = 875 / Policy::SLOTS as u64;
    let mut got_reward = false;
    let mut got_slash = false;
    let mut got_finalize_batch = false;

    for inherent in &inherents {
        match inherent.ty {
            InherentType::Reward => {
                if inherent.target == Address::burn_address() {
                    assert_eq!(inherent.value, Coin::from_u64_unchecked(one_slot_reward));
                    got_slash = true;
                } else {
                    assert_eq!(
                        inherent.value,
                        Coin::from_u64_unchecked(875 - one_slot_reward as u64)
                    );
                    got_reward = true;
                }
            }
            InherentType::FinalizeBatch => {
                assert_eq!(inherent.target, staking_contract_address.clone());
                assert_eq!(inherent.value, Coin::ZERO);
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_slash && got_finalize_batch);
}

#[test]
fn it_can_penalize_delayed_batch() {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    );

    let staking_contract_address = blockchain.staking_contract_address();

    //Delay in ms, so this means a 30s delay. For a 1m target batch time, this represents half of it
    let delay = 30000;

    let previous_timestamp = blockchain.state().election_head.header.timestamp;

    // We introduce a delay on purpose
    let next_timestamp = previous_timestamp
        + Policy::BLOCK_SEPARATION_TIME * (Policy::blocks_per_batch() as u64)
        + delay;

    let (genesis_supply, genesis_timestamp) = blockchain.get_genesis_parameters();

    // Total reward for the previous batch
    let prev_supply = Policy::supply_at(
        u64::from(genesis_supply),
        genesis_timestamp,
        genesis_timestamp,
    );

    let current_supply =
        Policy::supply_at(u64::from(genesis_supply), genesis_timestamp, next_timestamp);

    let max_reward = current_supply - prev_supply;

    let penalty = Policy::batch_delay_penalty(delay);

    log::info!(
        " The max available reward is {}, but due to a delay of {}ms there is a penalty of {}",
        max_reward,
        delay,
        penalty
    );

    let hash = Blake2bHasher::default().digest(&[]);
    let macro_header = MacroHeader {
        version: 1,
        block_number: 42,
        round: 0,
        timestamp: next_timestamp,
        parent_hash: hash.clone(),
        parent_election_hash: hash.clone(),
        seed: VrfSeed::default(),
        extra_data: vec![],
        state_root: hash.clone(),
        body_root: hash.clone(),
        history_root: hash,
    };

    let staking_contract = blockchain.get_staking_contract();

    let body = MacroBody {
        validators: None,
        pk_tree_root: None,
        lost_reward_set: staking_contract.previous_lost_rewards(),
        disabled_set: staking_contract.previous_disabled_slots(),
    };

    let macro_block = MacroBlock {
        header: macro_header,
        body: Some(body),
        justification: None,
    };

    // Simple case. Expect 1x FinalizeBatch, 1x Reward to validator
    let inherents = blockchain.finalize_previous_batch(blockchain.state(), &macro_block);
    assert_eq!(inherents.len(), 2);

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    for inherent in &inherents {
        match inherent.ty {
            InherentType::Reward => {
                assert_eq!(
                    inherent.value,
                    Coin::from_u64_unchecked((max_reward as f64 * penalty) as u64)
                );
                got_reward = true;
            }
            InherentType::FinalizeBatch => {
                assert_eq!(inherent.value, Coin::ZERO);
                assert_eq!(inherent.target, staking_contract_address.clone());
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch);
}
