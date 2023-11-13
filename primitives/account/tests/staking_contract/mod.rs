use std::{cmp::max, vec};

use nimiq_account::{
    AccountTransactionInteraction, Accounts, BlockState, DataStoreWrite, StakingContract,
    StakingContractStoreWrite, TransactionLog,
};
use nimiq_bls::{
    CompressedPublicKey as BlsPublicKey, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey,
};
use nimiq_database::{
    traits::{Database, WriteTransaction},
    volatile::VolatileDatabase,
    DatabaseProxy,
};
use nimiq_keys::{Address, EdDSAPublicKey, KeyPair, PrivateKey};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
    EdDSASignatureProof, SignatureProof, Transaction,
};

mod punished_slots;
mod staker;
mod validator;

const VALIDATOR_ADDRESS: &str = "83fa05dbe31f85e719f4c4fd67ebdba2e444d9f8";
const VALIDATOR_PRIVATE_KEY: &str =
    "d0fbb3690f5308f457e245a3cc65ae8d6945155eadcac60d489ffc5583a60b9b";

const VALIDATOR_SIGNING_KEY: &str =
    "b300481ddd7af6be3cf5c123b7af2c21f87f4ac808c8b0e622eb85826124a844";
const VALIDATOR_SIGNING_SECRET_KEY: &str =
    "84c961b11b52a8244ffc5e9d0965bc2dfa6764970f8e4989d45901de401baf27";

const VALIDATOR_VOTING_KEY: &str = "713c60858b5c72adcf8b72b4dbea959d042769dcc93a0190e4b8aec92283548138833950aa214d920c17d3d19de27f6176d9fb21620edae76ad398670e17d5eba2f494b9b6901d457592ea68f9d35380c857ba44856ae037aff272ad6c1900442b426dde0bc53431e9ce5807f7ec4a05e71ce4a1e7e7b2511891521c4d3fd975764e3031ef646d48fa881ad88240813d40e533788f0dac2bc4d4c25db7b108c67dd28b7ec4c240cdc044badcaed7860a5d3da42ef860ed25a6db9c07be000a7f504f6d1b24ac81642206d5996b20749a156d7b39f851e60f228b19eef3fb3547469f03fc9764f5f68bc88e187ffee0f43f169acde847c78ea88029cdb19b91dd9562d60b607dd0347d67a0e33286c8908e4e9579a42685da95f06a9201";
const VALIDATOR_VOTING_SECRET_KEY: &str =
        "65100f4aa301ded3d9868c3d76052dd0dfede426b51af371dcd8a4a076f11651c86286d2891063ce7b78217a6e163f38ebfde7eb9dcbf5927b2278b00d77329141d44f070620dd6b995455a6cdfe8eee03f657ff255cfb8fb3460ce1135701";

const STAKER_ADDRESS: &str = "8c551fabc6e6e00c609c3f0313257ad7e835643c";
const STAKER_PRIVATE_KEY: &str = "62f21a296f00562c43999094587d02c0001676ddbd3f0acf9318efbcad0c8b43";

const NON_EXISTENT_ADDRESS: &str = "9cd82948650d902d95d52ea2ec91eae6deb0c9fe";
const NON_EXISTENT_PRIVATE_KEY: &str =
    "b410a7a583cbc13ef4f1cbddace30928bcb4f9c13722414bc4a2faaba3f4e187";

fn staker_address() -> Address {
    Address::from_hex(STAKER_ADDRESS).unwrap()
}

fn validator_address() -> Address {
    Address::from_hex(VALIDATOR_ADDRESS).unwrap()
}

fn non_existent_address() -> Address {
    Address::from_hex(NON_EXISTENT_ADDRESS).unwrap()
}

