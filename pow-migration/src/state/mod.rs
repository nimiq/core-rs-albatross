pub mod types;

use std::{collections::HashMap, ops::Range, str::FromStr, vec};

use nimiq_bls::PublicKey as BlsPublicKey;
use nimiq_genesis_builder::config::{
    GenesisAccount, GenesisHTLC, GenesisStaker, GenesisVestingContract,
};
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::coin::Coin;
use nimiq_rpc::{
    primitives::{
        BasicAccount as PoWBasicAccount, Block, HTLCAccount as PoWHTLCAccount, TransactionDetails,
        VestingAccount as PoWVestingAccount,
    },
    Client,
};
use nimiq_serde::Deserialize;
use nimiq_transaction::account::htlc_contract::{AnyHash, AnyHash32, AnyHash64};

use crate::state::types::{Error, GenesisAccounts, GenesisValidator};

// POW estimated block time in milliseconds
const POW_BLOCK_TIME_MS: u64 = 60 * 1000; // 1 min
                                          // PoS validator deposit
const VALIDATOR_DEPOSIT: u64 = 10; // FixMe: This should match the actual PoS validator deposit.

fn pos_basic_account_from_account(pow_account: &PoWBasicAccount) -> Result<GenesisAccount, Error> {
    let address = Address::from_user_friendly_address(&pow_account.address)?;
    let balance = Coin::try_from(pow_account.balance)?;
    Ok(GenesisAccount { address, balance })
}

fn pos_vesting_account_from_account(
    pow_account: &PoWVestingAccount,
    cutting_block: &Block,
    pos_genesis_ts: u64,
) -> Result<GenesisVestingContract, Error> {
    let owner = Address::from_user_friendly_address(&pow_account.owner_address)?;
    let address = Address::from_user_friendly_address(&pow_account.address)?;
    let balance = Coin::try_from(pow_account.balance)?;
    let start_time = if pow_account.vesting_start <= cutting_block.number {
        cutting_block.timestamp as u64
    } else {
        (pow_account.vesting_start - cutting_block.number) as u64 * POW_BLOCK_TIME_MS
            + pos_genesis_ts
    };
    let time_step = pow_account.vesting_step_blocks as u64 * POW_BLOCK_TIME_MS;
    let step_amount = Coin::try_from(pow_account.vesting_step_amount)?;
    let total_amount = Coin::try_from(pow_account.vesting_total_amount)?;
    Ok(GenesisVestingContract {
        address,
        owner,
        balance,
        start_time,
        step_amount,
        time_step,
        total_amount,
    })
}

fn pos_htlc_account_from_account(
    pow_account: &PoWHTLCAccount,
    cutting_block: &Block,
    pos_genesis_ts: u64,
) -> Result<GenesisHTLC, Error> {
    let address = Address::from_user_friendly_address(&pow_account.address)?;
    let recipient = Address::from_user_friendly_address(&pow_account.recipient_address)?;
    let sender = Address::from_user_friendly_address(&pow_account.sender_address)?;
    let balance = Coin::try_from(pow_account.balance)?;
    let hash_count = pow_account.hash_count;
    let timeout = if pow_account.timeout <= cutting_block.number {
        cutting_block.timestamp as u64
    } else {
        (pow_account.timeout - cutting_block.number) as u64 * POW_BLOCK_TIME_MS + pos_genesis_ts
    };
    let total_amount = Coin::try_from(pow_account.total_amount)?;
    let hash_root = pos_anyhash_from_hash_root(&pow_account.hash_root, pow_account.hash_algorithm)?;
    Ok(GenesisHTLC {
        address,
        recipient,
        sender,
        balance,
        hash_root,
        hash_count,
        timeout,
        total_amount,
    })
}

fn pos_anyhash_from_hash_root(hash_root: &str, algorithm: u8) -> Result<AnyHash, Error> {
    match algorithm {
        1u8 => Ok(AnyHash::Blake2b(AnyHash32::from_str(hash_root)?)),
        3u8 => Ok(AnyHash::Sha256(AnyHash32::from_str(hash_root)?)),
        4u8 => Ok(AnyHash::Sha512(AnyHash64::from_str(hash_root)?)),
        _ => Err(Error::InvalidValue),
    }
}

