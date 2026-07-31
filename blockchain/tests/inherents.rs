use std::sync::Arc;

use nimiq_account::{Account, BlockLogger, BlockState, StakingContractStoreWrite, TransactionLog};
use nimiq_block::{
    Block, DoubleProposalProof, DoubleVoteProof, ForkProof, MacroBlock, MacroBody, MacroHeader,
    SkipBlockInfo,
};
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::AggregateSignature;
use nimiq_database::{mdbx::MdbxDatabase, traits::WriteTransaction};
use nimiq_hash::{Blake2sHash, HashOutput};
use nimiq_keys::Address;
use nimiq_primitives::{
    coin::Coin,
    key_nibbles::KeyNibbles,
    networks::NetworkId,
    policy::Policy,
    slots_allocation::{JailedValidator, PenalizedSlot, Validator, Validators},
    TendermintIdentifier, TendermintProposal, TendermintStep, TendermintVote,
};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{signing_key, validator_address, voting_key},
};
use nimiq_transaction::inherent::Inherent;
use nimiq_utils::time::OffsetTime;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

const EXPECTED_REWARD: u64 = 166_810_895;
const ONE_SLOT_REWARD: u64 = EXPECTED_REWARD / Policy::SLOTS as u64;

struct RewardTestSetup {
    blockchain: Arc<Blockchain>,
    validator_address: Address,
}

struct MixedRewardValidators {
    payable_reward_address: Address,
    penalized_address: Address,
    burn_address_validator: Address,
    payable_slots: std::ops::Range<u16>,
    unpayable_slots: std::ops::Range<u16>,
    penalized_slots: std::ops::Range<u16>,
    burn_address_slots: std::ops::Range<u16>,
}

impl RewardTestSetup {
    fn new() -> Self {
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let blockchain = Arc::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        );

        let staking_contract = blockchain.get_staking_contract();
        let validator_address = staking_contract
            .active_validators
            .iter()
            .next()
            .unwrap()
            .0
            .clone();