fn bls_key_pair(sk: &str) -> BlsKeyPair {
    BlsKeyPair::from(BlsSecretKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn bls_public_key(pk: &str) -> BlsPublicKey {
    BlsPublicKey::deserialize_from_vec(&hex::decode(pk).unwrap()).unwrap()
}

fn ed25519_key_pair(sk: &str) -> KeyPair {
    KeyPair::from(PrivateKey::deserialize_from_vec(&hex::decode(sk).unwrap()).unwrap())
}

fn ed25519_public_key(pk: &str) -> EdDSAPublicKey {
    EdDSAPublicKey::deserialize_from_vec(&hex::decode(pk).unwrap()).unwrap()
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateValidator { .. }
        | IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::AddStake { .. } => Transaction::new_extended(
            non_existent_address(),
            AccountType::Basic,
            vec![],
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            data.serialize_to_vec(),
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            1,
            NetworkId::Dummy,
        ),
        _ => Transaction::new_signaling(
            non_existent_address(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            100.try_into().unwrap(),
            data.serialize_to_vec(),
            1,
            NetworkId::Dummy,
        ),
    }
}

fn make_signed_incoming_transaction(
    data: IncomingStakingTransactionData,
    value: u64,
    in_key_pair: &KeyPair,
) -> Transaction {
    let mut tx = make_incoming_transaction(data, value);

    let in_proof = SignatureProof::EdDSA(EdDSASignatureProof::from(
        in_key_pair.public,
        in_key_pair.sign(&tx.serialize_content()),
    ));

    tx.recipient_data =
        IncomingStakingTransactionData::set_signature_on_data(&tx.recipient_data, in_proof)
            .unwrap();

    let out_private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(NON_EXISTENT_PRIVATE_KEY).unwrap()).unwrap();

    let out_key_pair = KeyPair::from(out_private_key);

    let out_proof = SignatureProof::EdDSA(EdDSASignatureProof::from(
        out_key_pair.public,
        out_key_pair.sign(&tx.serialize_content()),
    ))
    .serialize_to_vec();

    tx.proof = out_proof;

    tx
}

fn make_delete_validator_transaction() -> Transaction {
    let mut tx = Transaction::new_extended(
        Policy::STAKING_CONTRACT_ADDRESS,
        AccountType::Staking,
        OutgoingStakingTransactionData::DeleteValidator.serialize_to_vec(),
        non_existent_address(),
        AccountType::Basic,
        vec![],
        (Policy::VALIDATOR_DEPOSIT - 100).try_into().unwrap(),
        100.try_into().unwrap(),
        1,
        NetworkId::Dummy,
    );

    let private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(VALIDATOR_PRIVATE_KEY).unwrap()).unwrap();

    let key_pair = KeyPair::from(private_key);
    let signature = key_pair.sign(&tx.serialize_content());
    tx.proof = SignatureProof::EdDSA(EdDSASignatureProof::from(key_pair.public, signature))
        .serialize_to_vec();

    tx
}

fn make_sample_contract(
    mut data_store: DataStoreWrite,
    staker_active_balance: Option<u64>,
) -> (Address, Option<Address>, StakingContract) {
    let validator_address = validator_address();
    let staker_address = staker_address();

    let signing_key =
        EdDSAPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let mut contract = StakingContract::default();

    let mut store = StakingContractStoreWrite::new(&mut data_store);

    contract
        .create_validator(
            &mut store,
            &validator_address,
            signing_key,
            voting_key,
            validator_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            None,
            None,
            false,
            &mut TransactionLog::empty(),
        )
        .unwrap();

    if let Some(staker_active_balance) = staker_active_balance {
        contract
            .create_staker(
                &mut store,
                &staker_address,
                Coin::from_u64_unchecked(staker_active_balance),
                Some(validator_address.clone()),
                Coin::ZERO,
                None,
                &mut TransactionLog::empty(),
            )
            .unwrap();
    }
    (validator_address, Some(staker_address), contract)
}

#[allow(dead_code)]
enum ValidatorState {
    Active,
    Jailed,
    Retired,
    Deleted,
}

struct ValidatorSetup {
    env: DatabaseProxy,
    accounts: Accounts,
    staking_contract: StakingContract,
    effective_state_block_state: BlockState,
    before_state_release_block_state: BlockState,
    state_release_block_state: BlockState,
    validator_address: Address,
    staker_address: Option<Address>,
}

impl ValidatorSetup {
    fn setup(
        state_block_height: BlockState,
        before_state_release_block_state: BlockState,
        state_release_block_state: BlockState,
        staker_active_balance: Option<u64>,
    ) -> ValidatorSetup {
        let env = VolatileDatabase::new(20).unwrap();
        let accounts = Accounts::new(env.clone());
        let data_store = accounts.data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut db_txn_og = env.write_transaction();
        let mut db_txn = (&mut db_txn_og).into();

        let (validator_address, staker_address, staking_contract) =
            make_sample_contract(data_store.write(&mut db_txn), staker_active_balance);

        db_txn_og.commit();

        ValidatorSetup {
            env,
            accounts,
            staking_contract,
            effective_state_block_state: state_block_height,
            before_state_release_block_state,
            state_release_block_state,
            validator_address,
            staker_address,
        }
    }