/// Gets the set of the Genesis Accounts by taking a snapshot of the accounts in
/// a specific block number defined by `cutting_block`.
pub async fn get_accounts(
    client: &Client,
    cutting_block: &Block,
    pos_genesis_ts: u64,
) -> Result<GenesisAccounts, Error> {
    let mut genesis_accounts = GenesisAccounts {
        vesting_accounts: vec![],
        basic_accounts: vec![],
        htlc_accounts: vec![],
    };
    let mut start_prefix = "".to_string();
    loop {
        let chunk = client
            .get_accounts_tree_chunk(&cutting_block.hash, &start_prefix)
            .await?;
        if chunk.nodes.is_empty() || start_prefix == chunk.tail {
            break;
        }
        start_prefix = chunk.tail;
        log::debug!(size = chunk.nodes.len(), "Processing accounts tree chunk");
        for node in chunk.nodes {
            match node.account {
                nimiq_rpc::primitives::Account::Basic(pow_account) => {
                    let pos_basic_account = pos_basic_account_from_account(&pow_account)?;
                    genesis_accounts.basic_accounts.push(pos_basic_account);
                }
                nimiq_rpc::primitives::Account::Vesting(pow_account) => {
                    let pos_vesting_account = pos_vesting_account_from_account(
                        &pow_account,
                        cutting_block,
                        pos_genesis_ts,
                    )?;
                    genesis_accounts.vesting_accounts.push(pos_vesting_account);
                }
                nimiq_rpc::primitives::Account::HTLC(pow_account) => {
                    let pos_htlc_account =
                        pos_htlc_account_from_account(&pow_account, cutting_block, pos_genesis_ts)?;
                    genesis_accounts.htlc_accounts.push(pos_htlc_account);
                }
            }
        }
    }
    Ok(genesis_accounts)
}

