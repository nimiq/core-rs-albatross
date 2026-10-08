use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, coin::Coin, transaction::TransactionError};
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::{account::AccountType, key_nibbles::KeyNibbles};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::account::bridge_contract::{
    AnyMerkleProof, BridgeError, ChainConfig, OutgoingTransaction,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::account::bridge_contract::{
    CreationTransactionData, OutgoingBridgeTransactionData,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, HashType, Transaction};
#[cfg(feature = "interaction-traits")]
pub use store::BridgeContractStoreWrite;
pub use store::BridgeNonce;
use store::{BridgeContractStoreRead, BridgeContractStoreReadOps};
use thiserror::Error;

use crate::{
    account::oracle_contract::OracleContract, convert_receipt, data_store_ops::DataStoreReadOps,
    AccountReceipt,
};
#[cfg(feature = "interaction-traits")]
use crate::{
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{
        AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
    },
    reserved_balance::ReservedBalance,
    Account, BlockState, InherentLogger, Log, TransactionLog,
};
mod store;

/// The Bridge contract for cross-chain asset transfers.
///
/// This contract manages cross-chain transactions by validating Merkle proofs
/// against oracle-verified state hashes and processing asset transfers between
/// different blockchain networks.
///
/// The bridge contract uses a subtrie structure similar to the staking contract:
/// ```text
/// BRIDGE_CONTRACT_ADDRESS: BridgeContract
///     |--> PREFIX_NONCE || ADDRESS: BridgeNonce
/// ```
///
/// Nonces are stored in the accounts tree subtree for replay protection.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BridgeContract {
    /// The owner/operator of the bridge contract
    pub owner: Address,

    /// The oracle contract address for state verification
    pub oracle_address: Address,

    /// Contract balance (deposits and fees)
    pub balance: Coin,

    /// Source chain ID this bridge instance supports
    pub source_chain_id: u32,

    /// Chain-specific configuration (address format, endianness, hash function, etc.)
    /// This is used to validate incoming transactions according to the source chain's rules
    pub chain_config: ChainConfig,

    /// Total number of cross-chain transactions processed
    pub transaction_count: u64,
}

impl BridgeContract {
    /// Get the nonce for a given address, if it exists.
    pub fn get_nonce<T: DataStoreReadOps>(&self, data_store: &T, address: &Address) -> u64 {
        BridgeContractStoreRead::new(data_store)
            .get_nonce(address)
            .map(|n| n.nonce)
            .unwrap_or(0)
    }

    /// Checks that `burn_proof` proves its burn record against `oracle`, the bridge's oracle.
    ///
    /// Every oracle write replaces slot `i mod n` with `H(old ++ value)`, so the proof is one
    /// `MerklePath`: from the record's leaf through the record tree to the root the oracle wrote
    /// at `oracle_state_index`, then through the slot's buffer levels, first the value that root
    /// overwrote (a left sibling, the zero hash on the slot's first use) and then every value
    /// written to the slot since (right siblings). It must end in the slot's current value. An
    /// index older than the oracle's window still names its slot, so a proof never expires but
    /// grows by one node per rotation; `max_proof_depth` bounds all levels together.
    pub fn verify_burn_proof(
        &self,
        oracle: &OracleContract,
        burn_proof: &OutgoingTransaction,
    ) -> Result<(), BurnProofError> {
        match burn_proof.merkle_proof {
            AnyMerkleProof::Blake2bPath(_)
            | AnyMerkleProof::Sha256Path(_)
            | AnyMerkleProof::Keccak256Path(_) => {}
            AnyMerkleProof::Blake2b(_)
            | AnyMerkleProof::Sha256(_)
            | AnyMerkleProof::Keccak256(_) => return Err(BurnProofError::NotAMerklePath),
        }

        if !burn_proof.is_proof_depth_valid(self.chain_config.max_proof_depth) {
            return Err(BurnProofError::TooDeep {
                depth: burn_proof.proof_depth(),
                max_depth: self.chain_config.max_proof_depth,
            });
        }

        let index = burn_proof.oracle_state_index;
        let slot_value = oracle
            .get_hash_at_index(index)
            .ok_or(BurnProofError::UnwrittenIndex(index))?;

        let leaf_hash = burn_proof
            .extract_burn_transaction_hash(&self.chain_config.hash_function)
            .map_err(BurnProofError::InvalidRecord)?;

        // Folding fails only if the path's hash type differs from the leaf's.
        let root = burn_proof
            .compute_merkle_root(leaf_hash)
            .map_err(|_| BurnProofError::SlotMismatch(index))?;

        if &root != slot_value {
            return Err(BurnProofError::SlotMismatch(index));
        }

        Ok(())
    }
}