        Self {
            blockchain,
            validator_address,
        }
    }

    fn set_validator_reward_address(&self, reward_address: Address) {
        let mut txn = self.blockchain.write_transaction();
        let mut db_txn = (&mut txn).into();
        let data_store = self
            .blockchain
            .state
            .accounts
            .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut data_store_write = data_store.write(&mut db_txn);
        let mut store = StakingContractStoreWrite::new(&mut data_store_write);
        let mut staking_contract = self.blockchain.get_staking_contract();

        staking_contract
            .update_validator(
                &mut store,
                &self.validator_address,
                None,
                None,
                Some(reward_address),
                None,
                &mut TransactionLog::empty(),
            )
            .unwrap();
        txn.commit();
    }

    fn configure_mixed_reward_validators(&mut self) -> MixedRewardValidators {
        let slot_validator = self
            .blockchain
            .state
            .current_slots
            .as_ref()
            .unwrap()
            .validators[0]
            .clone();
        let payable_reward_address = {
            let staking_contract = self.blockchain.get_staking_contract();
            let txn = self.blockchain.read_transaction();
            let data_store = self
                .blockchain
                .state
                .accounts
                .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
            staking_contract
                .get_validator(&data_store.read(&txn), &self.validator_address)
                .unwrap()
                .reward_address
        };
        let unpayable_address = Address::from([0xb; Address::SIZE]);
        let penalized_address = Address::from([0xc; Address::SIZE]);
        let burn_address_validator = Address::from([0xd; Address::SIZE]);

        let mut txn = self.blockchain.write_transaction();
        let mut db_txn = (&mut txn).into();
        let mut staking_contract = self.blockchain.get_staking_contract();
        {
            let data_store = self
                .blockchain
                .state
                .accounts
                .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
            let mut data_store_write = data_store.write(&mut db_txn);
            let mut store = StakingContractStoreWrite::new(&mut data_store_write);

            for (validator_address, reward_address) in [
                (&unpayable_address, Policy::STAKING_CONTRACT_ADDRESS.clone()),
                (&penalized_address, payable_reward_address.clone()),
                (&burn_address_validator, Address::burn_address()),
            ] {
                staking_contract
                    .create_validator(
                        &mut store,
                        validator_address,
                        slot_validator.signing_key,
                        slot_validator.voting_key.compressed().clone(),
                        reward_address,
                        None,
                        Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
                        None,
                        None,
                        false,
                        Policy::genesis_block_number(),
                        &mut TransactionLog::empty(),
                    )
                    .unwrap();
            }
        }
        self.blockchain
            .state
            .accounts
            .tree
            .put(
                &mut db_txn,
                &KeyNibbles::from(&Policy::STAKING_CONTRACT_ADDRESS),
                Account::Staking(staking_contract),
            )
            .unwrap();
        txn.commit();

        let slots_per_validator = Policy::SLOTS / 4;
        let payable_slots = 0..slots_per_validator;
        let unpayable_slots = payable_slots.end..payable_slots.end + slots_per_validator;
        let penalized_slots = unpayable_slots.end..unpayable_slots.end + slots_per_validator;
        let burn_address_slots = penalized_slots.end..Policy::SLOTS;
        Arc::get_mut(&mut self.blockchain)
            .unwrap()
            .state
            .current_slots = Some(Validators::new(vec![
            Validator::new(
                self.validator_address.clone(),
                slot_validator.voting_key.clone(),
                slot_validator.signing_key,
                payable_slots.clone(),
            ),
            Validator::new(
                unpayable_address.clone(),
                slot_validator.voting_key.clone(),
                slot_validator.signing_key,
                unpayable_slots.clone(),
            ),
            Validator::new(
                penalized_address.clone(),
                slot_validator.voting_key.clone(),
                slot_validator.signing_key,
                penalized_slots.clone(),
            ),
            Validator::new(
                burn_address_validator.clone(),
                slot_validator.voting_key,
                slot_validator.signing_key,
                burn_address_slots.clone(),
            ),
        ]));

        MixedRewardValidators {
            payable_reward_address,
            penalized_address,
            burn_address_validator,
            payable_slots,
            unpayable_slots,
            penalized_slots,
            burn_address_slots,
        }
    }

    fn penalize_slots(&self, slots: impl IntoIterator<Item = u16>) {
        self.penalize_validator_slots(&self.validator_address, slots);
    }

    fn penalize_validator_slots(
        &self,
        validator_address: &Address,
        slots: impl IntoIterator<Item = u16>,
    ) {
        for slot in slots {
            let penalize_inherent = Inherent::Penalize {
                slot: PenalizedSlot {
                    slot,
                    validator_address: validator_address.clone(),
                    offense_event_block: 1 + Policy::genesis_block_number(),
                },
            };

            let mut txn = self.blockchain.write_transaction();
            assert!(self
                .blockchain
                .state
                .accounts
                .commit(
                    &mut (&mut txn).into(),
                    &[],
                    &[penalize_inherent],
                    &BlockState::new(
                        Policy::blocks_per_batch() + 1 + Policy::genesis_block_number(),
                        1,
                        self.blockchain.protocol_version()
                    ),
                    &mut BlockLogger::empty()
                )
                .is_ok());
            txn.commit();
        }
    }

    fn create_macro_block(&self) -> MacroBlock {
        let block_number = Policy::macro_block_of(2).unwrap();
        let staking_contract = self.blockchain.get_staking_contract();
        let next_batch_initial_punished_set = staking_contract
            .punished_slots
            .next_batch_initial_punished_set(block_number, &staking_contract.active_validators);
        let macro_header = MacroHeader {
            network: NetworkId::UnitAlbatross,
            version: 1,
            block_number,
            round: 0,
            timestamp: self.blockchain.state.election_head.header.timestamp + 20000,
            next_batch_initial_punished_set,
            ..Default::default()
        };
        let reward_transactions =
            self.blockchain
                .create_reward_transactions(&macro_header, &staking_contract, None);

        MacroBlock {
            header: macro_header,
            body: Some(MacroBody {
                transactions: reward_transactions,
            }),
            justification: None,
        }
    }
}

