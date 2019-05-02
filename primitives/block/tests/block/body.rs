use hex;

use account::{Account, AccountType, PrunedAccount, VestingContract, Receipt, ReceiptType};
use beserial::{Deserialize, Serialize};
use nimiq_block::{BlockBody, BlockError};
use hash::{Blake2bHash, Hash};
use keys::Address;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use transaction::{TransactionError, TransactionFlags, TransactionFormat};

const GENESIS_BODY: &str = "0000000000000000000000000000000000000000836c6f766520616920616d6f72206d6f68616262617420687562756e2063696e7461206c7975626f76206268616c616261736120616d6f7572206b61756e6120706927617261206c696562652065736871207570656e646f207072656d6120616d6f7265206b61747265736e616e20736172616e6720616e7075207072656d612079657500000000";
const B169500_BODY: &str = "b403040ff02b4a9a4d10f8a4beff71750cd3aa7f324e696d6275732d393630656364346238313936303030303030323266386531313034663066663830323530373939373632320003010000ad8e224835e6cc0cadbcf500a49dae46f67697040224786862babbdb05e7c4430612135eb2a836812300000000000000000100000000000000000002961b2a0000a4010301daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f94dc65fbd91fc9f5ed1d92dba86d2e59607774b13e27a556d92a56ade9808d32cebdb4e704995dd63f97429a827541d3c6253bc0c2f472c3341b538e2f4bc60800e930b6408b84cb3a7985c54e742c4a1af3110b865386144a0e1badd0bceefc162cecfda0761ce11e06a076c2882b2e21309602973a4443ba4d672b665d4f3f0100bf703dd4eb71e245cced26f014f40dc29f0f71e6bffce722a7ceecba1b71cb8d44d5c0727a1a08efd376cd005293e33236232fad0000000000004e2000000000000001180002961a2a8f6693b1eb1537aa8eec3f99898d91474e5cfa1de1c81a1ee9e0c04ba0205e310d65708ea6e5ed9e7953f4dce64e73392333a2b83d1b30d0df9eb74bcc68980900725323e8f226e7b9dac90331505844f83e45f28cc6f0534039f55c725536d1ec6573413bec835fa4fb54f4dfc3b273a9e8ecc95b000000000011c7020000000000000000000296192a53ddbfd2dd72a78a8b62699e0012585f4768f07e397ae3bb23d0cd379c0c12c9dbe0aca1ae8f005330bbb33c80ee1449229ad6d0e482a1fc8124233dd953600e000100ad8e224835e6cc0cadbcf500a49dae46f67697040200000000000000001b215589344cf570d36bec770825eae30b73213924786862babbdb05e7c4430612135eb2a836812303daebe368963c60d22098a5e9f1ebcb8e54d0b7beca942a2a0a9d95391804fe8f01000296350000000000000001";
const B67795_BODY: &str = "6c4e6bc79391038a786ae11c0a8175c4a29c628125426565702d33627a4b4e587935613972797937793646374d7142726f70716e3730326a526b000200bf703dd4eb71e245cced26f014f40dc29f0f71e6bffce722a7ceecba1b71cb8d8a98acef6877ce586313cf7d46410f195c43fb5e00000000000061a80000000000000096000108d22ae2b8fe84104a06dd7c07987f08c77320704152a17f6c3d2a036fe20929794f5734f37fc9513335cea0844a47558748aae6603e1e4e3f3cc39480bcd4051e080c010000f3e6c569c694030b329b03b06ebbb8bb27586e1702bab04d1e40cc8e99fcc3caacc23e6ddd0a340d150000000000000000010000000000000000000108d32a0000a40103011fe43246c44f12e0d30d9bc3cf8de1bcf5740127e67caeea1459a9422802e42ac22aff463016308427f10d89913917eb9b39b2b422cfa51baa188fb33a61ffa4ca8ac9412bbc05153b1faa9fae61a31d6b6ce09cc9a04c671a31668189dd28d0004b4e09a27985ffe0de96e75dee210658990aed0a269ec8b256b8274d7d8ea4d290f25b7c95e88759c54a7e92a9c53ceba5417e434c79bf5f69e489f58da2420e000100f3e6c569c694030b329b03b06ebbb8bb27586e170200000000000000007d4731244bf358a96a2529488fe40135174cb8e5bab04d1e40cc8e99fcc3caacc23e6ddd0a340d15031fe43246c44f12e0d30d9bc3cf8de1bcf5740127e67caeea1459a9422802e42a01000108e80000000000000001";