/// Gets the set of validators registered in the PoW chain by parsing the required
/// transactions within the validator registration window defined by the
/// `block_window` range.
pub async fn get_validators(
    client: &Client,
    block_window: Range<u32>,
) -> Result<Vec<GenesisValidator>, Error> {
    let mut txns_by_sender = HashMap::<String, Vec<TransactionDetails>>::new();
    let mut transactions = client
        .get_transactions_by_address(&Address::burn_address().to_string(), u16::MAX)
        .await?;
    let mut possible_validators = HashMap::new();
    let mut validators = vec![];

    // Remove any transaction outside of the validator registration window
    transactions.retain(|txn| block_window.contains(&txn.block_number));

    // Group all transactions by its sender
    for txn in transactions {
        txns_by_sender
            .entry(txn.from_address.clone())
            .and_modify(|txns| txns.push(txn.clone()))
            .or_insert(vec![txn]);
    }
    // First look for the 6 transactions that carries the validator data
    for (_, txns) in txns_by_sender.iter() {
        let mut signing_key = SchnorrPublicKey::default();
        let mut address: Address = Address::default();
        let mut voting_key = vec![vec![0u8]; 5];
        let mut txns_parsed = [false; 6];
        for txn in txns {
            if let Some(data_hex) = &txn.data {
                if let Ok(data) = hex::decode(data_hex) {
                    if data.len() < 64 {
                        continue;
                    }
                    match data[0] {
                        1u8 => {
                            if let Ok(sk) = SchnorrPublicKey::from_bytes(&data[12..44]) {
                                if let Ok(addr) = Address::deserialize_from_vec(&data[44..]) {
                                    signing_key = sk;
                                    address = addr;
                                    txns_parsed[0] = true;
                                }
                            }
                        }
                        2u8 => {
                            voting_key[0] = data[7..].to_vec();
                            txns_parsed[1] = true;
                        }
                        3u8 => {
                            voting_key[1] = data[7..].to_vec();
                            txns_parsed[2] = true;
                        }
                        4u8 => {
                            voting_key[2] = data[7..].to_vec();
                            txns_parsed[3] = true;
                        }
                        5u8 => {
                            voting_key[3] = data[7..].to_vec();
                            txns_parsed[4] = true;
                        }
                        6u8 => {
                            voting_key[4] = data[7..].to_vec();
                            txns_parsed[5] = true;
                        }
                        _ => {}
                    }
                }
            }
        }

        // If we already parsed the 6 transactions, we just need to parse the BLS public key to see if we have found a possible validator
        if txns_parsed.into_iter().all(|parsed| parsed) {
            let mut voting_key_bytes = vec![];
            for mut vk in voting_key {
                voting_key_bytes.append(&mut vk);
            }
            if let Ok(voting_key) = BlsPublicKey::deserialize_from_vec(&voting_key_bytes) {
                let possible_validator = GenesisValidator {
                    balance: Coin::ZERO,
                    validator: nimiq_genesis_builder::config::GenesisValidator {
                        validator_address: address.clone(),
                        signing_key,
                        voting_key,
                        reward_address: address.clone(),
                    },
                };
                log::debug!(%address, "Found possible validator");
                possible_validators.insert(address, possible_validator);
            } else {
                log::warn!(
                    %address,
                    "Possible validator with invalid BLS public key registered"
                );
            }
        }
    }

    // Now look for the commit transaction
    for (_, txns) in txns_by_sender.iter() {
        for txn in txns.iter().filter(|&txn| txn.value >= VALIDATOR_DEPOSIT) {
            if let Some(data) = &txn.data {
                if let Ok(address_bytes) = hex::decode(data) {
                    if let Ok(address_str) = std::str::from_utf8(&address_bytes) {
                        if let Ok(address) = Address::from_str(address_str) {
                            if let Some(mut validator) = possible_validators.remove(&address) {
                                log::info!(%address, "Found commit transaction for validator");
                                // If the transaction had a value greater than the deposit, the excess will be converted
                                // to stake by `get_stakers`.
                                validator.balance = Coin::from_u64_unchecked(txn.value);
                                validators.push(validator);
                            } else {
                                log::warn!(
                                    %address,
                                    "Found commit transaction for unknown validator"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(validators)
}

/// Gets the set of stakers registered in the PoW chain by parsing the required
/// transactions within the pre-stake registration window defined by the
/// `block_window` range. It uses a set of already registered validators and
/// returns an updated set of validators along with the stakers.
pub async fn get_stakers(
    client: &Client,
    registered_validators: &[GenesisValidator],
    block_window: Range<u32>,
) -> Result<(Vec<GenesisStaker>, Vec<GenesisValidator>), Error> {
    let mut txns_by_sender = HashMap::<String, Vec<TransactionDetails>>::new();
    let mut transactions = client
        .get_transactions_by_address(&Address::burn_address().to_string(), u16::MAX)
        .await?;
    let mut validators = HashMap::new();
    let mut stakers = vec![];

    // Build the hashmap for validators and check if there needs to be a staker for the validator address
    for validator in registered_validators {
        validators.insert(
            validator.validator.validator_address.to_string(),
            validator.clone(),
        );
        if validator.balance > Coin::from_u64_unchecked(VALIDATOR_DEPOSIT) {
            stakers.push(GenesisStaker {
                staker_address: validator.validator.validator_address.clone(),
                balance: validator.balance - Coin::from_u64_unchecked(VALIDATOR_DEPOSIT),
                delegation: validator.validator.validator_address.clone(),
            })
        }
    }

    // Remove any transaction outside of the validator registration window
    transactions.retain(|txn| block_window.contains(&txn.block_number));

    // Group all transactions by its sender
    for txn in transactions {
        txns_by_sender
            .entry(txn.from_address.clone())
            .and_modify(|txns| txns.push(txn.clone()))
            .or_insert(vec![txn]);
    }

    // Now look for the commit transaction
    for (_, txns) in txns_by_sender.iter() {
        for txn in txns.iter() {
            if let Some(data) = &txn.data {
                if let Ok(address_bytes) = hex::decode(data) {
                    if let Ok(address_str) = std::str::from_utf8(&address_bytes) {
                        if let Ok(address) = Address::from_str(address_str) {
                            if let Some(validator) = validators.get_mut(address_str) {
                                log::info!(staker_address=txn.from_address, validator_address=%address, "Found pre-stake transaction for validator");
                                if let Ok(staker_address) = Address::from_str(&txn.from_address) {
                                    let stake = Coin::from_u64_unchecked(txn.value);
                                    validator.balance += stake;
                                    stakers.push(GenesisStaker {
                                        staker_address,
                                        balance: stake,
                                        delegation: validator.validator.validator_address.clone(),
                                    });
                                } else {
                                    log::error!(
                                        staker_address = txn.from_address,
                                        "Could not build staker address from transaction sender"
                                    );
                                }
                            } else {
                                log::warn!(
                                    staker_address = txn.from_address,
                                    "Found pre-staking transaction for unknown validator, ignored"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok((stakers, validators.into_values().collect()))
}