#[test]
fn it_can_create_batch_finalization_inherents() {
    let setup = RewardTestSetup::new();
    let macro_block = setup.create_macro_block();

    // Simple case. Expect 1x FinalizeBatch, 1x Reward to validator
    let inherents = setup.blockchain.finalize_previous_batch(&macro_block);
    assert_eq!(inherents.len(), 2);

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    for inherent in &inherents {
        match inherent {
            Inherent::Reward { value, .. } => {
                assert_eq!(*value, Coin::from_u64_unchecked(EXPECTED_REWARD));
                got_reward = true;
            }
            Inherent::FinalizeBatch => {
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch && inherents.len() == 2);

    // Penalize one slot. Expect 1x FinalizeBatch, 1x Reward to validator, 1x Reward burn
    // adds slot 0 to previous_lost_rewards -> slot won't get reward on next finalize_previous_batch
    setup.penalize_slots([0]);
    let macro_block = setup.create_macro_block();

    let inherents = setup.blockchain.finalize_previous_batch(&macro_block);
    assert_eq!(inherents.len(), 3);
    let mut got_reward = false;
    let mut got_penalize = false;
    let mut got_finalize_batch = false;

    for inherent in &inherents {
        match inherent {
            Inherent::Reward {
                validator_address,
                target,
                value,
            } => {
                if *target == Address::burn_address() {
                    assert_eq!(*validator_address, Address::burn_address());
                    assert_eq!(*value, Coin::from_u64_unchecked(ONE_SLOT_REWARD));
                    got_penalize = true;
                } else {
                    assert_eq!(*validator_address, setup.validator_address);
                    assert_eq!(
                        *value,
                        Coin::from_u64_unchecked(EXPECTED_REWARD - ONE_SLOT_REWARD),
                    );
                    got_reward = true;
                }
            }
            Inherent::FinalizeBatch => {
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_penalize && got_finalize_batch);
}

#[test]
fn validator_deliberately_burning_rewards_works() {
    let setup = RewardTestSetup::new();
    // Set validator's reward address to the burn address.
    setup.set_validator_reward_address(Address::burn_address());
    let macro_block = setup.create_macro_block();

    // Simple case. Expect 1x FinalizeBatch, 1x Reward to validator
    let inherents = setup.blockchain.finalize_previous_batch(&macro_block);
    assert_eq!(inherents.len(), 2);

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    for inherent in &inherents {
        match inherent {
            Inherent::Reward { target, value, .. } => {
                assert_eq!(target, &Address::burn_address());
                assert_eq!(*value, Coin::from_u64_unchecked(EXPECTED_REWARD));
                got_reward = true;
            }
            Inherent::FinalizeBatch => {
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch && inherents.len() == 2);

    // Penalize one slot. Expect 1x FinalizeBatch, 1x Reward to validator, 1x Reward burn
    // adds slot 0 to previous_lost_rewards -> slot won't get reward on next finalize_previous_batch
    setup.penalize_slots([0]);
    let macro_block = setup.create_macro_block();

    let inherents = setup.blockchain.finalize_previous_batch(&macro_block);
    assert_eq!(inherents.len(), 3);
    assert_eq!(
        inherents,
        [
            Inherent::Reward {
                validator_address: setup.validator_address.clone(),
                target: Address::burn_address(),
                value: Coin::from_u64_unchecked(EXPECTED_REWARD - ONE_SLOT_REWARD),
            },
            Inherent::Reward {
                validator_address: Address::burn_address(),
                target: Address::burn_address(),
                value: Coin::from_u64_unchecked(ONE_SLOT_REWARD),
            },
            Inherent::FinalizeBatch
        ]
    );
}

#[test]
fn validator_reward_address_that_cannot_accept_inherents_burns_rewards() {
    let setup = RewardTestSetup::new();
    // Point the validator reward address at the staking contract, which cannot accept Reward
    // inherents, so the reward must be burned instead.
    setup.set_validator_reward_address(Policy::STAKING_CONTRACT_ADDRESS);
    let macro_block = setup.create_macro_block();
    let reward_transactions = &macro_block.body.as_ref().unwrap().transactions;

    assert_eq!(
        reward_transactions.as_slice(),
        &[nimiq_transaction::reward::RewardTransaction {
            validator_address: Address::burn_address(),
            recipient: Address::burn_address(),
            value: Coin::from_u64_unchecked(EXPECTED_REWARD),
        }]
    );

    assert_eq!(
        setup.blockchain.finalize_previous_batch(&macro_block),
        [
            Inherent::Reward {
                validator_address: Address::burn_address(),
                target: Address::burn_address(),
                value: Coin::from_u64_unchecked(EXPECTED_REWARD),
            },
            Inherent::FinalizeBatch
        ]
    );
}

#[test]
fn penalized_slots_and_unpayable_reward_address_burn_all_rewards() {
    let setup = RewardTestSetup::new();
    // Point the validator reward address at the staking contract, which cannot accept Reward
    // inherents, so even the non-penalized portion must be burned.
    setup.set_validator_reward_address(Policy::STAKING_CONTRACT_ADDRESS);
    setup.penalize_slots([0]);
    let macro_block = setup.create_macro_block();
    let reward_transactions = &macro_block.body.as_ref().unwrap().transactions;

    assert_eq!(
        reward_transactions.as_slice(),
        &[nimiq_transaction::reward::RewardTransaction {
            validator_address: Address::burn_address(),
            recipient: Address::burn_address(),
            value: Coin::from_u64_unchecked(EXPECTED_REWARD),
        }]
    );

    assert_eq!(
        setup.blockchain.finalize_previous_batch(&macro_block),
        [
            Inherent::Reward {
                validator_address: Address::burn_address(),
                target: Address::burn_address(),
                value: Coin::from_u64_unchecked(EXPECTED_REWARD),
            },
            Inherent::FinalizeBatch
        ]
    );
}

#[test]
fn multiple_validators_mix_all_reward_outcomes() {
    let mut setup = RewardTestSetup::new();
    // Validator A can receive rewards, validator B cannot, all of validator C's slots are
    // penalized, and validator D deliberately pays rewards to the burn address.
    let validators = setup.configure_mixed_reward_validators();
    setup.penalize_validator_slots(
        &validators.penalized_address,
        validators.penalized_slots.clone(),
    );

    let macro_block = setup.create_macro_block();
    let reward_transactions = &macro_block.body.as_ref().unwrap().transactions;
    let [payable_reward, burn_address_payout, aggregate_burn] = reward_transactions.as_slice()
    else {
        panic!("Expected two validator payouts and one aggregate burn transaction");
    };

    assert_eq!(payable_reward.validator_address, setup.validator_address);
    assert_eq!(payable_reward.recipient, validators.payable_reward_address);
    assert_eq!(
        burn_address_payout.validator_address,
        validators.burn_address_validator
    );
    assert_eq!(burn_address_payout.recipient, Address::burn_address());
    assert_eq!(aggregate_burn.validator_address, Address::burn_address());
    assert_eq!(aggregate_burn.recipient, Address::burn_address());

    let payable_base_reward = ONE_SLOT_REWARD * validators.payable_slots.len() as u64;
    let burn_address_base_reward = ONE_SLOT_REWARD * validators.burn_address_slots.len() as u64;
    let burned_slot_count =
        (validators.unpayable_slots.len() + validators.penalized_slots.len()) as u64;
    let remainder = EXPECTED_REWARD % Policy::SLOTS as u64;
    let burned_reward = ONE_SLOT_REWARD * burned_slot_count;

    assert_eq!(
        aggregate_burn.value,
        Coin::from_u64_unchecked(burned_reward)
    );
    assert!(
        (payable_reward.value == Coin::from_u64_unchecked(payable_base_reward + remainder)
            && burn_address_payout.value == Coin::from_u64_unchecked(burn_address_base_reward))
            || (payable_reward.value == Coin::from_u64_unchecked(payable_base_reward)
                && burn_address_payout.value
                    == Coin::from_u64_unchecked(burn_address_base_reward + remainder))
    );

    assert_eq!(
        setup.blockchain.finalize_previous_batch(&macro_block),
        [
            Inherent::Reward {
                validator_address: setup.validator_address.clone(),
                target: validators.payable_reward_address.clone(),
                value: payable_reward.value,
            },
            Inherent::Reward {
                validator_address: validators.burn_address_validator,
                target: Address::burn_address(),
                value: burn_address_payout.value,
            },
            Inherent::Reward {
                validator_address: Address::burn_address(),
                target: Address::burn_address(),
                value: aggregate_burn.value,
            },
            Inherent::FinalizeBatch,
        ]
    );
}

#[test]
fn it_can_penalize_delayed_batch() {
    let genesis_block_number = Policy::genesis_block_number();
    let time = Arc::new(OffsetTime::new());
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let blockchain = Arc::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    );

    // Delay in ms, so this means a 30s delay. For a 1m target batch time, this represents half of it
    let delay = 30000;

    let previous_timestamp = blockchain.state.election_head.header.timestamp;

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

    let staking_contract = blockchain.get_staking_contract();
    let next_batch_initial_punished_set = staking_contract
        .punished_slots
        .current_batch_punished_slots();

    let macro_header = MacroHeader {
        network: NetworkId::UnitAlbatross,
        version: 1,
        block_number: 42 + genesis_block_number,
        round: 0,
        timestamp: next_timestamp,
        next_batch_initial_punished_set,
        ..Default::default()
    };

    let staking_contract = blockchain.get_staking_contract();
    let reward_transactions =
        blockchain.create_reward_transactions(&macro_header, &staking_contract, None);

    let body = MacroBody {
        transactions: reward_transactions,
    };

    let macro_block = MacroBlock {
        header: macro_header,
        body: Some(body),
        justification: None,
    };

    // Simple case. Expect 1x FinalizeBatch, 1x Reward to validator
    let inherents = blockchain.finalize_previous_batch(&macro_block);
    assert_eq!(inherents.len(), 2);

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    for inherent in &inherents {
        match inherent {
            Inherent::Reward { value, .. } => {
                assert_eq!(
                    *value,
                    Coin::from_u64_unchecked((max_reward as f64 * penalty) as u64)
                );
                got_reward = true;
            }
            Inherent::FinalizeBatch => {
                got_finalize_batch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch);
}

#[test]
/// Create a skip block and check that correct inherents are produced.
fn it_correctly_creates_inherents_from_skip_block() {
    let temp_producer1 = TemporaryBlockProducer::new();
    let skip_block = temp_producer1.next_block(vec![], true);
    let skip_block = skip_block.unwrap_micro();

    let blockchain_rg = temp_producer1.blockchain.read();
    let slot = blockchain_rg
        .get_proposer_at(skip_block.block_number(), skip_block.block_number(), None)
        .unwrap();

    let skip_block_info = SkipBlockInfo::from_micro_block(&skip_block);

    // Create the inherents from any forks or skip block info.
    let inherents = blockchain_rg.create_punishment_inherents(
        skip_block.block_number(),
        &skip_block.body.as_ref().unwrap().equivocation_proofs,
        skip_block_info,
        None,
    );

    // Check inherents are correct.
    assert_eq!(
        inherents,
        vec![Inherent::Penalize {
            slot: PenalizedSlot {
                slot: slot.number,
                validator_address: slot.validator.address,
                offense_event_block: skip_block.block_number()
            }
        }]
    );
}

#[test]
/// Create a block with fork proof and check that correct inherents are produced.
fn it_correctly_creates_inherents_from_fork_proof() {
    let temp_producer1 = TemporaryBlockProducer::new();
    // Create block 1 of the fork (which is not pushed to the blockchain).
    let micro_block_fork1 = temp_producer1.next_block_no_push(vec![], false);
    let micro_block_fork1 = micro_block_fork1.unwrap_micro();

    // Create block 2 of the fork (which *is* pushed to the blockchain).
    let micro_block_fork2 = temp_producer1.next_block(vec![0x42], false);
    let micro_block_fork2 = micro_block_fork2.unwrap_micro();

    // Create a follow up block, which will contain the fork proof.
    let reporting_micro_block = temp_producer1.next_block(vec![], false);
    let mut reporting_micro_block = reporting_micro_block.unwrap_micro();

    // Produce and add the fork proof.
    let fork_proof = ForkProof::new(
        validator_address(),
        micro_block_fork1.header.clone(),
        micro_block_fork1
            .justification
            .clone()
            .unwrap()
            .unwrap_micro(),
        micro_block_fork2.header.clone(),
        micro_block_fork2
            .justification
            .clone()
            .unwrap()
            .unwrap_micro(),
    );
    reporting_micro_block
        .body
        .as_mut()
        .unwrap()
        .equivocation_proofs
        .push(fork_proof.into());

    let blockchain_rg = temp_producer1.blockchain.read();
    let slot = blockchain_rg
        .get_proposer_at(
            micro_block_fork1.block_number(),
            micro_block_fork1.block_number(),
            None,
        )
        .unwrap();

    let skip_block_info = SkipBlockInfo::from_micro_block(&reporting_micro_block);

    // Create the inherents from any forks or skip block info.
    let inherents = blockchain_rg.create_punishment_inherents(
        reporting_micro_block.block_number(),
        &reporting_micro_block.body.unwrap().equivocation_proofs,
        skip_block_info,
        None,
    );

    // Check inherents are correct.
    assert_eq!(
        inherents,
        vec![Inherent::Jail {
            jailed_validator: JailedValidator {
                slots: slot.validator.slots,
                validator_address: slot.validator.address,
                offense_event_block: micro_block_fork1.block_number(),
            },
            new_epoch_slot_range: None
        }]
    );
}

#[test]
/// Create a block with fork proof in the following epoch and check that correct inherents are produced.
fn it_correctly_creates_inherents_in_next_epoch_from_fork_proof() {
    let temp_producer1 = TemporaryBlockProducer::new();
    // Fill the blockchain with enough blocks to be in the last batch of the first epoch.
    for _ in 0..Policy::blocks_per_epoch() - 2 {
        temp_producer1.next_block(vec![], false);
    }

    // Create block 1 of the fork (which is not pushed to the blockchain).
    let micro_block_fork1 = temp_producer1.next_block_no_push(vec![], false);
    let micro_block_fork1 = micro_block_fork1.unwrap_micro();

    // Create block 2 of the fork (which *is* pushed to the blockchain).
    let micro_block_fork2 = temp_producer1.next_block(vec![0x42], false);
    let micro_block_fork2 = micro_block_fork2.unwrap_micro();

    // Create macro block.
    temp_producer1.next_block(vec![], false);

    // Create a follow up block in the next epoch, which will contain the fork proof.
    let reporting_micro_block = temp_producer1.next_block(vec![], false);
    let mut reporting_micro_block = reporting_micro_block.unwrap_micro();

    assert_ne!(
        Policy::epoch_at(micro_block_fork1.block_number()),
        Policy::epoch_at(reporting_micro_block.block_number())
    );

    // Produce and add the fork proof.
    let fork_proof = ForkProof::new(
        validator_address(),
        micro_block_fork1.header.clone(),
        micro_block_fork1
            .justification
            .clone()
            .unwrap()
            .unwrap_micro(),
        micro_block_fork2.header.clone(),
        micro_block_fork2
            .justification
            .clone()
            .unwrap()
            .unwrap_micro(),
    );
    reporting_micro_block
        .body
        .as_mut()
        .unwrap()
        .equivocation_proofs
        .push(fork_proof.into());

    let blockchain_rg = temp_producer1.blockchain.read();
    let slot = blockchain_rg
        .get_proposer_at(
            micro_block_fork1.block_number(),
            micro_block_fork1.block_number(),
            None,
        )
        .unwrap();
    let current_epoch_validator = blockchain_rg
        .current_validators()
        .expect("We need to have validators")
        .get_validator_by_address(&slot.validator.address)
        .unwrap()
        .clone();

    let skip_block_info = SkipBlockInfo::from_micro_block(&reporting_micro_block);

    // Create the inherents from any forks or skip block info.
    let inherents = blockchain_rg.create_punishment_inherents(
        reporting_micro_block.block_number(),
        &reporting_micro_block.body.unwrap().equivocation_proofs,
        skip_block_info,
        None,
    );

    // Check inherents are correct.
    assert_eq!(
        inherents,
        vec![Inherent::Jail {
            jailed_validator: JailedValidator {
                slots: slot.validator.slots,
                validator_address: slot.validator.address,
                offense_event_block: micro_block_fork1.block_number(),
            },
            new_epoch_slot_range: Some(current_epoch_validator.slots)
        }]
    );
}

/// Create a block with double proposal proof and check that correct inherents are produced.
#[test]
fn it_correctly_creates_inherents_from_double_proposal_proof() {
    let signing_key = signing_key();

    let temp_producer = TemporaryBlockProducer::new();
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }
    let header1 = temp_producer
        .next_block_no_push(vec![], false)
        .unwrap_macro()
        .header;
    let header2 = temp_producer
        .next_block(vec![], false)
        .unwrap_macro()
        .header;

    let block_number = header1.block_number;
    let round = header1.round;

    let proposal1 = TendermintProposal {
        proposal: header1,
        round: 0,
        valid_round: None,
    };
    let proposal2 = TendermintProposal {
        proposal: header2,
        round: 0,
        valid_round: None,
    };
    let justification1 = signing_key.sign(proposal1.hash().as_bytes());
    let justification2 = signing_key.sign(proposal2.hash().as_bytes());

    // Produce the double proposal proof.
    let double_proposal_proof = DoubleProposalProof::new(
        validator_address(),
        proposal1,
        justification1,
        proposal2,
        justification2,
    );

    let mut reporting_micro_block = temp_producer.next_block(vec![], false).unwrap_micro();
    reporting_micro_block
        .body
        .as_mut()
        .unwrap()
        .equivocation_proofs
        .push(double_proposal_proof.clone().into());
    let blockchain = temp_producer.blockchain.read();

    // Check that the double proposal proof is valid.
    blockchain
        .verify_equivocation_proofs(
            &Block::Micro(reporting_micro_block),
            &blockchain.read_transaction(),
        )
        .unwrap();

    let blockchain = temp_producer.blockchain.read();
    let slot = blockchain
        .get_proposer_at(block_number, round, None)
        .unwrap();

    // Create the inherents from the double proposal proof.
    let inherents = blockchain.create_punishment_inherents(
        block_number + 1,
        &[double_proposal_proof.into()],
        None,
        None,
    );

    // Check inherents are correct.
    assert_eq!(
        inherents,
        vec![Inherent::Jail {
            jailed_validator: JailedValidator {
                slots: slot.validator.slots,
                validator_address: slot.validator.address,
                offense_event_block: block_number,
            },
            new_epoch_slot_range: None
        }]
    );
}

/// Create a block with double vote proof and check that correct inherents are produced.
#[test]
fn it_correctly_creates_inherents_from_double_vote_proof() {
    let voting_key = voting_key();

    let temp_producer = TemporaryBlockProducer::new();
    for _ in 0..Policy::blocks_per_batch() - 1 {
        temp_producer.next_block(vec![], false);
    }
    let header = temp_producer
        .next_block(vec![], false)
        .unwrap_macro()
        .header;

    // Produce the double vote proof.
    let validators = temp_producer
        .blockchain
        .read()
        .get_validators_for_epoch(Policy::epoch_at(header.block_number), None)
        .unwrap();
    let validator = validators.validators[0].clone();
    let tendermint_id = TendermintIdentifier {
        network: header.network,
        block_number: header.block_number,
        round_number: header.round,
        step: TendermintStep::PreVote,
    };
    let signature1 = voting_key.sign(&TendermintVote {
        proposal_hash: None,
        id: tendermint_id.clone(),
    });
    let signature2 = voting_key.sign(&TendermintVote {
        proposal_hash: Some(Blake2sHash::default()),
        id: tendermint_id.clone(),
    });
    let slots1 = validator.slots.clone();
    let mut slots2 = validator.slots.clone();
    slots2.next().unwrap();
    let double_vote_proof = DoubleVoteProof::new(
        tendermint_id,
        validator.address,
        None,
        AggregateSignature::from_signatures(
            &slots1.clone().map(|_| signature1).collect::<Vec<_>>(),
        ),
        slots1.clone().map(|i| i.into()).collect(),
        Some(Blake2sHash::default()),
        AggregateSignature::from_signatures(
            &slots2.clone().map(|_| signature2).collect::<Vec<_>>(),
        ),
        slots2.clone().map(|i| i.into()).collect(),
    );
    let mut reporting_micro_block = temp_producer.next_block(vec![], false).unwrap_micro();
    reporting_micro_block
        .body
        .as_mut()
        .unwrap()
        .equivocation_proofs
        .push(double_vote_proof.clone().into());

    let blockchain = temp_producer.blockchain.read();
    let slot = blockchain
        .get_proposer_at(header.block_number, header.round, None)
        .unwrap();

    // Check that the double vote proof is valid.
    blockchain
        .verify_equivocation_proofs(
            &Block::Micro(reporting_micro_block),
            &blockchain.read_transaction(),
        )
        .unwrap();

    // Create the inherents from the double vote proof.
    let inherents = blockchain.create_punishment_inherents(
        header.block_number + 1,
        &[double_vote_proof.into()],
        None,
        None,
    );

    // Check inherents are correct.
    assert_eq!(
        inherents,
        vec![Inherent::Jail {
            jailed_validator: JailedValidator {
                slots: slot.validator.slots,
                validator_address: slot.validator.address,
                offense_event_block: header.block_number,
            },
            new_epoch_slot_range: None
        }]
    );
}

#[test(tokio::test)]
async fn create_fork_proof() {
    // Build a fork using two producers.
    let producer1 = TemporaryBlockProducer::new();
    let producer2 = TemporaryBlockProducer::new();

    let mut fork_rx = BroadcastStream::new(producer1.blockchain.read().fork_notifier.subscribe());

    // Easy rebranch
    // [0] - [0] - [0] - [0]
    //          \- [0]
    let block = producer1.next_block(vec![], false);
    let _next_block = producer1.next_block(vec![0x48], false);
    producer2.push(block).unwrap();

    let fork = producer2.next_block(vec![], false);
    producer1.push(fork).unwrap();

    // Verify that the fork proof was generated
    assert!(fork_rx.next().await.is_some());
}

#[test]
fn it_can_create_version_upgrade_inherents() {
    let time = Arc::new(OffsetTime::new());
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let blockchain = Arc::new(
        Blockchain::new(
            env,
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    );

    let block_number = Policy::election_block_after(Policy::genesis_block_number());

    let staking_contract = blockchain.get_staking_contract();
    let active_validators = staking_contract.active_validators.clone();
    let next_batch_initial_punished_set = staking_contract
        .punished_slots
        .next_batch_initial_punished_set(block_number, &active_validators);

    let mut macro_header = MacroHeader {
        network: NetworkId::UnitAlbatross,
        version: 2,
        block_number,
        round: 0,
        timestamp: blockchain.state.election_head.header.timestamp + 20000,
        next_batch_initial_punished_set,
        ..Default::default()
    };

    let reward_transactions =
        blockchain.create_reward_transactions(&macro_header, &staking_contract, None);

    let body = MacroBody {
        transactions: reward_transactions,
    };

    let macro_block = MacroBlock {
        header: macro_header.clone(),
        body: Some(body.clone()),
        justification: None,
    };

    // Simple case. Expect 1x FinalizeBatch, 1x FinalizeEpoch, 1x Reward to validator, 1x Version Upgrade
    let inherents = blockchain.create_macro_block_inherents(&macro_block);
    assert_eq!(inherents.len(), 4);

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    let mut got_finalize_epoch = false;
    let mut got_version_upgrade = false;
    for inherent in &inherents {
        match inherent {
            Inherent::Reward { .. } => {
                got_reward = true;
            }
            Inherent::FinalizeBatch => {
                got_finalize_batch = true;
            }
            Inherent::FinalizeEpoch => {
                got_finalize_epoch = true;
            }
            Inherent::VersionUpgrade { new_version } => {
                got_version_upgrade = true;
                assert_eq!(*new_version, 2);
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch && got_finalize_epoch && got_version_upgrade);

    // Downgrading version will remove version upgrade.
    macro_header.version = 1;

    let macro_block = MacroBlock {
        header: macro_header.clone(),
        body: Some(body),
        justification: None,
    };

    // Simple case. Expect 1x FinalizeBatch, 1x FinalizeEpoch, 1x Reward to validator
    let inherents = blockchain.create_macro_block_inherents(&macro_block);
    assert_eq!(inherents.len(), 3);

    let mut got_reward = false;
    let mut got_finalize_batch = false;
    let mut got_finalize_epoch = false;
    for inherent in &inherents {
        match inherent {
            Inherent::Reward { .. } => {
                got_reward = true;
            }
            Inherent::FinalizeBatch => {
                got_finalize_batch = true;
            }
            Inherent::FinalizeEpoch => {
                got_finalize_epoch = true;
            }
            _ => panic!(),
        }
    }
    assert!(got_reward && got_finalize_batch && got_finalize_epoch);
}

#[test]
fn it_burns_all_rewards_when_all_slots_penalized() {
    let setup = RewardTestSetup::new();
    // Penalize ALL slots so every slot has zero eligible count.
    setup.penalize_slots(0..Policy::SLOTS);

    // This must not panic — previously it would trigger
    // "Must have positive total probability" in DiscreteDistribution::new().
    let macro_block = setup.create_macro_block();
    let reward_transactions = &macro_block.body.as_ref().unwrap().transactions;

    // All rewards should be burned: expect exactly one burn transaction.
    assert_eq!(
        reward_transactions.len(),
        1,
        "Expected exactly one burn transaction"
    );
    assert_eq!(
        reward_transactions[0].recipient,
        Address::burn_address(),
        "The only transaction should burn rewards"
    );
    assert_eq!(
        reward_transactions[0].validator_address,
        Address::burn_address(),
    );
    // The burn amount should equal the full reward pot (slot rewards + remainder).
    assert!(
        !reward_transactions[0].value.is_zero(),
        "Burned reward should be non-zero"
    );
}
