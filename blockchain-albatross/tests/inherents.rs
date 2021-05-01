use beserial::Serialize;
use nimiq_account::{Inherent, InherentType};
use nimiq_block_albatross::MacroHeader;
use nimiq_blockchain_albatross::Blockchain;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_database::WriteTransaction;
use nimiq_genesis::NetworkInfo;
use nimiq_hash::{Blake2bHasher, Hasher};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_primitives::slots::SlashedSlot;
use nimiq_vrf::VrfSeed;
use std::sync::Arc;

#[test]
fn it_can_create_batch_finalization_inherents() {
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Arc::new(Blockchain::new(env, NetworkId::UnitAlbatross).unwrap());

    let validator_registry_addr = NetworkInfo::from_network_id(blockchain.network_id)
        .validator_registry_address()
        .expect("No ValidatorRegistry")
        .clone();

    let hash = Blake2bHasher::default().digest(&[]);
    let macro_header = MacroHeader {
        version: 1,
        block_number: 42,
        view_number: 0,
        timestamp: blockchain.state().election_head.header.timestamp + 1,
        parent_hash: hash.clone(),
        parent_election_hash: hash.clone(),
        seed: VrfSeed::default(),
        extra_data: vec![],
        state_root: hash.clone(),
        body_root: hash.clone(),
        history_root: hash,
    };

    // Simple case. Expect 1x FinalizeBatch, 1x Reward to validator
    let inherents = blockchain.finalize_previous_batch(&blockchain.state(), &macro_header);
    assert_eq!(inherents.len(), 2);

    let validator = blockchain
        .get_staking_contract()
        .active_validators_sorted
        .iter()
        .next()
        .unwrap()
        .clone();

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    for inherent in &inherents {
        match inherent.ty {
            InherentType::Reward => {
                assert_eq!(inherent.value, Coin::from_u64_unchecked(8_74999));
                assert_eq!(inherent.target, validator.reward_address);
                got_reward = true;
            }
            InherentType::FinalizeBatch => {
                assert_eq!(inherent.value, Coin::ZERO);
                assert_eq!(inherent.target, validator_registry_addr.clone());
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch);

    // Slash one slot. Expect 1x FinalizeBatch, 1x Reward to validator, 1x Reward burn
    let slot = SlashedSlot {
        slot: 0,
        validator_id: validator.id.clone(),
        event_block: 0,
    };
    let slash_inherent = Inherent {
        ty: InherentType::Slash,
        target: validator_registry_addr.clone(),
        value: Coin::ZERO,
        data: slot.serialize_to_vec(),
    };
    let mut txn = WriteTransaction::new(&blockchain.env);
    // adds slot 0 to previous_lost_rewards -> slot won't get reward on next finalize_previous_batch
    assert!(blockchain
        .state()
        .accounts
        .commit(
            &mut txn,
            &[],
            &[slash_inherent],
            policy::BATCH_LENGTH + 1,
            0
        )
        .is_ok());
    txn.commit();

    let inherents = blockchain.finalize_previous_batch(&blockchain.state(), &macro_header);
    assert_eq!(inherents.len(), 3);
    let one_slot_reward = 8_74999 / policy::SLOTS as u64;
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
                        Coin::from_u64_unchecked(8_74999 - one_slot_reward as u64)
                    );
                    assert_eq!(inherent.target, validator.reward_address);
                    got_reward = true;
                }
            }
            InherentType::FinalizeBatch => {
                assert_eq!(inherent.target, validator_registry_addr.clone());
                assert_eq!(inherent.value, Coin::ZERO);
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_slash && got_finalize_batch);
}