    fn new(staker_active_balance: Option<u64>) -> ValidatorSetup {
        Self::setup(
            BlockState::default(),
            BlockState::default(),
            BlockState::default(),
            staker_active_balance,
        )
    }

    fn setup_retired_validator(staker_active_balance: Option<u64>) -> Self {
        let mut validator_setup = ValidatorSetup::new(staker_active_balance);
        let data_store = validator_setup
            .accounts
            .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut db_txn_og = validator_setup.env.write_transaction();
        let mut db_txn = (&mut db_txn_og).into();

        // Retire the validator.
        let retire_tx = make_signed_incoming_transaction(
            IncomingStakingTransactionData::RetireValidator {
                proof: SignatureProof::default(),
            },
            0,
            &ed25519_key_pair(VALIDATOR_PRIVATE_KEY),
        );

        let block_state = BlockState::new(Policy::genesis_block_number() + 3, 3);
        validator_setup
            .staking_contract
            .commit_incoming_transaction(
                &retire_tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty(),
            )
            .expect("Failed to commit transaction");

        let effective_state_block_state =
            BlockState::new(Policy::election_block_after(block_state.number), 2);
        let after_cooldown =
            Policy::block_after_reporting_window(effective_state_block_state.number);
        let after_cooldown = BlockState::new(after_cooldown, 1000);
        let before_cooldown = BlockState::new(after_cooldown.number - 1, 9000);

        db_txn_og.commit();

        validator_setup.set_block_state(
            effective_state_block_state,
            before_cooldown,
            after_cooldown,
        );
        validator_setup
    }

    fn setup_deleted_validator(staker_active_balance: Option<u64>) -> Self {
        let mut validator_setup = Self::setup_retired_validator(staker_active_balance);
        let data_store = validator_setup
            .accounts
            .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut db_txn_og = validator_setup.env.write_transaction();
        let mut db_txn = (&mut db_txn_og).into();

        // Delete the validator.
        let delete_tx = make_delete_validator_transaction();

        let block_state = BlockState::new(validator_setup.state_release_block_state.number, 4);
        validator_setup
            .staking_contract
            .commit_outgoing_transaction(
                &delete_tx,
                &block_state,
                data_store.write(&mut db_txn),
                &mut TransactionLog::empty(),
            )
            .expect("Failed to commit transaction");

        db_txn_og.commit();

        validator_setup.set_block_state(block_state, BlockState::default(), BlockState::default());
        validator_setup
    }

    fn setup_jailed_validator(staker_active_balance: Option<u64>) -> ValidatorSetup {
        let jailing_inherent_block_state = BlockState::new(Policy::genesis_block_number() + 2, 2);

        let mut validator_setup = ValidatorSetup::new(staker_active_balance);
        let data_store = validator_setup
            .accounts
            .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut db_txn_og = validator_setup.env.write_transaction();
        let mut db_txn = (&mut db_txn_og).into();

        // Jail validator
        let mut data_store_write = data_store.write(&mut db_txn);
        let mut staking_contract_store = StakingContractStoreWrite::new(&mut data_store_write);
        let _result = validator_setup
            .staking_contract
            .jail_validator(
                &mut staking_contract_store,
                &validator_setup.validator_address,
                jailing_inherent_block_state.number,
                &mut TransactionLog::empty(),
            )
            .unwrap();

        db_txn_og.commit();

        let effective_state_block_state = BlockState::new(jailing_inherent_block_state.number, 2);
        let jail_release_block_state = BlockState::new(
            Policy::block_after_jail(effective_state_block_state.number),
            2,
        );
        let before_release_block_state = BlockState::new(jail_release_block_state.number - 1, 2);

        validator_setup.set_block_state(
            effective_state_block_state,
            before_release_block_state,
            jail_release_block_state,
        );
        validator_setup
    }