#[test]
fn it_can_deserialize_genesis_body() {
    let v: Vec<u8> = hex::decode(GENESIS_BODY).unwrap();
    let body: BlockBody = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(body.miner, Address::from(&hex::decode("0000000000000000000000000000000000000000").unwrap()[..]));
    assert_eq!(body.extra_data, "love ai amor mohabbat hubun cinta lyubov bhalabasa amour kauna pi'ara liebe eshq upendo prema amore katresnan sarang anpu prema yeu".as_bytes().to_vec());
    assert_eq!(body.transactions.len(), 0);
    assert_eq!(body.account_receipts.len(), 0);
}

#[test]
fn it_can_serialize_genesis_body() {
    let v: Vec<u8> = hex::decode(GENESIS_BODY).unwrap();
    let body: BlockBody = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(body.serialized_size());
    let size = body.serialize(&mut v2).unwrap();
    assert_eq!(size, body.serialized_size());
    assert_eq!(hex::encode(v2), GENESIS_BODY);
}

#[test]
fn it_can_calculate_genesis_body_hash() {
    let body = BlockBody::deserialize_from_vec(&hex::decode(GENESIS_BODY).unwrap()).unwrap();
    assert_eq!(Hash::hash::<Blake2bHash>(&body), Blake2bHash::from("7cda9a7fdf06655905ae5dbd9c535451471b078fa6f3df0e287e5b0fb47a573a"));
}

#[test]
fn it_can_deserialize_b169500_body() {
    let v: Vec<u8> = hex::decode(B169500_BODY).unwrap();
    let body: BlockBody = Deserialize::deserialize(&mut &v[..]).unwrap();
    assert_eq!(body.miner, Address::from(&hex::decode("b403040ff02b4a9a4d10f8a4beff71750cd3aa7f").unwrap()[..]));
    assert_eq!(body.extra_data, "Nimbus-960ecd4b819600000022f8e1104f0ff802507997622".as_bytes().to_vec());
    assert_eq!(body.transactions.len(), 3);
    assert_eq!(body.account_receipts.len(), 1);

    assert_eq!(body.transactions[0].format(), TransactionFormat::Extended);
    assert_eq!(body.transactions[0].sender, Address::from("ad8e224835e6cc0cadbcf500a49dae46f6769704"));
    assert_eq!(body.transactions[0].sender_type, AccountType::HTLC);
    assert_eq!(body.transactions[0].recipient, Address::from("24786862babbdb05e7c4430612135eb2a8368123"));
    assert_eq!(body.transactions[0].recipient_type, AccountType::Basic);
    assert_eq!(body.transactions[0].value, Coin::from_u64(1).unwrap());
    assert_eq!(body.transactions[0].fee, Coin::ZERO);
    assert_eq!(body.transactions[0].validity_start_height, 169499);
    assert_eq!(body.transactions[0].flags, TransactionFlags::empty());
    assert_eq!(body.transactions[0].data, vec![]);

    assert_eq!(body.transactions[1].format(), TransactionFormat::Basic);
    assert_eq!(body.transactions[1].sender, Address::from("86bd79da00997fd8be41bfd58f95a1f6f01d51f7"));
    assert_eq!(body.transactions[1].recipient, Address::from("44d5c0727a1a08efd376cd005293e33236232fad"));
    assert_eq!(body.transactions[1].value, Coin::from_u64(20000).unwrap());
    assert_eq!(body.transactions[1].fee, Coin::from_u64(280).unwrap());
    assert_eq!(body.transactions[1].validity_start_height, 169498);

    assert_eq!(body.transactions[2].format(), TransactionFormat::Basic);
    assert_eq!(body.transactions[2].sender, Address::from("d0fc18c65972d86d207a818dc4974019c3cc65c8"));
    assert_eq!(body.transactions[2].recipient, Address::from("6573413bec835fa4fb54f4dfc3b273a9e8ecc95b"));
    assert_eq!(body.transactions[2].value, Coin::from_u64(1165058).unwrap());
    assert_eq!(body.transactions[2].fee, Coin::ZERO);
    assert_eq!(body.transactions[2].validity_start_height, 169497);

    assert_eq!(body.account_receipts[0].receipt_type(), ReceiptType::PrunedAccount);
    let receipt = match &body.account_receipts[0] {
        Receipt::PrunedAccount(pruned_account) => pruned_account,
    };
    assert_eq!(receipt.address, Address::from("ad8e224835e6cc0cadbcf500a49dae46f6769704"));
    assert_eq!(receipt.account.balance(), Coin::ZERO);
    assert_eq!(receipt.account.account_type(), AccountType::HTLC);
}