/// Why a burn proof does not prove its record against the bridge's oracle.
#[derive(Clone, Debug, Error)]
pub enum BurnProofError {
    #[error("the burn proof is not a Merkle path")]
    NotAMerklePath,
    #[error("the Merkle path has {depth} nodes, more than the maximum of {max_depth}")]
    TooDeep { depth: usize, max_depth: u32 },
    #[error("oracle index {0} has not been written")]
    UnwrittenIndex(u64),
    #[error("cannot hash the burn record: {0}")]
    InvalidRecord(BridgeError),
    #[error("the Merkle path does not end in the current value of the oracle slot of index {0}")]
    SlotMismatch(u64),
}

#[cfg(feature = "interaction-traits")]
impl BridgeContract {
    fn can_change_balance(
        &self,
        transaction: &Transaction,
        new_balance: Coin,
        is_reserve: bool,
    ) -> Result<(), AccountError> {
        // Outgoing bridge transactions are burn-releases, authorized by the Merkle burn-proof
        // and the burn target's signature verified in `commit_outgoing_transaction`, so we
        // deliberately do NOT require the owner's signature here. Requiring it made
        // `reserve_balance` (mempool admission) reject the very transactions that `commit`
        // accepts, diverging the mempool from consensus and breaking permissionless
        // submission. Balance availability is still enforced by `reserve_balance` itself.

        // If withdrawing, must withdraw the full balance (contract deletion)
        if new_balance < self.balance {
            // For reserve_balance, we allow reserving any amount up to the full balance
            if is_reserve {
                return Ok(());
            }
            // For actual withdrawal, only allow full withdrawal (balance goes to zero)
            if new_balance != Coin::ZERO {
                return Err(AccountError::InvalidForSender);
            }
            // Check that the transaction value equals the current balance
            if transaction.value != self.balance {
                return Err(AccountError::InvalidForSender);
            }
        }

        Ok(())
    }
}

#[cfg(feature = "interaction-traits")]
fn validate_oracle_hash_compatibility(
    oracle_address: &Address,
    chain_config: &ChainConfig,
    data_store: DataStoreWrite,
) -> Result<(), AccountError> {
    // Convert address to KeyNibbles for data store query
    let oracle_key = KeyNibbles::from(oracle_address);

    // Query the oracle contract directly from the global accounts tree
    // (not the bridge's subtrie).
    let oracle_account: Option<Account> = data_store.get_global(&oracle_key);

    // Extract oracle contract if it exists
    let oracle = match oracle_account {
        Some(Account::Oracle(oracle)) => oracle,
        Some(_) => {
            log::warn!(
                %oracle_address,
                "Bridge creation failed: address exists but is not an oracle contract"
            );
            return Err(AccountError::InvalidForRecipient);
        }
        None => {
            log::warn!(
                %oracle_address,
                "Bridge creation failed: oracle contract does not exist in accounts tree"
            );
            return Err(AccountError::InvalidForRecipient);
        }
    };

    // If oracle has no hashes yet, any hash function is acceptable
    if oracle.hashes.is_empty() {
        log::info!(
            %oracle_address,
            hash_function=HashType::from_any_hash(&chain_config.hash_function).name(),
            "Oracle is empty, bridge hash function will be accepted",
        );
        return Ok(());
    }

    // Get the oracle's hash type from its first hash
    let oracle_hash_type = HashType::from_any_hash(&oracle.hashes[0]);
    let bridge_hash_type = HashType::from_any_hash(&chain_config.hash_function);

    // Validate hash types match
    if oracle_hash_type != bridge_hash_type {
        log::warn!(
            oracle_hash_type = oracle_hash_type.name(),
            bridge_hash_type = bridge_hash_type.name(),
            %oracle_address,
            "Bridge creation failed: hash function mismatch"
        );
        return Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData,
        ));
    }

    log::info!(
        hash_type = oracle_hash_type.name(),
        "Bridge-oracle hash compatibility validated",
    );

    Ok(())
}

