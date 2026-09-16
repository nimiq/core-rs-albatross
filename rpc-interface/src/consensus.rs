use async_trait::async_trait;
use nimiq_consensus::consensus::consensus_proxy::ConsensusSyncStatus;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{AnyHash, PreImage};

use crate::types::{RPCResult, Transaction, ValidityStartHeight};

#[nimiq_jsonrpc_derive::proxy(name = "ConsensusProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ConsensusInterface {
    type Error;

    /// Returns a boolean specifying if we have established consensus with the network.
    // `nimiq_jsonrpc_derive::proxy` requires the receiver type to be a mutable reference.
    #[allow(clippy::wrong_self_convention)]
    async fn is_consensus_established(&self) -> RPCResult<bool, (), Self::Error>;

    /// Returns the status of the sync process
    async fn get_sync_status(&self) -> RPCResult<ConsensusSyncStatus, (), Self::Error>;

    /// Given a serialized transaction, it will return the corresponding transaction struct.
    async fn get_raw_transaction_info(
        &self,
        raw_tx: String,
    ) -> RPCResult<Transaction, (), Self::Error>;

    /// Sends the given serialized transaction to the network.
    async fn send_raw_transaction(&self, raw_tx: String)
        -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized basic transaction.
    async fn create_basic_transaction(
        &self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a basic transaction to the network.
    async fn send_basic_transaction(
        &self,
        wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized basic transaction with an arbitrary data field.
    async fn create_basic_transaction_with_data(
        &self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a basic transaction, with an arbitrary data field, to the network.
    async fn send_basic_transaction_with_data(
        &self,
        wallet: Address,
        recipient: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction creating a new vesting contract.
    async fn create_new_vesting_transaction(
        &self,
        wallet: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction creating a new vesting contract to the network.
    async fn send_new_vesting_transaction(
        &self,
        wallet: Address,
        owner: Address,
        start_time: u64,
        time_step: u64,
        num_steps: u32,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming a vesting contract.
    async fn create_redeem_vesting_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming a vesting contract to the network.
    async fn send_redeem_vesting_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction creating a new HTLC contract.
    async fn create_new_htlc_transaction(
        &self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction creating a new HTLC contract to the network.
    async fn send_new_htlc_transaction(
        &self,
        wallet: Address,
        htlc_sender: Address,
        htlc_recipient: Address,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming a HTLC contract
    /// using the `RegularTransfer` method.
    async fn create_redeem_regular_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: PreImage,
        hash_root: AnyHash,
        hash_count: u8,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming a HTLC contract, using the `RegularTransfer` method, to the
    /// network.
    async fn send_redeem_regular_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        pre_image: PreImage,
        hash_root: AnyHash,
        hash_count: u8,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming a HTLC contract using the `TimeoutResolve`
    /// method.
    async fn create_redeem_timeout_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming a HTLC contract, using the `TimeoutResolve` method, to the
    /// network.
    async fn send_redeem_timeout_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction redeeming a HTLC contract using the `EarlyResolve`
    /// method.
    async fn create_redeem_early_htlc_transaction(
        &self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction redeeming a HTLC contract, using the `EarlyResolve` method, to the
    /// network.
    async fn send_redeem_early_htlc_transaction(
        &self,
        contract_address: Address,
        recipient: Address,
        htlc_sender_signature: String,
        htlc_recipient_signature: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized signature that can be used to redeem funds from a HTLC contract using
    /// the `EarlyResolve` method.
    async fn sign_redeem_early_htlc_transaction(
        &self,
        wallet: Address,
        contract_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Returns a serialized `new_staker` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_new_staker_transaction(
        &self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `new_staker` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_new_staker_transaction(
        &self,
        sender_wallet: Address,
        staker_wallet: Address,
        delegation: Option<Address>,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `stake` transaction. The funds to be staked and the transaction fee will
    /// be paid from the `sender_wallet`.
    async fn create_stake_transaction(
        &self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `stake` transaction to the network. The funds to be staked and the transaction fee will
    /// be paid from the `sender_wallet`.
    async fn send_stake_transaction(
        &self,
        sender_wallet: Address,
        staker_address: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `update_staker` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_update_staker_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `update_staker` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn send_update_staker_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `set_active_stake` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_set_active_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `set_active_stake` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn send_set_active_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        new_active_balance: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `retire_stake` transaction. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn create_retire_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `retire_stake` transaction to the network. You can pay the transaction fee from a basic
    /// account (by providing the sender wallet) or from the staker account's balance (by not
    /// providing a sender wallet).
    async fn send_retire_stake_transaction(
        &self,
        sender_wallet: Option<Address>,
        staker_wallet: Address,
        retire_stake: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `remove_stake` transaction. The transaction fee will be paid from the funds
    /// being removed.
    async fn create_remove_stake_transaction(
        &self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `remove_stake` transaction to the network. The transaction fee will be paid from the funds
    /// being removed.
    async fn send_remove_stake_transaction(
        &self,
        staker_wallet: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `new_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the validator deposit.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// "" = Set the signal data field to None.
    /// "0x29a4b..." = Set the signal data field to Some(0x29a4b...).
    async fn create_new_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_secret_key: String,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `new_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee and the validator deposit.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// "" = Set the signal data field to None.
    /// "0x29a4b..." = Set the signal data field to Some(0x29a4b...).
    async fn send_new_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        signing_secret_key: String,
        voting_secret_key: String,
        reward_address: Address,
        signal_data: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `update_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// null = No change in the signal data field.
    /// "" = Change the signal data field to None.
    /// "0x29a4b..." = Change the signal data field to Some(0x29a4b...).
    async fn create_update_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_secret_key: Option<String>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `update_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    /// Since JSON doesn't have a primitive for Option (it just has the null primitive), we can't
    /// have a double Option. So we use the following work-around for the signal data:
    /// null = No change in the signal data field.
    /// "" = Change the signal data field to None.
    /// "0x29a4b..." = Change the signal data field to Some(0x29a4b...).
    async fn send_update_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        new_signing_secret_key: Option<String>,
        new_voting_secret_key: Option<String>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `deactivate_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_deactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `deactivate_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_deactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `reactivate_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_reactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `reactivate_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_reactivate_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `set_signal_data` transaction, signed with the validator's signing
    /// (warm) key. You need to provide the address of a basic account (the sender wallet) to pay
    /// the transaction fee. The `new_signal_data` field is interpreted as follows:
    /// null = Clear the signal data field (set it to None).
    /// "0x29a4b..." = Set the signal data field to Some(0x29a4b...).
    /// Note: this *replaces the entire signal data field*, overwriting any protocol version
    /// previously signaled via `create_signal_version_transaction`. Use that method instead if you
    /// only want to update the version while preserving the rest of the signal data.
    /// This transaction is only valid from protocol version `upgrades::v2::WARM_KEY_SIGNALING` onwards.
    async fn create_set_signal_data_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `set_signal_data` transaction to the network. See
    /// `create_set_signal_data_transaction` for the meaning of `new_signal_data`.
    async fn send_set_signal_data_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        new_signal_data: Option<String>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `signal_version` transaction that signals support for the given
    /// protocol `version`, signed with the validator's signing (warm) key. You need to provide the
    /// address of a basic account (the sender wallet) to pay the transaction fee.
    /// Note: this only updates the protocol-version bytes of the signal data and preserves the
    /// rest of the field; it cannot clear the field. To replace the entire signal data (including
    /// clearing it), use `create_set_signal_data_transaction`.
    /// This transaction is only valid from protocol version `upgrades::v2::WARM_KEY_SIGNALING` onwards.
    async fn create_signal_version_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        version: u16,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `signal_version` transaction that signals support for the given protocol `version`
    /// to the network.
    async fn send_signal_version_transaction(
        &self,
        sender_wallet: Address,
        validator_address: Address,
        signing_secret_key: String,
        version: u16,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `retire_validator` transaction. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn create_retire_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `retire_validator` transaction to the network. You need to provide the address of a basic
    /// account (the sender wallet) to pay the transaction fee.
    async fn send_retire_validator_transaction(
        &self,
        sender_wallet: Address,
        validator_wallet: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized `delete_validator` transaction. The transaction fee will be paid from the
    /// validator deposit that is being returned.
    /// Note in order for this transaction to be accepted fee + value should be equal to the validator deposit, which is not a fixed value:
    /// Failed delete validator transactions can diminish the validator deposit
    async fn create_delete_validator_transaction(
        &self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a `delete_validator` transaction to the network. The transaction fee will be paid from the
    /// validator deposit that is being returned.
    async fn send_delete_validator_transaction(
        &self,
        validator_wallet: Address,
        recipient: Address,
        fee: Coin,
        value: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction creating a new bridge contract.
    async fn create_new_bridge_transaction(
        &self,
        wallet: Address,
        owner: Address,
        oracle_address: Address,
        source_chain_id: u32,
        chain_config: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction creating a new bridge contract to the network.
    async fn send_new_bridge_transaction(
        &self,
        wallet: Address,
        owner: Address,
        oracle_address: Address,
        source_chain_id: u32,
        chain_config: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction creating a new oracle contract.
    async fn create_new_oracle_transaction(
        &self,
        wallet: Address,
        owner: Address,
        hash_count: u16,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction creating a new oracle contract to the network.
    async fn send_new_oracle_transaction(
        &self,
        wallet: Address,
        owner: Address,
        hash_count: u16,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction that deposits `value` into an existing bridge contract,
    /// locking it for transfer to the bridge's destination chain. The deposit and the fee are paid
    /// by `wallet`.
    ///
    /// `data` is a hex string that is stored in the transaction's recipient data. The bridge
    /// contract does not interpret it; the off-chain relayer reads the destination of the deposit
    /// from it.
    async fn create_bridge_deposit_transaction(
        &self,
        wallet: Address,
        bridge_address: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction that deposits `value` into an existing bridge contract to the network.
    /// See `create_bridge_deposit_transaction` for the parameters.
    async fn send_bridge_deposit_transaction(
        &self,
        wallet: Address,
        bridge_address: Address,
        data: String,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction that releases funds from a bridge contract against a proof
    /// that the corresponding tokens were burned on the source chain.
    ///
    /// Releases are permissionless: `signer_wallet` signs the burn proof and pays the fee, which is
    /// charged even if the release later fails.
    ///
    /// - `recipient` and `value` must match the target address and amount encoded in the burn
    ///   transaction.
    /// - `burn_transaction_data` is the raw burn transaction as a hex string.
    /// - `merkle_proof` is a hex-encoded, serialized `AnyMerkleProof` of the burn transaction.
    /// - `oracle_state_index` is the index of the oracle state the proof is verified against.
    async fn create_bridge_release_transaction(
        &self,
        signer_wallet: Address,
        bridge_address: Address,
        recipient: Address,
        burn_transaction_data: String,
        merkle_proof: String,
        oracle_state_index: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction that releases funds from a bridge contract to the network. See
    /// `create_bridge_release_transaction` for the parameters.
    ///
    /// If the node has the full state, the burn transaction is checked against the bridge first,
    /// so that a release that would fail is not broadcast and does not cost the signer a fee.
    async fn send_bridge_release_transaction(
        &self,
        signer_wallet: Address,
        bridge_address: Address,
        recipient: Address,
        burn_transaction_data: String,
        merkle_proof: String,
        oracle_state_index: u64,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction that appends `hashes` to an existing oracle contract.
    /// The update is signed by `owner_wallet`, while the fee is paid by `sender_wallet`.
    /// All hashes must use the oracle's hash algorithm.
    async fn create_update_oracle_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        hashes: Vec<AnyHash>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction that appends `hashes` to an existing oracle contract to the network.
    /// See `create_update_oracle_transaction` for the parameters.
    async fn send_update_oracle_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        hashes: Vec<AnyHash>,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction that transfers ownership of an existing oracle contract to
    /// `new_owner`. The change is signed by `owner_wallet`, the current owner, while the fee is paid
    /// by `sender_wallet`.
    async fn create_change_oracle_owner_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        new_owner: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction that transfers ownership of an existing oracle contract to the network.
    /// See `create_change_oracle_owner_transaction` for the parameters.
    async fn send_change_oracle_owner_transaction(
        &self,
        sender_wallet: Address,
        owner_wallet: Address,
        oracle_address: Address,
        new_owner: Address,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns a serialized transaction that withdraws the deposit of an oracle contract to
    /// `recipient`, which deletes the contract. The transaction is signed by `owner_wallet`.
    /// Note that in order for this transaction to be accepted, `value` must equal the full balance
    /// of the contract, so `fee` must currently be zero.
    async fn create_delete_oracle_transaction(
        &self,
        owner_wallet: Address,
        oracle_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<String, (), Self::Error>;

    /// Sends a transaction that withdraws the deposit of an oracle contract to the network. See
    /// `create_delete_oracle_transaction` for the parameters.
    async fn send_delete_oracle_transaction(
        &self,
        owner_wallet: Address,
        oracle_address: Address,
        recipient: Address,
        value: Coin,
        fee: Coin,
        validity_start_height: ValidityStartHeight,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;
}