#[test]
fn it_can_serialize_b169500_body() {
    let v: Vec<u8> = hex::decode(B169500_BODY).unwrap();
    let body: BlockBody = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(body.serialized_size());
    let size = body.serialize(&mut v2).unwrap();
    assert_eq!(size, body.serialized_size());
    assert_eq!(hex::encode(v2), B169500_BODY);
}

#[test]
fn it_can_serialize_b67795_body() {
    let v: Vec<u8> = hex::decode(B67795_BODY).unwrap();
    let body: BlockBody = Deserialize::deserialize(&mut &v[..]).unwrap();
    let mut v2: Vec<u8> = Vec::with_capacity(body.serialized_size());
    let size = body.serialize(&mut v2).unwrap();
    assert_eq!(size, body.serialized_size());
    assert_eq!(hex::encode(v2), B67795_BODY);
}

#[test]
fn it_can_calculate_b169500_body_hash() {
    let body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    assert_eq!(Hash::hash::<Blake2bHash>(&body), Blake2bHash::from("a65795b408a23693203cb7bbae3206fafcf0f49a8e2832b51df27e904d1f397d"));
}

#[test]
fn verify_accepts_an_empty_body() {
    let body = BlockBody {
        miner: Address::from([1u8; Address::SIZE]),
        extra_data: Vec::new(),
        transactions: Vec::new(),
        account_receipts: Vec::new()
    };
    assert!(body.verify(1, NetworkId::Main).is_ok());
}

#[test]
fn verify_accepts_body_169500() {
    let body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    assert!(body.verify(169500, NetworkId::Main).is_ok());
}

#[test]
fn verify_accepts_body_67795() {
    let body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B67795_BODY).unwrap()).unwrap();
    assert!(body.verify(67795, NetworkId::Main).is_ok());
}

#[test]
fn verify_rejects_duplicate_transactions() {
    let mut body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    let tx = body.transactions[2].clone();
    body.transactions.push(tx);
    assert_eq!(body.verify(169500, NetworkId::Main), Err(BlockError::DuplicateTransaction));
}

#[test]
fn verify_rejects_unordered_transactions() {
    let mut body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    let tx = body.transactions[2].clone();
    body.transactions.insert(0, tx);
    body.transactions.remove(2);
    assert_eq!(body.verify(169500, NetworkId::Main), Err(BlockError::TransactionsNotOrdered));
}

#[test]
fn verify_rejects_invalid_transactions() {
    let mut body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    body.transactions[1].proof[0] += 1;
    assert_eq!(body.verify(169500, NetworkId::Main), Err(BlockError::InvalidTransaction(TransactionError::InvalidProof)));
}

#[test]
fn verify_rejects_expired_transactions() {
    let body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    assert_eq!(body.verify(169400, NetworkId::Main), Err(BlockError::ExpiredTransaction));
    assert_eq!(body.verify(169620, NetworkId::Main), Err(BlockError::ExpiredTransaction));
}

#[test]
fn verify_rejects_duplicate_pruned_accounts() {
    let mut body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    let account = body.account_receipts[0].clone();
    body.account_receipts.push(account);
    assert_eq!(body.verify(169500, NetworkId::Main), Err(BlockError::DuplicateAccountReceipt));
}

#[test]
fn verify_rejects_unordered_pruned_accounts() {
    let mut body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    let mut account = body.account_receipts[0].clone();
    match account {
        Receipt::PrunedAccount(ref mut account) => account.address = Address::from([1u8; Address::SIZE]),
    }
    body.account_receipts.push(account);
    assert_eq!(body.verify(169500, NetworkId::Main), Err(BlockError::AccountReceiptsNotOrdered));
}

#[test]
fn verify_rejects_invalid_pruned_accounts() {
    let mut body: BlockBody = BlockBody::deserialize_from_vec(&hex::decode(B169500_BODY).unwrap()).unwrap();
    let pruned_account = PrunedAccount {
        address: Address::from([255u8; Address::SIZE]),
        account: Account::Vesting(VestingContract {
            balance: Coin::from_u64(1000).unwrap(),
            owner: Address::from([1u8; Address::SIZE]),
            vesting_start: 1,
            vesting_step_blocks: 1,
            vesting_step_amount: Coin::ZERO,
            vesting_total_amount: Coin::from_u64(1000).unwrap()
        })
    };
    body.account_receipts.push(Receipt::PrunedAccount(pruned_account));
    assert_eq!(body.verify(169500, NetworkId::Main), Err(BlockError::InvalidAccountReceipt));
}