    fn set_block_state(
        &mut self,
        state_block_height: BlockState,
        before_state_release_block_state: BlockState,
        state_release_block_state: BlockState,
    ) {
        self.effective_state_block_state = state_block_height;
        self.before_state_release_block_state = before_state_release_block_state;
        self.state_release_block_state = state_release_block_state;
    }
}

struct StakerSetup {
    env: DatabaseProxy,
    accounts: Accounts,
    staking_contract: StakingContract,
    effective_block_state: BlockState,
    before_release_block_state: BlockState,
    release_block_state: BlockState,
    retire_stake_block_state: BlockState,
    validator_address: Address,
    staker_address: Address,
    active_stake: Coin,
    inactive_stake: Coin,
    retired_stake: Coin,
    validator_state_release: Option<u32>,
}

impl StakerSetup {
    fn setup_staker_with_inactive_retired_balance(
        validator_state: ValidatorState,
        active_stake: u64,
        inactive_stake: u64,
        retired_stake: u64,
    ) -> Self {
        // Setup jailed validator
        let mut validator_state_release = None;
        let mut validator_setup = match validator_state {
            ValidatorState::Active => {
                ValidatorSetup::new(Some(active_stake + inactive_stake + retired_stake))
            }
            ValidatorState::Jailed => {
                let validator_setup = ValidatorSetup::setup_jailed_validator(Some(
                    active_stake + inactive_stake + retired_stake,
                ));
                validator_state_release = Some(validator_setup.state_release_block_state.number);
                validator_setup
            }
            ValidatorState::Retired => {
                let validator_setup = ValidatorSetup::setup_retired_validator(Some(
                    active_stake + inactive_stake + retired_stake,
                ));
                validator_state_release = Some(validator_setup.state_release_block_state.number);
                validator_setup
            }
            ValidatorState::Deleted => ValidatorSetup::setup_deleted_validator(Some(
                active_stake + inactive_stake + retired_stake,
            )),
        };

        let data_store = validator_setup
            .accounts
            .data_store(&Policy::STAKING_CONTRACT_ADDRESS);
        let mut db_txn_og = validator_setup.env.write_transaction();
        let mut db_txn = (&mut db_txn_og).into();
        let mut data_store_write = data_store.write(&mut db_txn);
        let mut staking_contract_store = StakingContractStoreWrite::new(&mut data_store_write);

        let active_stake = Coin::from_u64_unchecked(active_stake);
        let inactive_stake = Coin::from_u64_unchecked(inactive_stake);
        let retired_stake = Coin::from_u64_unchecked(retired_stake);
        let staker_address = validator_setup.staker_address.unwrap();
        let deactivation_block = Policy::genesis_block_number() + 2;

        let effective_block_state =
            BlockState::new(Policy::election_block_after(deactivation_block), 2);
        let release_block_state = BlockState::new(
            Policy::block_after_reporting_window(effective_block_state.number),
            2,
        );
        let before_release_block_state = BlockState::new(release_block_state.number - 1, 2);
        let mut retire_stake_block_state = BlockState::default();

        // Deactivate part of the stake.
        validator_setup
            .staking_contract
            .set_active_stake(
                &mut staking_contract_store,
                &staker_address,
                active_stake,
                deactivation_block,
                &mut TransactionLog::empty(),
            )
            .expect("Failed to set inactive stake");

        if !retired_stake.is_zero() {
            let retire_block = if matches!(validator_state, ValidatorState::Jailed) {
                max(validator_state_release.unwrap(), release_block_state.number)
            } else {
                release_block_state.number
            };
            retire_stake_block_state = BlockState::new(retire_block, 3);

            // Retire part of the stake.
            validator_setup
                .staking_contract
                .retire_stake(
                    &mut staking_contract_store,
                    &staker_address,
                    retired_stake,
                    retire_block,
                    &mut TransactionLog::empty(),
                )
                .expect("Failed to set inactive stake");
        }

        db_txn_og.commit();

        StakerSetup {
            env: validator_setup.env,
            accounts: validator_setup.accounts,
            staking_contract: validator_setup.staking_contract,
            effective_block_state,
            before_release_block_state,
            release_block_state,
            validator_address: validator_setup.validator_address,
            staker_address,
            active_stake,
            inactive_stake,
            retired_stake,
            validator_state_release,
            retire_stake_block_state,
        }
    }
}
