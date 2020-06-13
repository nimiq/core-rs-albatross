use std::convert::{TryFrom, TryInto};
use std::sync::Arc;

use json::{JsonValue, Null, object};
use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use blockchain_albatross::Blockchain;
use bls::bls12_381::{CompressedPublicKey, CompressedSignature};
use consensus::AlbatrossConsensusProtocol;
use keys::Address;
use network_primitives::networks::NetworkInfo;
use nimiq_mempool::Mempool;
use primitives::account::AccountType;
use primitives::coin::Coin;
use transaction::account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionProof, SelfStakingTransactionData};
use transaction::Transaction;
use validator::validator::Validator;
use nimiq_transaction_builder::{Recipient, TransactionBuilder, recipient::staking_contract::StakingRecipientBuilder};

use crate::handler::Method;
use crate::handlers::mempool::MempoolHandler;
use crate::handlers::Module;
use crate::handlers::wallet::UnlockedWalletManager;

pub struct MempoolAlbatrossHandler {
    pub mempool: Arc<Mempool<Blockchain>>,
    pub validator: Option<Arc<Validator>>,
    pub unlocked_wallets: Option<Arc<RwLock<UnlockedWalletManager>>>,
    generic: MempoolHandler<AlbatrossConsensusProtocol>,
}

impl MempoolAlbatrossHandler {
    pub fn new(
        mempool: Arc<Mempool<Blockchain>>,
        validator: Option<Arc<Validator>>,
        unlocked_wallets: Option<Arc<RwLock<UnlockedWalletManager>>>,
    ) -> Self {
        Self {
            mempool: Arc::clone(&mempool),
            validator,
            unlocked_wallets: unlocked_wallets.as_ref().map(Arc::clone),
            generic: MempoolHandler::new(mempool, unlocked_wallets),
        }
    }

    fn parse_address(value: &JsonValue, kind: &str) -> Result<Address, JsonValue> {
        JsonValue::as_str(value)
            .ok_or_else(|| object! {"message" => format!("Invalid {} address", kind)})
            .and_then(|it| Address::from_user_friendly_address(it)
                .map_err(|_| object! {"message" => format!("Invalid {} address", kind)}))
    }

