use nimiq_account::{DataStoreWrite, StakingContract, StakingContractStoreWrite, TransactionLog};
use nimiq_bls::{
    CompressedPublicKey as BlsPublicKey, KeyPair as BlsKeyPair, SecretKey as BlsSecretKey,
};
use nimiq_keys::{Address, KeyPair, PrivateKey, PublicKey};
use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionProof},
    SignatureProof, Transaction,
};

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

fn ed25519_public_key(pk: &str) -> PublicKey {
    PublicKey::deserialize_from_vec(&hex::decode(pk).unwrap()).unwrap()
}

fn make_sample_contract(mut data_store: DataStoreWrite, with_staker: bool) -> StakingContract {
    let staker_address = staker_address();

    let cold_address = validator_address();

    let signing_key =
        PublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_SIGNING_KEY).unwrap()).unwrap();

    let voting_key =
        BlsPublicKey::deserialize_from_vec(&hex::decode(VALIDATOR_VOTING_KEY).unwrap()).unwrap();

    let mut contract = StakingContract::default();

    let mut store = StakingContractStoreWrite::new(&mut data_store);

    contract
        .create_validator(
            &mut store,
            &cold_address,
            signing_key,
            voting_key,
            cold_address.clone(),
            None,
            Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
            &mut TransactionLog::empty(),
        )
        .unwrap();

    if with_staker {
        contract
            .create_staker(
                &mut store,
                &staker_address,
                Coin::from_u64_unchecked(150_000_000),
                Some(cold_address),
                &mut TransactionLog::empty(),
            )
            .unwrap();
    }

    contract
}

fn make_incoming_transaction(data: IncomingStakingTransactionData, value: u64) -> Transaction {
    match data {
        IncomingStakingTransactionData::CreateValidator { .. }
        | IncomingStakingTransactionData::CreateStaker { .. }
        | IncomingStakingTransactionData::AddStake { .. } => Transaction::new_extended(
            non_existent_address(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            value.try_into().unwrap(),
            100.try_into().unwrap(),
            data.serialize_to_vec(),
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

    let in_proof = SignatureProof::from(
        in_key_pair.public,
        in_key_pair.sign(&tx.serialize_content()),
    );

    tx.data = IncomingStakingTransactionData::set_signature_on_data(&tx.data, in_proof).unwrap();

    let out_private_key =
        PrivateKey::deserialize_from_vec(&hex::decode(NON_EXISTENT_PRIVATE_KEY).unwrap()).unwrap();

    let out_key_pair = KeyPair::from(out_private_key);

    let out_proof = SignatureProof::from(
        out_key_pair.public,
        out_key_pair.sign(&tx.serialize_content()),
    )
    .serialize_to_vec();

    tx.proof = out_proof;

    tx
}
