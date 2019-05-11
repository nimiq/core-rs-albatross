#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate nimiq_bls as bls;
extern crate nimiq_block_albatross as block;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_account as account;

mod config;

use block::{MacroHeader, Slot};
use rand_chacha::ChaChaRng;
use hash::{Blake2bHasher, Blake2bHash, Hasher, Hash};
use rand::{SeedableRng, Rng};
use bls::bls12_381::SecretKey;
use chrono::{DateTime, Utc};
use std::convert::TryFrom;
use config::{GenesisConfig, GenesisStake};
use account::{StakingContract, BasicAccount, Account};
use crate::config::GenesisAccount;
use beserial::{Serialize, SerializingError};
use keys::Address;


lazy_static! {
    static ref STAKING_CONTRACT_ADDRESS: Address = Address::from_user_friendly_address("NQ02 4GCK GTFJ J8RF 41J0 VKFX 5QTM KSTG MRJP").unwrap();
}


fn main() {
    simple_logger::init().expect("Failed to initialize logging");

    let GenesisConfig {
        seed_message,
        timestamp,
        stakes,
        accounts
    } = GenesisConfig::from_file("albatross-testnet.toml");

    // if no timestamp given, take current time.
    let timestamp = timestamp.unwrap_or_else(Utc::now);

    // seed random number generator
    let seed_hash = Blake2bHasher::new().digest(seed_message.as_bytes());
    let mut csprng = ChaChaRng::from_seed(seed_hash.clone().into());

    //let staking_contract = generate_staking_contract(&stakes);
    //let accounts = generate_accounts(staking_contract, accounts);
    let header = generate_header(&mut csprng, &timestamp, &stakes);


    println!(r#"
                // Seed: '{}' ({})
                // Creation time: {}
                genesis_block: Block::Macro(MacroBlock {{
                    header: MacroHeader {{
                        version: 1,
                        slot_allocation: vec![/*TODO*/],
                        block_number: 1,
                        view_number: 0,
                        parent_macro_hash: "0000000000000000000000000000000000000000000000000000000000000000".into(),
                        seed: block_seed("{}"),
                        parent_hash: "0000000000000000000000000000000000000000000000000000000000000000".into(),
                        state_root: "0000000000000000000000000000000000000000000000000000000000000000".into(),
                        timestamp: {},
                    }},
                    justification: None, // PbftProof::new(),
                }}),
                genesis_hash: "{}".into(),
                genesis_accounts: vec!["#,
             seed_message,
             seed_hash,
             timestamp.to_rfc3339(),
             hex::encode(header.seed.serialize_to_vec()),
             header.timestamp,
             header.hash::<Blake2bHash>()
    );

    println!(r#"                    GenesisStakes::new()"#);
    for stake in &stakes {
        println!(r#"                        .stake("{}", "{}", {}, "{}")"#,
            stake.staker_address,
            stake.reward_address,
            stake.balance,
            hex::encode(stake.validator_key.serialize_to_vec()),
        );
    }
    println!(r#"                        .finalize(),"#);

    for account in accounts {
        println!(r#"                    basic_account("{}", {}),"#, account.address, account.balance);
    }

    println!(r#"                ],"#)

}

fn generate_header(csprng: &mut ChaChaRng, time: &DateTime<Utc>, stakes: &Vec<GenesisStake>) -> MacroHeader {
    let secret_key = SecretKey::generate(csprng);

    // Generate macro block seed by signing a random bit string
    let seed = secret_key.sign(&csprng.gen::<[u8; 32]>());

    MacroHeader {
        version: 1,
        slot_allocation: vec![],
        block_number: 1,
        view_number: 0,
        parent_macro_hash: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        seed,
        parent_hash: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        state_root: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        timestamp: u64::try_from(time.timestamp_millis()).expect("Timestamp negative"),
    }
}

/*

/// generate genesis staking contract
fn generate_staking_contract(stakes: &Vec<GenesisStake>) -> StakingContract {
    let mut contract = StakingContract::default();

    for stake in stakes {
        info!("Staking: staker={}, value={}", stake.staker_address.to_user_friendly_address(), stake.balance);
        contract.stake(&stake.staker_address, stake.balance, stake.validator_key.clone(), Some(stake.reward_address.clone()))
            .expect("Genesis staking failed");
    }

    contract
}

/// generate slot selection for genesis macro header
fn generate_slot_selection(_stakes: &Vec<GenesisStake>) -> Vec<Slot> {
    vec![]
}

fn generate_accounts(staking_contract: StakingContract, genesis_accounts: Vec<GenesisAccount>) -> Vec<(Address, Account)> {
    let mut accounts: Vec<(Address, Account)> = Vec::new();

    // push stacking contract
    accounts.push((STAKING_CONTRACT_ADDRESS.clone(), Account::Staking(staking_contract)));

    // push basic accounts
    for genesis_account in genesis_accounts {
        info!("Serializing account: {} {}", genesis_account.address.to_user_friendly_address(), genesis_account.balance);
        accounts.push((genesis_account.address, Account::Basic(BasicAccount {
            balance: genesis_account.balance
        })));
    }

    accounts
}
*/