    /// Create validator
    /// Parameters:
    /// - sender_address: NIM address used to create this transaction
    /// - validator_key: Public key of validator (BLS)
    /// - proof_of_knowledge: Proof of knowledge of validator key
    /// - reward_address: NIM address used for the reward
    /// - amount: Initial staking amount in Luna
    /// - fee: Fee for transaction in Luna
    pub(crate) fn create_validator(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let sender_address = Self::parse_address(params.get(0).unwrap_or(&Null), "sender")?;
        let validator_key = params.get(1)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| object! {"message" => "Invalid validator key"})
            .and_then(|it| hex::decode(it)
                .map_err(|_| object! {"message" => "Validator key must be hex-encoded"}))
            .and_then(|it| CompressedPublicKey::deserialize_from_vec(&it)
                .map_err(|_| object! {"message" => "Invalid public key"}))?;
        let proof_of_knowledge = params.get(2)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| object! {"message" => "Invalid proof of knowledge"})
            .and_then(|it| hex::decode(it)
                .map_err(|_| object! {"message" => "Proof of knowledge must be hex-encoded"}))
            .and_then(|it| CompressedSignature::deserialize_from_vec(&it)
                .map_err(|_| object! {"message" => "Invalid proof of knowledge"}))?;
        let reward_address = Self::parse_address(params.get(3).unwrap_or(&Null), "reward")?;
        let amount = params.get(4)
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| object! {"message" => "Invalid amount"})
            .and_then(|it| Coin::try_from(it)
                .map_err(|e| object! {"message" => format!("Invalid amount: {}", e)}))?;
        let fee = params.get(5)
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|e| object! {"message" => format!("Invalid fee: {}", e)})?;

        let network_id = self.mempool.network_id();
        let staking_contract = NetworkInfo::from_network_id(network_id)
            .validator_registry_address().unwrap();
        let staking_data = IncomingStakingTransactionData::CreateValidator {
            validator_key,
            reward_address,
            proof_of_knowledge,
        };

        let mut tx = Transaction::new_extended(
            sender_address, AccountType::Basic,    // sender
            staking_contract.clone(), AccountType::Staking, // recipient
            amount, fee,    // amount, fee
            staking_data.serialize_to_vec(),       // data
            self.mempool.current_height(),         // validity_start_height
            network_id,                            // network_id
        );

        debug!("Transaction data: {:#?}", staking_data);

        let unlocked_wallets = self.unlocked_wallets.as_ref()
            .ok_or_else(|| object! {"message" => "No wallets"})?;
        let unlocked_wallets = unlocked_wallets.read();
        let wallet_account = unlocked_wallets.get(&tx.sender)
            .ok_or_else(|| object! {"message" => "Sender account is locked"})?;
        wallet_account.sign_transaction(&mut tx);

        self.generic.push_transaction(tx)
    }

    /// Retire validator
    /// Parameters:
    /// - sender: NIM address used to create this transaction
    /// - fee: Fee for transaction in Luna
    pub(crate) fn retire_validator(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        // Make sure a validator object is available
        let validator = self.validator.as_ref().ok_or_else(|| object! {"message" => "No validator configured"})?;

        let sender = Self::parse_address(params.get(0).unwrap_or(&Null), "sender")?;
        let fee = params.get(1)
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|e| object! {"message" => format!("Invalid fee: {}", e)})?;

        let staking_contract = NetworkInfo::from_network_id(self.mempool.network_id())
            .validator_registry_address().unwrap();

        let mut recipient = Recipient::new_staking_builder(staking_contract.clone());
        recipient.retire_validator(&validator.validator_key.public);

        let tx = self.build_validator_signalling_transaction(sender, recipient, fee)?;
        self.generic.push_transaction(tx)
    }

    /// Reactivate validator
    /// Parameters:
    /// - sender: NIM address used to create this transaction
    /// - [fee]: Fee for transaction in Luna
    pub(crate) fn reactivate_validator(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        // Make sure a validator object is available
        let validator = self.validator.as_ref().ok_or_else(|| object! {"message" => "No validator configured"})?;

        let sender = Self::parse_address(params.get(0).unwrap_or(&Null), "sender")?;
        let fee = params.get(1)
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|e| object! {"message" => format!("Invalid fee: {}", e)})?;

        let staking_contract = NetworkInfo::from_network_id(self.mempool.network_id())
            .validator_registry_address().unwrap();

        let mut recipient = Recipient::new_staking_builder(staking_contract.clone());
        recipient.reactivate_validator(&validator.validator_key.public);

        let tx = self.build_validator_signalling_transaction(sender, recipient, fee)?;
        self.generic.push_transaction(tx)
    }

    /// Unpark validator
    /// Parameters:
    /// - sender: NIM address used to create this transaction
    /// - fee: Fee for transaction in Luna
    pub(crate) fn unpark_validator(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        // Make sure a validator object is available
        let validator = self.validator.as_ref().ok_or_else(|| object! {"message" => "No validator configured"})?;

        let sender = Self::parse_address(params.get(0).unwrap_or(&Null), "sender")?;
        let fee = params.get(1)
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|e| object! {"message" => format!("Invalid fee: {}", e)})?;

        let staking_contract = NetworkInfo::from_network_id(self.mempool.network_id())
            .validator_registry_address().unwrap();

        let mut recipient = Recipient::new_staking_builder(staking_contract.clone());
        recipient.unpark_validator(&validator.validator_key.public);

        let tx = self.build_validator_signalling_transaction(sender, recipient, fee)?;
        self.generic.push_transaction(tx)
    }

    fn build_validator_signalling_transaction(&self,
        sender: Address,
        recipient: StakingRecipientBuilder,
        fee: Coin
    ) -> Result<Transaction, JsonValue> {
        let unlocked_wallets = self.unlocked_wallets.as_ref()
            .ok_or_else(|| object! {"message" => "No wallets"})?;
        let unlocked_wallets = unlocked_wallets.read();
        let wallet_account = unlocked_wallets.get(&sender)
            .ok_or_else(|| object! {"message" => "Sender account is locked"})?;

        let mut tx_builder = TransactionBuilder::new();
        tx_builder
            .with_sender(sender)
            .with_value(Coin::ZERO)
            .with_fee(fee)
            .with_network_id(self.mempool.network_id())
            .with_validity_start_height(self.mempool.current_height())
            .with_recipient(recipient.generate().unwrap());

        let mut proof_builder = tx_builder.generate().unwrap().unwrap_signalling();
        proof_builder.sign_with_validator_key_pair(&self.validator.as_ref().unwrap().validator_key);
        let mut proof_builder = proof_builder.generate().unwrap().unwrap_basic();
        proof_builder.sign_with_key_pair(&wallet_account.key_pair);
        proof_builder.generate().ok_or_else(|| object! {"message" => "Failed to construct transaction"})
    }

    /// Stakes NIM
    /// Parameters:
    /// - sender_address: NIM address used to deduct funds from
    /// - validator_key: Public key of validator (BLS)
    /// - amount: Amount in Luna to stake
    /// - fee: Fee for transaction in Luna
    /// - staker_address: NIM address used to stake (optional, default is sender)
    pub(crate) fn stake(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let sender_address = Self::parse_address(params.get(0).unwrap_or(&Null), "sender")?;
        let validator_key = params.get(1)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| object! {"message" => "Invalid validator key"})
            .and_then(|it| hex::decode(it)
                .map_err(|_| object! {"message" => "Validator key must be hex-encoded"}))
            .and_then(|it| CompressedPublicKey::deserialize_from_vec(&it)
                .map_err(|_| object! {"message" => "Invalid public key"}))?;
        let amount = params.get(2)
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| object! {"message" => "Invalid amount"})
            .and_then(|it| Coin::try_from(it)
                .map_err(|e| object! {"message" => format!("Invalid amount: {}", e)}))?;
        let fee = params.get(3)
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|e| object! {"message" => format!("Invalid fee: {}", e)})?;
        let staker_address = if let Some(value) = params.get(4) {
            Some(Self::parse_address(value, "staker")?)
        } else { None };

        let network_id = self.mempool.network_id();
        let staking_contract = NetworkInfo::from_network_id(network_id)
            .validator_registry_address().unwrap();
        let staking_data = IncomingStakingTransactionData::Stake {
            validator_key,
            staker_address,
        };

        let mut tx = Transaction::new_extended(
            sender_address, AccountType::Basic,    // sender
            staking_contract.clone(), AccountType::Staking, // recipient
            amount, fee,    // amount, fee
            staking_data.serialize_to_vec(),       // data
            self.mempool.current_height(),         // validity_start_height
            network_id,                            // network_id
        );

        debug!("Transaction data: {:#?}", staking_data);

        let unlocked_wallets = self.unlocked_wallets.as_ref()
            .ok_or_else(|| object! {"message" => "No wallets"})?;
        let unlocked_wallets = unlocked_wallets.read();
        let wallet_account = unlocked_wallets.get(&tx.sender)
            .ok_or_else(|| object! {"message" => "Sender account is locked"})?;
        wallet_account.sign_transaction(&mut tx);

        self.generic.push_transaction(tx)
    }

    /// Retires staked NIM
    /// Parameters:
    /// - validator_key: Public key of validator (BLS)
    /// - staker_address: NIM address used to stake
    /// - amount: Amount to retire
    /// - fee: Fee for transaction in Luna
    pub(crate) fn retire(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let validator_key = params.get(0)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| object! {"message" => "Invalid validator key"})
            .and_then(|it| hex::decode(it)
                .map_err(|_| object! {"message" => "Validator key must be hex-encoded"}))
            .and_then(|it| CompressedPublicKey::deserialize_from_vec(&it)
                .map_err(|_| object! {"message" => "Invalid public key"}))?;
        let staker_address = Self::parse_address(params.get(1).unwrap_or(&Null), "staker")?;
        let amount = params.get(2)
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| object! {"message" => "Invalid amount"})
            .and_then(|it| Coin::try_from(it)
                .map_err(|e| object! {"message" => format!("Invalid amount: {}", e)}))?;
        let fee = params.get(3)
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|e| object! {"message" => format!("Invalid fee: {}", e)})?;

        let network_id = self.mempool.network_id();
        let genesis_account = NetworkInfo::from_network_id(network_id)
            .validator_registry_address().unwrap();

        let staking_data = SelfStakingTransactionData::RetireStake(validator_key);

        let mut tx = Transaction::new_extended(
            genesis_account.clone(), AccountType::Staking,
            genesis_account.clone(), AccountType::Staking,
            amount, fee, // amount, fee
            staking_data.serialize_to_vec(),       // data
            self.mempool.current_height(),      // validity_start_height
            network_id,                         // network_id
        );

        let unlocked_wallets = self.unlocked_wallets.as_ref()
            .ok_or_else(|| object! {"message" => "No wallets"})?;
        let unlocked_wallets = unlocked_wallets.read();
        let wallet_account = unlocked_wallets.get(&staker_address)
            .ok_or_else(|| object! {"message" => "Sender account is locked"})?;
        wallet_account.sign_transaction(&mut tx);

        self.generic.push_transaction(tx)
    }

    /// Unstakes NIM
    /// Parameters:
    /// - staker_address: NIM address used to stake
    /// - amount: Amount to unstake
    pub(crate) fn unstake(&self, params: &[JsonValue]) -> Result<JsonValue, JsonValue> {
        let staker_address = Self::parse_address(params.get(0).unwrap_or(&Null), "staker")?;
        let amount = params.get(1)
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| object! {"message" => "Invalid amount"})
            .and_then(|it| Coin::try_from(it)
                .map_err(|e| object! {"message" => format!("Invalid amount: {}", e)}))?;

        let network_id = self.mempool.network_id();
        let staking_contract = NetworkInfo::from_network_id(network_id)
            .validator_registry_address().unwrap();

        let mut tx = Transaction::new_extended(
            staking_contract.clone(), AccountType::Staking, // sender
            staker_address, AccountType::Basic, // recipient
            amount, Coin::try_from(0).unwrap(), // amount, fee
            vec![],                             // data
            self.mempool.current_height(),      // validity_start_height
            network_id,                         // network_id
        );

        let unlocked_wallets = self.unlocked_wallets.as_ref()
            .ok_or_else(|| object! {"message" => "No wallets"})?;
        let unlocked_wallets = unlocked_wallets.read();
        let wallet_account = unlocked_wallets.get(&tx.recipient)
            .ok_or_else(|| object! {"message" => "Sender account is locked"})?;

        let proof = OutgoingStakingTransactionProof::Unstake(
            wallet_account.create_signature_proof(&tx)
        );
        tx.proof = proof.serialize_to_vec();

        self.generic.push_transaction(tx)
    }
}

impl Module for MempoolAlbatrossHandler {
    rpc_module_methods! {
        // Transactions
        "sendRawTransaction" => generic.send_raw_transaction,
        "createRawTransaction" => generic.create_raw_transaction,
        "sendTransaction" => generic.send_transaction,
        "mempoolContent" => generic.mempool_content,
        "mempool" => generic.mempool,
        "createValidator" => create_validator,
        "retireValidator" => retire_validator,
        "reactivateValidator" => reactivate_validator,
        "unparkValidator" => unpark_validator,
        "stake" => stake,
        "retire" => retire,
        "unstake" => unstake,
        "getTransaction" => generic.get_transaction,
    }
}