#[cfg(feature = "interaction-traits")]
impl AccountTransactionInteraction for BridgeContract {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        _block_state: &BlockState,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        let data = CreationTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;

        // Verify the creation data
        data.verify().map_err(AccountError::InvalidTransaction)?;

        // Validate oracle hash compatibility
        validate_oracle_hash_compatibility(&data.oracle_address, &data.chain_config, data_store)?;

        // The deposit is the transaction value
        let deposit = transaction.value;

        tx_logger.push_log(Log::BridgeCreate {
            contract_address: transaction.recipient.clone(),
            owner: data.owner.clone(),
            oracle_address: data.oracle_address.clone(),
            source_chain_id: data.source_chain_id,
            deposit,
        });

        Ok(Account::Bridge(BridgeContract {
            balance: initial_balance + deposit,
            owner: data.owner,
            oracle_address: data.oracle_address,
            source_chain_id: data.source_chain_id,
            chain_config: data.chain_config,
            transaction_count: 0,
        }))
    }

    fn revert_new_contract(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance -= transaction.value;

        tx_logger.push_log(Log::BridgeCreate {
            contract_address: transaction.recipient.clone(),
            owner: self.owner.clone(),
            oracle_address: self.oracle_address.clone(),
            source_chain_id: self.source_chain_id,
            deposit: transaction.value,
        });

        Ok(())
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        // Regular incoming transactions to bridge (user locking funds)
        // The transaction should have IncomingTransaction in recipient_data
        // specifying the target chain and address

        // Increment bridge balance (user is locking funds)
        self.balance += transaction.value;
        self.transaction_count += 1;

        tx_logger.push_log(Log::BridgeIncoming {
            contract_address: transaction.recipient.clone(),
            sender: transaction.sender.clone(),
            value: transaction.value,
        });

        Ok(None)
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Revert the balance increase (user was locking funds)
        self.balance -= transaction.value;
        self.transaction_count -= 1;

        tx_logger.push_log(Log::BridgeIncoming {
            contract_address: transaction.recipient.clone(),
            sender: transaction.sender.clone(),
            value: transaction.value,
        });

        Ok(())
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        // Parse the outgoing bridge transaction data from sender_data
        let outgoing_data = OutgoingBridgeTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;

        // Verify the transaction signature
        outgoing_data
            .verify(transaction)
            .map_err(AccountError::InvalidTransaction)?;

        // Parse burn data using the validation program
        let parsed_burn = outgoing_data
            .burn_proof
            .parse_burn_data(&self.chain_config)
            .map_err(|error| {
                log::warn!(?error, "Failed to parse burn data");
                AccountError::InvalidTransaction(TransactionError::InvalidData)
            })?;

        // Only the target can release its burn, since the release consumes the target's nonce.
        // Anyone can still submit a release that the target signed.
        if !outgoing_data
            .proof
            .is_signed_by(&parsed_burn.target_address)
        {
            log::warn!(
                signer = %outgoing_data.proof.compute_signer(),
                target = %parsed_burn.target_address,
                "Release is not signed by the burn target",
            );
            return Err(AccountError::InvalidSignature);
        }

        // The fee is taken from the burned amount: the target receives `value` and the block
        // reward receives `fee`, so together they must add up to exactly the burned amount.
        if transaction.value.checked_add(transaction.fee) != Some(parsed_burn.amount) {
            log::warn!(
                tx_value = %transaction.value,
                tx_fee = %transaction.fee,
                parsed_value = %parsed_burn.amount,
                "Transaction value and fee do not add up to the burned amount"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidValue,
            ));
        }

        // Verify the transaction recipient matches the burn proof target address
        if transaction.recipient != parsed_burn.target_address {
            log::warn!(
                tx_recipient = %transaction.recipient,
                parsed_recipient = %parsed_burn.target_address,
                "Transaction recipient mismatch",
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // Verify the target chain ID matches this bridge's source chain ID
        // (burn happened on source chain, releasing on Nimiq)
        if parsed_burn.target_chain_id != self.source_chain_id {
            log::warn!(
                expected = self.source_chain_id,
                obtained = parsed_burn.target_chain_id,
                "Chain ID mismatch"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // Check nonce for replay protection
        let mut store = BridgeContractStoreWrite::new(&mut data_store);
        let highest_nonce = store
            .get_nonce(&parsed_burn.target_address)
            .map(|n| n.nonce)
            .unwrap_or(0);

        if parsed_burn.target_nonce != highest_nonce + 1 {
            log::warn!(
                parsed_nonce = parsed_burn.target_nonce,
                expected_nonce = highest_nonce + 1,
                "Invalid nonce: expected sequential nonce"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // Query the oracle contract from the global accounts tree.
        let oracle_key = KeyNibbles::from(&self.oracle_address);
        let oracle_account: Option<Account> = store.data_store().get_global(&oracle_key);

        let oracle = match oracle_account {
            Some(Account::Oracle(oracle)) => oracle,
            _ => {
                log::warn!(
                    oracle_address = %self.oracle_address,
                    "Oracle contract not found or invalid",
                );
                return Err(AccountError::InvalidForSender);
            }
        };

        // The proof must end in the current value of the oracle slot that the relayer's root was
        // written to.
        self.verify_burn_proof(&oracle, &outgoing_data.burn_proof)
            .map_err(|error| {
                log::warn!(%error, "Invalid burn proof");
                match error {
                    BurnProofError::UnwrittenIndex(_) | BurnProofError::InvalidRecord(_) => {
                        AccountError::InvalidTransaction(TransactionError::InvalidData)
                    }
                    BurnProofError::NotAMerklePath
                    | BurnProofError::TooDeep { .. }
                    | BurnProofError::SlotMismatch(_) => {
                        AccountError::InvalidTransaction(TransactionError::InvalidProof)
                    }
                }
            })?;

        // Decrement bridge balance by the whole burned amount, `value + fee`
        self.balance = self.balance.checked_sub(parsed_burn.amount).ok_or(
            AccountError::InsufficientFunds {
                needed: parsed_burn.amount,
                balance: self.balance,
            },
        )?;

        // Update processed nonces in the accounts tree
        store.put_nonce(
            &parsed_burn.target_address,
            BridgeNonce {
                nonce: parsed_burn.target_nonce,
            },
        );

        // Increment transaction count
        self.transaction_count += 1;

        // Log the outgoing transaction
        tx_logger.push_log(Log::pay_fee_log(transaction));
        tx_logger.push_log(Log::BridgeOutgoing {
            contract_address: transaction.sender.clone(),
            recipient: parsed_burn.target_address.clone(),
            value: transaction.value,
        });

        // Create receipt for revert
        let receipt = ProcessOutgoingReceipt {
            target_address: parsed_burn.target_address,
            nonce: parsed_burn.target_nonce,
            amount: parsed_burn.amount,
        };

        Ok(Some(receipt.into()))
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Extract receipt
        let receipt = receipt.ok_or(AccountError::InvalidReceipt)?;
        let receipt = ProcessOutgoingReceipt::try_from(&receipt)?;

        // Revert balance change (add back the released funds)
        self.balance += receipt.amount;

        // Revert nonce update in the accounts tree
        let mut store = BridgeContractStoreWrite::new(&mut data_store);

        // Restore the previous nonce. If the reverted nonce was 1 (the first ever for
        // this address), remove the entry entirely so state matches the pre-tx condition.
        if receipt.nonce > 1 {
            store.put_nonce(
                &receipt.target_address,
                BridgeNonce {
                    nonce: receipt.nonce - 1,
                },
            );
        } else {
            store.remove_nonce(&receipt.target_address);
        }

        // Decrement transaction count
        self.transaction_count -= 1;

        // Log the revert
        tx_logger.push_log(Log::BridgeOutgoing {
            contract_address: transaction.sender.clone(),
            recipient: receipt.target_address.clone(),
            value: transaction.value,
        });
        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        // The fee is paid out of a successful release, and the bridge never pays for a failed one.
        // A failed release with a fee therefore cannot pay for its failure, so it is invalid: the
        // block containing it is invalid, rather than the fee being credited without anybody
        // paying it.
        if !transaction.fee.is_zero() {
            return Err(AccountError::InvalidForSender);
        }
        Ok(None)
    }

    fn revert_failed_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Do nothing: only a failed release without a fee is ever committed.
        Ok(())
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _block_state: &BlockState,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        // Both the released `value` and the `fee` come out of the bridge balance.
        let needed = reserved_balance
            .balance()
            .checked_add(transaction.total_value())
            .ok_or(AccountError::InvalidCoinValue)?;
        let new_balance = self.balance.safe_sub(needed)?;
        self.can_change_balance(transaction, new_balance, true)?;

        reserved_balance.reserve(self.balance, transaction.total_value())
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        reserved_balance.release(transaction.total_value());
        Ok(())
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountInherentInteraction for BridgeContract {
    fn commit_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Err(AccountError::InvalidForTarget)
    }

    fn revert_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForTarget)
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountPruningInteraction for BridgeContract {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Option<AccountReceipt> {
        Some(PrunedBridgeContract::from(self).into())
    }

    fn restore(
        _ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        let receipt = pruned_account.ok_or(AccountError::InvalidReceipt)?;
        let pruned_account = PrunedBridgeContract::try_from(receipt)?;
        Ok(Account::Bridge(BridgeContract::from(pruned_account)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct PrunedBridgeContract {
    pub owner: Address,
    pub oracle_address: Address,
    pub source_chain_id: u32,
    pub chain_config: ChainConfig,
    pub transaction_count: u64,
}

impl From<BridgeContract> for PrunedBridgeContract {
    fn from(contract: BridgeContract) -> Self {
        PrunedBridgeContract {
            owner: contract.owner,
            oracle_address: contract.oracle_address,
            source_chain_id: contract.source_chain_id,
            chain_config: contract.chain_config,
            transaction_count: contract.transaction_count,
        }
    }
}

impl From<PrunedBridgeContract> for BridgeContract {
    fn from(receipt: PrunedBridgeContract) -> Self {
        BridgeContract {
            balance: Coin::ZERO,
            owner: receipt.owner,
            oracle_address: receipt.oracle_address,
            source_chain_id: receipt.source_chain_id,
            chain_config: receipt.chain_config,
            transaction_count: receipt.transaction_count,
        }
    }
}

convert_receipt!(PrunedBridgeContract);

/// Receipt for process outgoing transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct ProcessOutgoingReceipt {
    /// The address that received the transaction
    pub target_address: Address,
    /// The nonce that was processed
    pub nonce: u64,
    /// The amount that was released
    pub amount: Coin,
}

convert_receipt!(ProcessOutgoingReceipt);
