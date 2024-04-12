pub mod types;

use std::{collections::HashMap, ops::Range, str::FromStr, vec};

use nimiq_bls::PublicKey as BlsPublicKey;
use nimiq_genesis_builder::config::{
    GenesisAccount, GenesisHTLC, GenesisStaker, GenesisVestingContract,
};
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::{coin::Coin, policy::Policy};
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
    if pos_genesis_ts < cutting_block.timestamp as u64 {
        log::error!(
            pos_genesis_ts,
            cutting_block_number_ts = cutting_block.timestamp,
            "Provided PoS genesis timestamp is less than the timestamp of the provided cutting block"
        );
        return Err(Error::InvalidTimestamp(pos_genesis_ts));
    }
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
    if pos_genesis_ts < cutting_block.timestamp as u64 {
        log::error!(
            pos_genesis_ts,
            cutting_block_number_ts = cutting_block.timestamp,
            "Provided PoS genesis timestamp is less than the timestamp of the provided cutting block"
        );
        return Err(Error::InvalidTimestamp(pos_genesis_ts));
    }
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
    for (_, txns) in txns_by_sender.iter_mut() {
        // First sort the transactions by this sender by timestamp
        txns.sort_by_cached_key(|transaction| transaction.timestamp);

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
                        inactive_from: None,
                        jailed_from: None,
                        retired: false,
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
        for txn in txns
            .iter()
            .filter(|&txn| txn.value >= Policy::VALIDATOR_DEPOSIT)
        {
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
    let mut stakers = HashMap::new();

    // Build the hashmap for validators and check if there needs to be a staker for the validator address
    for registered_validator in registered_validators {
        validators.insert(
            registered_validator.validator.validator_address.to_string(),
            registered_validator.clone(),
        );
        if registered_validator.balance > Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT) {
            let staker_address = registered_validator.validator.validator_address.clone();
            stakers.insert(
                staker_address.clone(),
                GenesisStaker {
                    staker_address,
                    balance: registered_validator.balance
                        - Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
                    delegation: registered_validator.validator.validator_address.clone(),
                    inactive_balance: Coin::ZERO,
                    inactive_from: None,
                },
            );
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

    // Now look for the pre-stake transaction
    for (_, txns) in txns_by_sender.iter_mut() {
        // First order transactions by timestamp
        txns.sort_by_cached_key(|transaction| transaction.timestamp);
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
                                    stakers
                                        .entry(staker_address.clone())
                                        .and_modify(|staker| {
                                            // If we have an entry for this staker, treat this as a switch to another
                                            // validator or an increase of the stake (if the validator isn't changed)
                                            staker.delegation =
                                                validator.validator.validator_address.clone();
                                            staker.balance += stake;
                                        })
                                        .or_insert(GenesisStaker {
                                            staker_address,
                                            balance: stake,
                                            delegation: validator
                                                .validator
                                                .validator_address
                                                .clone(),
                                            inactive_balance: Coin::ZERO,
                                            inactive_from: None,
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

    Ok((
        stakers.into_values().collect(),
        validators.into_values().collect(),
    ))
}

#[cfg(test)]
mod test {

    use nimiq_rpc::primitives::{BasicAccount, HTLCAccount, TransactionSequence, VestingAccount};

    use super::*;

    static BASIC_ACCOUNTS: &str = r#"
[
  {
    "id": "5e831d9063ad65715875412848218a9691c0bb75",
    "address": "NQ14 BS1H T433 MMJP 2N3M 84L4 G8CA JS8U 1ETM",
    "balance": 10,
    "type": 0
  },
  {
    "id": "3b09b06a119a82941e2bbd7d1d73bb4040a8888b",
    "address": "NQ69 7C4T 0SGH KA19 87HB PMXH SUVT 810A H24B",
    "balance": 1,
    "type": 0
  },
  {
    "id": "845fb0c6b72c8257ff7ae71eefff012d96e6c179",
    "address": "NQ98 GHFT 1HMP 5J15 FYTS UUFE YYQ1 5NBE DGBR",
    "balance": 10000,
    "type": 0
  },
  {
    "id": "472a8a78783c5b15091b97deccaa636261bc72fb",
    "address": "NQ75 8UM8 LX3Q 7HDH A28T JYFC RAK3 C9GT QUPT",
    "balance": 10,
    "type": 0
  }
]"#;
    static VESTING_ACCOUNTS: &str = r#"
[
  {
    "id": "924d35e70ea45104a6e1c34652598a6ee6995c73",
    "address": "NQ76 J96K BRQE LH8G 99P1 QD35 4NCA DTK9 JP3K",
    "balance": 150000000000,
    "type": 1,
    "owner": "2cfe57a93f73e4e553dc279fd0f27a2a100d8344",
    "ownerAddress": "NQ32 5KY5 FA9Y EFJE ALXU 4XFV 1UKS 5880 T0S4",
    "vestingStart": 1,
    "vestingStepBlocks": 129600,
    "vestingStepAmount": 75000000000,
    "vestingTotalAmount": 150000000000
  },
  {
    "id": "2caa80bf426bdc19dc9c00d9a43e2da6350fc73d",
    "address": "NQ75 5JM8 1FS2 DFE1 KP4U 03CS 8FHD LQSG YHRV",
    "balance": 50083600000,
    "type": 1,
    "owner": "62a153a5eeabe57af690496251d9c2304168db50",
    "ownerAddress": "NQ45 CAGM 79FE MFJP MVLG 95H5 3NE2 610N HNSG",
    "vestingStart": 1,
    "vestingStepBlocks": 129600,
    "vestingStepAmount": 46641800000,
    "vestingTotalAmount": 93283600000
  },
  {
    "id": "d12e0f24ce0ba902afd72dd02d984798da887a14",
    "address": "NQ80 S4P0 X96E 1ELG 5BXP 5P82 T627 K3D8 GXGL",
    "balance": 333300000,
    "type": 1,
    "owner": "268ee9cb7a2a1b4fb10a393163ebf1bb253dca94",
    "ownerAddress": "NQ89 4S7E KJTS 58DL YC8A 74QN 7SYH PCJK TJLL",
    "vestingStart": 1,
    "vestingStepBlocks": 129600,
    "vestingStepAmount": 166650000,
    "vestingTotalAmount": 333300000
  },
  {
    "id": "bb5049c1620eec25285e91ad1273713eb892def9",
    "address": "NQ50 PD84 KGB2 1TN2 AA2X J6NH 4UTH 7SU9 5PPR",
    "balance": 4104100000,
    "type": 1,
    "owner": "8a417d7cceb41b0084356247d2bc0cc3f7fb3464",
    "ownerAddress": "NQ73 H90P SY6E NGDG 111M C93V 5F0C QFTY ND34",
    "vestingStart": 1,
    "vestingStepBlocks": 129600,
    "vestingStepAmount": 2052050000,
    "vestingTotalAmount": 4104100000
  }
]"#;
    static HTLC_ACCOUNTS: &str = r#"
[
  {
    "id": "9efd0f63cc51523a5595d336942406678b22ece6",
    "address": "NQ02 KTXG XQXC A593 LMCM SCT9 8906 CX5J 5T76",
    "balance": 1,
    "type": 2,
    "sender": "1227f787551b89b4e09d023413e0e116d8dabc88",
    "senderAddress": "NQ56 28KY F1SM 3E4T 9Q4V 08S1 7Q71 2TCD MF48",
    "recipient": "6107814f34e17c5ce7f2f53a9cdfea6aac6c4fb8",
    "recipientAddress": "NQ62 C43Q 2KRL U5X5 RRYJ XLV9 RPYA DAN6 QKVQ",
    "hashRoot": "e1425cfffd4b00195bc0d5405ac04cb39a380f8ca517e4786f134e26c4c94044",
    "hashAlgorithm": 3,
    "hashCount": 255,
    "timeout": 421805,
    "totalAmount": 1
  },
  {
    "id": "4f79d3c2af9d6dc56a08fd73a5a8b179f8273c97",
    "address": "NQ27 9VUV 7GMF KMNU ASG8 YMRS BA5H F7U2 EF4P",
    "balance": 2,
    "type": 2,
    "sender": "6dccd1aa2b767f6790a2272542f8c826e5c1320e",
    "senderAddress": "NQ70 DP6D 3AHB ERYN F452 4UJL 5X68 4TJU 2CGE",
    "recipient": "eb08db69e9b743d40f013bdc6257edb3026aeddb",
    "recipientAddress": "NQ02 VC4D NSF9 NV1V 83Q1 7FE6 4MYD NC16 MTET",
    "hashRoot": "3434366636653237373432303632373237353734363536363666373236333635",
    "hashAlgorithm": 3,
    "hashCount": 1,
    "timeout": 284968,
    "totalAmount": 3
  },
  {
    "id": "deca4af7cee60d33bb978b81a24ffca7cb657012",
    "address": "NQ05 TT54 MVXE UQ6K 7EUP HE0S 4KYU LY5N AU0J",
    "balance": 1,
    "type": 2,
    "sender": "1227f787551b89b4e09d023413e0e116d8dabc88",
    "senderAddress": "NQ56 28KY F1SM 3E4T 9Q4V 08S1 7Q71 2TCD MF48",
    "recipient": "4000f22e0a80916f753727681f3af778e930d4cb",
    "recipientAddress": "NQ90 800F 4BGA G28N XV9P 4VL1 XEPP F3LK 1M6B",
    "hashRoot": "e1425cfffd4b00195bc0d5405ac04cb39a380f8ca517e4786f134e26c4c94044",
    "hashAlgorithm": 3,
    "hashCount": 255,
    "timeout": 421805,
    "totalAmount": 1
  },
  {
    "id": "c628c8e2a77637138672fdf90d8c63c1c04c2722",
    "address": "NQ37 QQLC HQM7 EQTH 71KJ YPUG T333 Q704 Q9R2",
    "balance": 1,
    "type": 2,
    "sender": "deec92cf871f08269a9c43574965b997386750be",
    "senderAddress": "NQ94 TTN9 5KU7 3U42 D6LU 8DBL JRDR JUU6 EL5X",
    "recipient": "026ef67bfe3e9bfcd2c6010cb9f8a46e9b2cb386",
    "recipientAddress": "NQ24 09PF CXYX 7SDY RLN6 046B KX54 DSDJ RCU6",
    "hashRoot": "4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d",
    "hashAlgorithm": 3,
    "hashCount": 255,
    "timeout": 421796,
    "totalAmount": 1
  }
]"#;

    fn get_block() -> Block {
        Block {
            number: 2815496,
            hash: "47215beabbc7353ead856e213f456146117a65e743a565d6065997f82b8a5e77".to_string(),
            pow: "00000000003e0e15517e92ac3470a21289d72b5b7ba682c927dc7ac3d6796ea7".to_string(),
            parent_hash: "cd35d5ada3224b8fa893af8539c6784cb0dcf9311839d2859b2d4254a22a62de"
                .to_string(),
            nonce: 2157594150,
            body_hash: "a6ca9f79be6e0dd6f9371f5b744f1622ea695b13fb1785d07c14d0e68877b894"
                .to_string(),
            accounts_hash: "0a2bbb2aa95759a5b90785c9440c8aa820310ff0e6e7f4808bffbf36418b8939"
                .to_string(),
            miner: "f3a2a520967fb046deee40af0c68c3dc43ef3238".to_string(),
            miner_address: "NQ04 XEHA A84N FXQ4 DPPE 82PG QS63 TH1X XCHQ".to_string(),
            difficulty: "4172434.6260065726".to_string(),
            extra_data:
                "6963656d696e696e672e636100582914a56f50200ee6456e29c3de5a1df61d090011ca144d1e02fc69"
                    .to_string(),
            size: 4125,
            timestamp: 1693429613,
            transactions: TransactionSequence::BlockHashes(vec![]),
        }
    }

    #[test]
    fn can_parse_basic_accounts() {
        let pow_accounts: Vec<BasicAccount> = serde_json::from_str(BASIC_ACCOUNTS).unwrap();
        for pow_account in pow_accounts {
            let pos_account = pos_basic_account_from_account(&pow_account).unwrap();
            assert_eq!(
                pos_account.balance,
                Coin::try_from(pow_account.balance).unwrap()
            );
            assert_eq!(pos_account.address.to_string(), pow_account.address);
        }
    }

    #[test]
    fn can_parse_vesting_accounts() {
        let pow_accounts: Vec<VestingAccount> = serde_json::from_str(VESTING_ACCOUNTS).unwrap();
        let cutting_block = get_block();

        // First test that we fail if we pass a timestamp that is less than the one in the cutting block number
        let ts = cutting_block.timestamp as u64 - 10;
        assert!(pos_vesting_account_from_account(&pow_accounts[0], &cutting_block, ts).is_err());

        // Now test with a proper TS
        let ts = cutting_block.timestamp as u64 + 10;
        for pow_account in pow_accounts {
            let pos_account =
                pos_vesting_account_from_account(&pow_account, &cutting_block, ts).unwrap();
            assert_eq!(
                pos_account.balance,
                Coin::try_from(pow_account.balance).unwrap()
            );
            assert_eq!(pos_account.address.to_string(), pow_account.address);
            assert_eq!(pos_account.owner.to_string(), pow_account.owner_address);
            assert_eq!(
                pos_account.total_amount,
                Coin::try_from(pow_account.vesting_total_amount).unwrap()
            );
            assert_eq!(
                pos_account.step_amount,
                Coin::try_from(pow_account.vesting_step_amount).unwrap()
            );
            assert_eq!(
                pos_account.time_step,
                pow_account.vesting_step_blocks as u64 * POW_BLOCK_TIME_MS
            );
            if pow_account.vesting_start < cutting_block.timestamp {
                assert_eq!(pos_account.start_time, cutting_block.timestamp as u64);
            } else {
                let timeout = (pow_account.vesting_start - cutting_block.timestamp) as u64
                    * POW_BLOCK_TIME_MS
                    + ts;
                assert_eq!(pos_account.start_time, timeout);
            }
        }
    }

    #[test]
    fn can_parse_htlc_accounts() {
        let pow_accounts: Vec<HTLCAccount> = serde_json::from_str(HTLC_ACCOUNTS).unwrap();
        let cutting_block = get_block();

        // First test that we fail if we pass a timestamp that is less than the one in the cutting block number
        let ts = cutting_block.timestamp as u64 - 10;
        assert!(pos_htlc_account_from_account(&pow_accounts[0], &cutting_block, ts).is_err());

        // Now test with a proper TS
        let ts = cutting_block.timestamp as u64 + 10;
        for pow_account in pow_accounts {
            let pos_account =
                pos_htlc_account_from_account(&pow_account, &cutting_block, ts).unwrap();
            assert_eq!(
                pos_account.balance,
                Coin::try_from(pow_account.balance).unwrap()
            );
            assert_eq!(
                pos_account.total_amount,
                Coin::try_from(pow_account.total_amount).unwrap()
            );
            assert_eq!(pos_account.address.to_string(), pow_account.address);
            assert_eq!(pos_account.sender.to_string(), pow_account.sender_address);
            assert_eq!(
                pos_account.recipient.to_string(),
                pow_account.recipient_address
            );
            assert_eq!(pos_account.hash_root.to_hex(), pow_account.hash_root);
            assert_eq!(pos_account.hash_count, pow_account.hash_count);
            if pow_account.timeout < cutting_block.timestamp {
                assert_eq!(pos_account.timeout, cutting_block.timestamp as u64);
            } else {
                let timeout =
                    (pow_account.timeout - cutting_block.timestamp) as u64 * POW_BLOCK_TIME_MS + ts;
                assert_eq!(pos_account.timeout, timeout);
            }
        }
    }
}
